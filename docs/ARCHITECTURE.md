# Sovereign Core architecture

## Initial decisions

- **Target:** x86_64.
- **Primary language:** Rust (`no_std` kernel and loader).
- **Assembly:** restricted to privileged CPU operations and transitions that cannot be
  expressed safely or reliably through Rust/compiler intrinsics.
- **Dependencies:** keep the initial kernel and loader dependency-free. Review value,
  maintenance status, license, and boot/runtime impact before adding any.
- **Boot path:** Sovereign Boot, a native Rust UEFI application. UEFI is the first
  firmware target; legacy BIOS support follows after the UEFI path is functional.

## Sovereign Boot

The loader lives in `loader/` as an independent workspace package and targets
`x86_64-unknown-uefi`. Its first milestone defines the UEFI application entry
point, the firmware system-table ABI fields needed at entry, and an
allocation-free Simple Text Output Protocol wrapper.

The implementation is intentionally staged. The following capabilities remain
to be implemented before Sovereign Boot can load the kernel:

1. UEFI Boot Services wrappers and Loaded Image / Simple File System protocols.
2. Kernel file discovery and bounded ELF64 validation.
3. Physical page allocation and ELF PT_LOAD segment placement.
4. A normalized boot-information structure with memory-map ownership.
5. ExitBootServices with a refreshed memory-map key.
6. CPU/page-table preparation and transfer to the kernel entry point.
7. A matching kernel-side Sovereign Boot ABI consumer.

Until those stages are integrated and tested, the kernel's existing Limine
handoff remains in place as a temporary compatibility path. Do not remove
Limine requests or image support before the Sovereign Boot path has passed
emulator and physical-machine validation.

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
- `firmware/`: normalized ACPI and platform information.
- `power/`: reboot, shutdown, and sleep-state coordination.
- `security/`: privilege separation and capability enforcement.
- `logging/`: early diagnostics and runtime log routing.

## Current kernel bootstrap

The kernel is currently linked as an ELF64 higher-half image for
`x86_64-unknown-none`. Its current entry path still requests the Limine base
revision 6 handoff and consumes HHDM, memory-map, and optional framebuffer
responses. The linker keeps those protocol requests in a writable ELF load
segment because Limine writes response pointers into them.

The entry disables interrupts, detects CPU capabilities with CPUID, and
establishes the baseline CR0/CR4 state needed for SSE2, enables supervisor
write-protect (CR0.WP), and enables NXE when the CPU advertises NX. It
deliberately leaves AVX/XSAVE, APIC routing, IDT installation, page-table
replacement, and interrupt enablement to later stages.

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
