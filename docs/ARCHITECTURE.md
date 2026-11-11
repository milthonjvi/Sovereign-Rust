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
