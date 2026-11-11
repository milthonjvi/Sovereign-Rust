# USB HID ↔ Input subsystem integration contract

This document defines the hand-off between `drivers/usb` and
`drivers/input`. It deliberately does not add an input implementation to USB
or modify files owned by the input workstream.

## Ownership boundary

### USB owns transport and device protocol

USB is responsible for:

- Discovering the HID interface and its interrupt-IN endpoint during USB
  configuration parsing.
- Setting the HID protocol/idle state when required.
- Retrieving the HID Report descriptor for report-protocol devices.
- Scheduling and executing interrupt-IN transfers through
  `HostController::interrupt_transfer`.
- Detecting endpoint stalls, transfer failures, device removal and re-enumeration.
- Supplying the raw report bytes, report length, interface metadata and device
  lifecycle to the input adapter.

USB must not own keymaps, character generation, desktop/window policy, input
event queues, or PS/2 compatibility policy.

### Input owns semantic decoding and event publication

Input is responsible for:

- Assigning and retiring `InputDeviceId` values.
- Translating HID usage-page/usage data into `InputEvent` values.
- Tracking prior report state to generate press/release edges.
- Normalizing pointer, touch, tablet, gamepad and consumer-control reports.
- Publishing events to its own SPSC producer and consuming them through
  `InputEventMux`.
- Handling queue overflow and clearing per-device state on disconnect.

Do not add a second USB-specific keyboard/mouse event queue or duplicate the
existing PS/2 event pipeline.

## Existing interfaces to reuse

USB-side APIs already present:

- `drivers::usb::hid::find_boot_interface` for boot keyboard/mouse compatibility.
- `drivers::usb::hid_transport::find_interface` for generic report-protocol HID interfaces.
- `drivers::usb::hid_transport::HidInterfaceInfo::interrupt_in_service_info` to obtain endpoint metadata plus speed-aware service intervals and payload limits.
- `drivers::usb::hid_transport::HidInterruptEndpoint::read_report` to poll a selected endpoint without duplicating USB transfer validation.
- `drivers::usb::hid::HidDescriptor::find_in_configuration` to locate a class descriptor by interface and alternate setting in the raw configuration descriptor.
- `drivers::usb::hid_transport::read_report_descriptor` to fetch the complete Report descriptor through the validated control-transfer path.
- `drivers::usb::hid::BootHidInterface::read_report` for bounded raw report transport.
- `drivers::usb::hid::get_report_descriptor` to construct the class-independent Report Descriptor request.
- `drivers::usb::hid::BootKeyboardReport::parse`
- `drivers::usb::hid::BootMouseReport::parse`
- `drivers::usb::hid::BootMouseReportWheel::parse`
- `drivers::usb::controller::HostController::interrupt_transfer`
- `drivers::usb::transfer::{DeviceAddress, Endpoint, TransferResult}`

Input-side APIs already present:

- `drivers::input::event::{InputDeviceId, InputEvent, ContactPhase, GamepadAxis}`
- `drivers::input::queue::{SpscQueue, SpscProducer, SpscConsumer}`
- `drivers::input::mux::{InputEventMux, InputSourceId}`

The USB host-controller interface accepts an `Endpoint` and a mutable report
buffer. The input adapter should use the discovered interface endpoint and
only publish a report after a successful transfer. The USB layer does not
currently own the input queue or an input-device ID allocator.

## Required adapter behavior

### Queue topology

Create one SPSC queue for the USB input producer and give its unique consumer
to `InputEventMux`. Keep the USB producer on one serialized polling/task
context. Do not share a single SPSC producer between a USB polling context and
the PS/2 IRQ context. The mux already supports independent consumers and
round-robin fairness.

### Boot keyboard

Use `BootKeyboardReport::parse` to validate the 8-byte boot report. Keep the
previous report per physical HID interface.

For every newly pressed or released key usage, publish:

```rust
InputEvent::HidKey {
    device,
    usage_page: 0x07,
    usage: usage as u16,
    pressed,
    modifiers: current_report.modifiers,
}
```

The modifier bitmap represents usages E0–E7. Compare the old and new modifier
bits and emit individual `HidKey` transitions for usages `0xE0 + bit`; do not
silently treat modifier changes as ordinary key-array entries. Preserve the
current modifier bitmap in each emitted event.

Do not translate HID usages into characters or layout-specific `KeyCode`
values in the USB layer. Keymap policy belongs above the transport.

### Boot mouse

Use `BootMouseReport::parse` (3-byte report) or
`BootMouseReportWheel::parse` (4-byte report when the selected device/report
format provides a wheel byte). Publish normalized relative motion with
`InputEvent::PointerMotion`:

- `dx` and `dy` from the signed report values.
- `wheel_y` from the signed wheel value; `wheel_x = 0` unless an extended
  report supplies horizontal scrolling.
- `buttons` as the HID button bitmap, preserving buttons beyond the first
  three when the report format supports them.

Do not assume every mouse has a wheel or exactly three buttons.

### Report-protocol HID devices

Boot protocol is only the compatibility path for boot keyboards and mice.
For other HID devices, Input must consume the Report descriptor and implement
bounded bit-field decoding using report IDs, usage pages, logical ranges,
signedness, and collection boundaries. The USB class request to retrieve a
Report descriptor is:

- bmRequestType: `0x81` (device-to-host, standard, interface)
- bRequest: `6` (GET_DESCRIPTOR)
- wValue: `0x2200` (HID Report descriptor type, index zero)
- wIndex: interface number
- wLength: descriptor length from the HID descriptor

Do not infer report layout from endpoint packet size. Packet size bounds the
transfer, not the semantic report structure. Validate descriptor lengths and
report IDs before reading fields. Keep parsing allocation-free and reject
truncated or out-of-range fields.

### Lifecycle and errors

- Emit `InputEvent::DeviceConnected` only after the interface is configured
  and ready to deliver reports.
- On unplug, emit `InputEvent::DeviceDisconnected`, release the input ID, and
  discard prior keyboard/button/contact state.
- On transfer timeout, stall or controller error, do not parse the buffer as
  a fresh report. Recover/reset the endpoint according to USB policy.
- Count dropped events if the SPSC queue is full; never block the USB transfer
  path waiting for input consumers.
- Keep each interface/collection's previous report state separate. A composite
  USB device may expose multiple HID interfaces and must not share their state.

## Suggested implementation ownership for the Input workstream

Implement the USB HID event adapter under `drivers/input`, not under the USB
transport module. It should accept the discovered USB interface metadata,
device address and a successful report byte slice, then publish normalized
`InputEvent` values. Keep the adapter independent of PS/2 decoders and reuse
the existing input event vocabulary and mux.

USB will continue improving discovery, control requests, endpoint scheduling,
transfer reliability, DMA and disconnect handling. Any missing transport
metadata should be requested from this module rather than duplicated in Input.


## Generic HID interface discovery added

The USB module now exposes `hid_transport::find_interface(configuration, interface_number, alternate_setting)`. It deliberately accepts non-boot HID interfaces (subclass/protocol zero included) and returns only transport metadata. `HidInterfaceInfo::interrupt_in_count` and `interrupt_in_endpoint` enumerate every interrupt-IN endpoint belonging to that interface. Input can therefore support report-protocol keyboards, mice, touchscreens, tablets, gamepads and vendor-defined collections without USB implementing their semantics.

For the HID class descriptor, retain the raw configuration descriptor during enumeration and use `hid::HidDescriptor::find_in_configuration(raw_configuration, interface_number, alternate_setting)`. `ParsedConfiguration` currently retains the standard interface/endpoint tree, not arbitrary class-specific descriptor bytes. Pass the resulting descriptor to `hid_transport::read_report_descriptor`; it issues the GET_DESCRIPTOR request and validates the complete transfer. Usage interpretation, logical ranges, collection semantics and normalized event publication remain Input responsibilities.


## Periodic scheduling metadata

Use `interrupt_in_service_info(configuration, ordinal, device.speed)` instead of assuming that the raw `bInterval` value means milliseconds. The returned `HidInterruptEndpoint` distinguishes Full/Low Speed frame intervals from High/SuperSpeed microframe intervals, reports High Speed transactions per opportunity, and retains SuperSpeed burst and bytes-per-interval data. The Input adapter should use this metadata to schedule polling; it should not reinterpret USB descriptor fields independently.

For Report Protocol devices, retrieve the HID class descriptor from the interface's class-specific descriptor records in the raw configuration descriptor, parse it with `HidDescriptor::parse`, then call `read_report_descriptor`. That helper enforces the declared descriptor length and uses the common control-transfer completion checks. The raw Report descriptor's usages and collections remain Input-owned semantics.


The raw configuration blob should be retained by the USB enumeration/device layer for class drivers that need class-specific descriptors. Do not try to reconstruct HID class descriptors from `ParsedConfiguration`, because that parsed representation intentionally stores only standard interface and endpoint metadata.
