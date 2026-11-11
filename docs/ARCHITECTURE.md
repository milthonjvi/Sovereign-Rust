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
6. Reading the ELF64 header and obtaining the file size through `EFI_FILE_INFO`.
7. Requiring `ET_EXEC` (until relocations exist), bounding the program-header table,
   and validating each `PT_LOAD` file range, memory size, alignment, canonical virtual
   address range, and executable entry-point coverage.

The loader also defines the initial versioned handoff ABI in
`loader/src/boot_info.rs`. Version 1 carries physical-address fields for the
firmware memory map, loaded kernel image, entry point, and optional ACPI and
framebuffer metadata. The contract is currently a definition only: the loader
does not yet populate or pass this structure, and the kernel does not yet
consume it. The kernel must validate all ranges before dereferencing addresses.

A guarded CPU helper in `loader/src/cpu.rs` checks extended CPUID support
and enables EFER.NXE only when the processor advertises NX. The UEFI entry
point invokes it before installing page-table entries that use the NX bit.

The loader also contains a four-level x86_64 page-table builder in
`loader/src/page_tables.rs`, plus a UEFI `AllocatePages` provider in
`loader/src/uefi/page_table_memory.rs`. It maps 4 KiB pages, rejects duplicate
leaf mappings and writable+executable mappings, and requires the caller to
confirm EFER.NXE is enabled before creating tables. The UEFI entry point now allocates a bounded contiguous physical span for the
validated ELF virtual image, copies every non-empty `PT_LOAD` file payload,
and zeroes the full span first so BSS and inter-segment gaps are initialized.
ELF segments that share a 4 KiB page are rejected, as are writable+executable
segments. The entry point then builds a new four-level page-table root and maps
each kernel segment with its ELF-derived write/execute permissions and NX
protection. The image allocation and page-table allocations use UEFI
`AllocatePages` and remain owned by the loader.

This is still not a complete handoff. The page-table root is not loaded into
CR3. The loader has not yet mapped its own executable image, current stack,
firmware interfaces needed before `ExitBootServices`, handoff structure, or
final memory-map buffer into the new address space. The kernel does not yet
consume the Sovereign Boot ABI. The UEFI allocation provider currently accesses
allocated pages through the firmware's active identity mapping; this assumption
must be replaced or explicitly guaranteed by the final mapping transition.

The next stages are:

1. Establish and map the loader runtime, stack, boot-info buffer, and required
   physical/direct-map regions in the new address space.
2. Populate the versioned handoff structure and retain every referenced buffer.
3. Acquire a stable final memory map and call `ExitBootServices` with retry logic.
4. Switch CR3 only after the post-exit transition path is fully mapped.
5. Transfer control using the agreed x86_64 entry ABI and add the kernel-side consumer.

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
