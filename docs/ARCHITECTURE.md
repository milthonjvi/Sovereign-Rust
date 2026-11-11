# Sovereign Core architecture

## Initial decisions

- **Target:** x86_64.
- **Primary language:** Rust (`no_std` kernel).
- **Assembly:** restricted to bootstrapping and CPU transitions that cannot be
  expressed safely or reliably through Rust/compiler intrinsics.
- **Dependencies:** keep the initial kernel dependency-free. Review value,
  maintenance status, license, and boot/runtime impact before adding any.
- **Boot path:** not locked yet. Select a boot protocol before implementing the
  first executable kernel image.

## Kernel layers

- `arch/`: CPU-specific mechanisms, privileged instructions, context switching,
  page-table primitives, and interrupt entry stubs.
- `boot/`: validated boot information passed into the kernel.
- `memory/`: physical frames, virtual address spaces, heap, and DMA allocation.
- `interrupts/`: exceptions, IDT, interrupt routing, and handler dispatch.
- `drivers/`: bus enumeration and hardware-specific device drivers.
- `task/`: threads, scheduler, and context lifecycle.
- `sync/`: synchronization primitives and wait queues.
- `time/`: monotonic clock, clocksource, and deadlines.
- `fs/`: VFS and filesystem implementations, independent of block drivers.
- `ipc/`: communication primitives between tasks.
- `syscall/`: controlled user/kernel ABI, after user mode exists.
- `firmware/`: normalized ACPI and bootloader platform information.
- `power/`: reboot, shutdown, and sleep-state coordination.
- `security/`: privilege separation and capability enforcement.
- `logging/`: early diagnostics and runtime log routing.

## x86_64 bootstrap

The kernel requests the Limine base revision 6 handoff and consumes its HHDM,
memory-map, and optional framebuffer responses. The linker keeps protocol
requests in a writable ELF load segment because the bootloader writes response
pointers into those structures. The kernel entry disables interrupts, detects
CPU capabilities with CPUID, and establishes the baseline CR0/CR4 state needed
for SSE2. It deliberately leaves AVX/XSAVE, APIC routing, IDT installation,
page-table replacement, and interrupt enablement to later stages.

The kernel is linked as an ELF64 higher-half image for `x86_64-unknown-none`.
The host-side boot/image pipeline is separate from the kernel-side protocol
handoff and is responsible for packaging the Limine bootloader files.

## Driver strategy

The driver framework starts with an allocation-free common contract in
`drivers/api.rs`. Drivers are split by hardware family:

- **PCI/PCIe:** bus enumeration and configuration-space access.
- **ACPI:** platform tables and interrupt/topology descriptions.
- **Framebuffer:** boot framebuffer first; accelerated graphics later.
- **Input:** PS/2 initially, then USB HID.
- **Serial:** UART for early diagnostics.
- **Timer:** PIT/HPET/APIC timer backends.
- **Storage:** virtio-blk or AHCI/NVMe after PCI, DMA, and interrupts.
- **Network:** NIC drivers, kept separate from network protocols.
- **USB:** xHCI host controller before USB device-class drivers.

A directory or module contract is not an implemented driver. Hardware support
must be reported per device and validated in QEMU and on physical hardware.
Do not probe hardware until MMIO/port access, memory mapping, interrupt routing,
and resource ownership are ready.

## Design constraints

1. Keep hardware-specific code behind explicit architecture boundaries.
2. Keep drivers decoupled from scheduler, memory-management, and filesystem internals.
3. Keep unsafe Rust small, documented, and localized.
4. Validate firmware tables, memory maps, DMA ranges, and device resources.
5. Interrupt handlers must be bounded; defer lengthy work to task context.
6. Do not add placeholder subsystems without an implementation or consumer.
7. A successful host-side build is not proof of bootability.
