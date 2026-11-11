# Sovereign Core

Sovereign Core is an operating-system kernel project written primarily in Rust,
targeting **x86_64**.

The design reuses proven operating-system concepts while keeping the kernel
implementation under this project's control. Assembly is intentionally limited
to privileged CPU operations and transitions that require it.

## Repository structure

- `kernel/` — `no_std` kernel, CPU support, boot handoff, and kernel subsystems.
- `loader/` — native Rust `no_std` UEFI boot application (Sovereign Boot).
- `boot/` — bootable image construction and firmware boot configuration.
- `tools/` — host-side build and image utilities.
- `docs/` — architecture decisions and parallel-work coordination.

## Current implementation

- x86_64 CPUID discovery and feature reporting.
- Isolated privileged primitives for control registers, MSRs, and port I/O.
- Early BSP baseline setup for CR0/CR4 and SSE2 OS support.
- Higher-half ELF linker script and a freestanding kernel entry point.
- Sovereign Boot UEFI entry point and allocation-free firmware console.
- UEFI Loaded Image and Simple File System protocol access.
- Kernel lookup at `\\EFI\\BOOT\\sovereign-kernel.elf` and bounded ELF64 validation
  for x86_64 `ET_EXEC` images, including file length, program-header bounds,
  `PT_LOAD` file/memory ranges, alignment, canonical virtual ranges, and executable entry point.

Sovereign Boot now loads the ELF image, zeroes BSS, builds four-/five-level
page tables with NX/W^X permissions, maps its PE32+ runtime sections, GDT,
emergency IDT, dedicated 256 KiB kernel stack, boot-info page, and UEFI memory
map. It populates SovereignBootInfo v1, refreshes the memory map, calls
ExitBootServices with stale-key retries, switches CR3, and transfers control
to a separate `sovereign-kernel-uefi` entry point. The kernel validates the
handoff and memory descriptors before installing its bootstrap descriptor
tables. The Limine-compatible `sovereign-kernel` target remains available.

**Build and firmware validation have not yet been run.** The UEFI image
packaging pipeline and QEMU/physical-machine validation remain outstanding.

## Build

Install the relevant freestanding Rust targets:

```sh
rustup target add x86_64-unknown-none x86_64-unknown-uefi
cargo build --release -p sovereign-kernel --bin sovereign-kernel
cargo build --release -p sovereign-kernel --bin sovereign-kernel-uefi
cargo build --release -p sovereign-boot-uefi --target x86_64-unknown-uefi
```

Both kernel binaries use `kernel/linker.ld`; the dedicated UEFI target consumes
SovereignBootInfo in RDI while the default target remains Limine-compatible.
The UEFI application is built as a separate firmware-targeted executable.
Packaging the two UEFI binaries into an EFI System Partition image is the next
image-pipeline step.

## Architecture and collaboration

See [Architecture](docs/ARCHITECTURE.md) and
[Parallel work coordination](docs/PARALLEL_WORK.md) before contributing.
