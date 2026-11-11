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

The loader defines the initial versioned handoff ABI in
`loader/src/boot_info.rs`. Version 1 carries physical-address fields for the
firmware memory map, loaded kernel image, entry point, and optional ACPI and
framebuffer metadata. The UEFI entry point now populates the structure and
validates its fields before retaining it; the kernel-side validator now exists in `kernel/src/boot/sovereign.rs` and
is exported by `kernel/src/boot/mod.rs`. It validates the ABI, checked address
ranges, optional metadata flags, and UEFI descriptor stride, and reads memory
map entries without assuming descriptors are tightly packed. The consumer is
not yet wired into a separate kernel entry point or actual register handoff. The kernel must validate all ranges before
dereferencing physical addresses.

A guarded CPU helper in `loader/src/cpu.rs` checks extended CPUID support,
enables EFER.NXE only when the processor advertises NX, enables CR0.WP so
supervisor writes respect read-only PTEs, and detects active CR4.LA57 mode.
The UEFI entry point invokes it before constructing the new page tables.

The loader also contains a four-/five-level x86_64 page-table builder in
`loader/src/page_tables.rs`, plus a UEFI `AllocatePages` provider in
`loader/src/uefi/page_table_memory.rs`. It detects the active CR4.LA57 mode and builds the matching four- or
five-level hierarchy. It maps 4 KiB pages, rejects duplicate leaf mappings
and writable+executable mappings, and requires a verified EFER.NXE token
before creating tables. The UEFI entry point now allocates a bounded contiguous physical span for the
validated ELF virtual image, copies every non-empty `PT_LOAD` file payload,
and zeroes the full span first so BSS and inter-segment gaps are initialized.
ELF segments that share a 4 KiB page are rejected, as are writable+executable
segments. The entry point then builds a new four-level page-table root and maps
each kernel segment with its ELF-derived write/execute permissions and NX
protection. The image allocation and page-table allocations use UEFI
`AllocatePages` and remain owned by the loader.

The loader now also reserves a 1 MiB UEFI memory-map buffer and a boot-info
page before page-table construction. It parses the loaded PE32+ image and identity-maps headers RO/NX, executable
sections RX, and writable data sections RW/NX. It also maps a bounded window
around the current RSP as RW/NX, the memory-map buffer as read-only, and the
boot-info page as read-only in the new address space. After
all page-table allocations, it captures a memory-map snapshot and populates
SovereignBootInfo v1 with the map's physical address, byte size, descriptor
stride/version, and kernel image metadata. The snapshot is not yet final for
ExitBootServices: later firmware calls can invalidate its map key.

This is still not a complete handoff. The page-table root is not loaded into
CR3, and the loader does not call ExitBootServices or transfer control. The
kernel does not yet consume the Sovereign Boot ABI. The UEFI allocation
provider and image loader currently access allocated pages through the
firmware's active identity mapping. The current-stack mapping is a conservative
window, not a firmware-provided authoritative stack bound. The loader PE sections receive section-derived permissions. This parser must
be validated against actual firmware-built EFI images before relying on those
mappings for a CR3 transition.

A bounded ExitBootServices helper now refreshes the memory map and retries
stale map keys without allocating or making unrelated firmware calls between
capture and each exit attempt. It is not invoked by the current entry point.

The next stages are:

1. Validate exact EFI image section permissions and obtain authoritative stack bounds.
2. Integrate the final memory-map refresh and ExitBootServices retry helper,
   with no intervening allocations or firmware calls.
3. Add the post-exit CR3 switch and x86_64 transition stub.
4. Add the kernel-side SovereignBootInfo consumer and entry ABI.
5. Validate the full path in QEMU and on physical UEFI hardware before retiring
   the Limine compatibility path.

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
