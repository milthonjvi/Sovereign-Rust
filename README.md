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

The loader validates segments but does not yet allocate pages, copy segment data,
zero BSS, acquire the final memory map, call ExitBootServices, or transfer control
to the kernel. The kernel still consumes its existing Limine handoff during this
transition; Limine has not been removed.

## Build

Install the relevant freestanding Rust targets:

```sh
rustup target add x86_64-unknown-none x86_64-unknown-uefi
cargo build --release -p sovereign-kernel --bin sovereign-kernel
cargo build --release -p sovereign-boot-uefi --target x86_64-unknown-uefi
```

The kernel build uses `kernel/linker.ld`. The UEFI application is built as a
separate firmware-targeted executable. Packaging it into a bootable EFI System
Partition image is a subsequent image-pipeline step.

## Architecture and collaboration

See [Architecture](docs/ARCHITECTURE.md) and
[Parallel work coordination](docs/PARALLEL_WORK.md) before contributing.
