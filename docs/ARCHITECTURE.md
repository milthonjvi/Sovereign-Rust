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
`x86_64-unknown-uefi`. It currently provides:

1. A UEFI application entry point and validated system-table signature/size.
2. An allocation-free Simple Text Output Protocol wrapper.
3. The Boot Services HandleProtocol entry needed to locate firmware protocols.
4. Loaded Image and Simple File System protocol definitions.
5. Opening the boot device's FAT volume and looking for
   `\\EFI\\BOOT\\sovereign-kernel.elf`.
6. Reading and validating the 64-byte ELF64 header: magic, class, endianness,
   version, executable/shared-object type, x86_64 machine, header size, and
   program-header table shape.

This is still a discovery/validation stage, not a complete kernel loader. It
does not yet validate the file's total length against the program-header table,
load PT_LOAD segments, allocate physical pages, construct final page tables,
or call ExitBootServices.

The next stages are:

1. Complete bounded ELF parsing and validate every program-header range.
2. Allocate and load PT_LOAD segments at validated physical/virtual addresses.
3. Define the versioned Sovereign Boot information structure.
4. Obtain a stable final memory map and call ExitBootServices with retry logic.
5. Prepare CPU/page-table state and transfer control to the kernel entry point.
6. Add a matching kernel-side Sovereign Boot ABI consumer.

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
