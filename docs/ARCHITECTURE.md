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

The native Rust UEFI application lives in `loader/` and targets
`x86_64-unknown-uefi`. It opens
`\\EFI\\BOOT\\sovereign-kernel.elf` from the boot device and accepts
bounded ELF64 x86_64 `ET_EXEC` images. Its loader currently performs:

1. UEFI system-table validation, console output, Loaded Image and Simple File
   System protocol discovery.
2. ELF header and program-header validation, including checked file ranges,
   segment alignment, canonical virtual ranges, executable entry coverage,
   page-level overlap rejection, and W^X rejection.
3. One contiguous physical allocation for the kernel virtual image span
   (maximum 256 MiB), zero initialization of the complete span, segment copies,
   and BSS/inter-segment zeroing.
4. PE32+ inspection of the loaded EFI application. Headers are mapped RO/NX,
   executable sections RX, and writable data sections RW/NX.
5. EFER.NXE enablement and active CR4.LA57 detection. CR0.WP is enabled only
   in the final transition, after ExitBootServices. The page-table
   builder creates the matching four- or five-level hierarchy with 4 KiB leaves.
6. Identity mappings for the EFI image, firmware GDT, emergency IDT, a dedicated
   256 KiB kernel stack, boot-info page, 1 MiB memory-map buffer, and all
   EFI_ACPI_RECLAIM_MEMORY / EFI_ACPI_MEMORY_NVS ranges. ACPI reclaimable
   pages are RO/NX and NVS pages RW/NX. Kernel segments are mapped at their
   linked higher-half virtual addresses with ELF-derived permissions.
7. Discovery and checksum validation of the UEFI ACPI 2.0/1.0 RSDP, followed
   by read-only identity mapping of its complete structure when present.
8. Population and validation of SovereignBootInfo v1, including the physical
   memory-map address, actual descriptor byte size and stride, descriptor
   version, optional ACPI RSDP, kernel physical/virtual range, and entry point.
9. A final GetMemoryMap refresh and bounded ExitBootServices retry sequence.
   The preallocated boot-info page is refreshed between each map capture and
   exit attempt without allocating or calling another firmware service.
10. Installation of a minimal emergency IDT, dedicated stack switch, CR0.WP
   activation, CR3 switch, global-TLB invalidation when PGE is active, and
   transfer to the kernel entry point. No firmware service is called after
   ExitBootServices succeeds.

The page-table implementation rejects duplicate leaf mappings, invalid
physical/virtual addresses, and writable+executable leaves. A private NX
proof token ensures EFER.NXE was enabled before installing NX entries.
CR0.WP is enabled immediately before the CR3 switch so supervisor writes
respect read-only mappings without changing firmware behavior during Boot Services.
When CR4.PGE is set, the transition disables it before changing CR3 and keeps
it disabled until the new root is active. This flushes global translations and
prevents firmware mappings from refilling stale global entries during the
transition; PGE is restored afterward. A CR3 reload alone does not discard them. The
four-/five-level choice follows the firmware's active CR4.LA57 setting.

The handoff contract is defined in `loader/src/boot_info.rs`. Its kernel-side
mirror and validator live in `kernel/src/boot/sovereign.rs`; the consumer
checks ABI size/version, flags, physical and virtual ranges, and descriptor
stride. The memory-map reader uses firmware-provided stride rather than
assuming descriptors are tightly packed.

A separate kernel binary target, `sovereign-kernel-uefi`, uses
`kernel/src/sovereign_main.rs` and the linker entry `_start`. It receives
the boot-info pointer in RDI, validates the handoff and every memory descriptor,
initializes the bootstrap GDT/TSS/IDT/PIC, and then halts with interrupts
disabled. The existing `sovereign-kernel` binary and Limine entry remain
unchanged as the compatibility path.

### Remaining validation and limitations

- The loader and UEFI page provider access freshly allocated physical pages
  through the firmware's active identity mapping before CR3 changes. This
  assumption must be verified on supported firmware implementations.
- PE32+ section parsing, the GDT mapping, emergency IDT, and the full transition
  have not yet been compiled or exercised on real firmware.
- The kernel entry validates the memory map but does not yet initialize the
  physical memory manager, reclaim loader memory, discover ACPI, initialize
  framebuffer output, or enable device interrupts.
- The boot image packaging pipeline must place the UEFI application at
  `EFI/BOOT/BOOTX64.EFI` and the dedicated UEFI kernel binary at
  `EFI/BOOT/sovereign-kernel.elf`.

The standalone UEFI loader has passed its release build in GitHub Actions.
The full kernel build remains blocked by compile errors in parallel driver and
subsystem work; no complete kernel image, QEMU boot, or physical-machine boot
has been recorded. Limine requests and the existing Limine image path must
remain until the Sovereign UEFI path passes those validations.

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
