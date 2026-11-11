# Sovereign Core

Sovereign Core is an operating-system kernel project written primarily in Rust,
targeting **x86_64**.

The design reuses proven operating-system concepts while keeping the kernel
implementation under this project's control. Assembly is intentionally limited
to privileged CPU operations and transitions that require it.

## Repository structure

- `kernel/` — `no_std` kernel library, freestanding entry point, CPU support,
  boot handoff, and kernel subsystems.
- `boot/` — Limine bootloader configuration and bootable image construction.
- `tools/` — host-side build and image utilities.
- `docs/` — architecture decisions and parallel-work coordination.

## Current implementation

- x86_64 CPUID discovery and feature reporting.
- Isolated privileged primitives for control registers, MSRs, and port I/O.
- Early BSP baseline setup for CR0/CR4 and SSE2 OS support.
- Limine protocol request structures for the memory map, HHDM, and framebuffer.
- Higher-half ELF linker script and a freestanding kernel entry point.

These are foundational components, not a claim of complete hardware support.
The kernel currently halts after validating its mandatory boot responses;
memory management, IDT/interrupt setup, diagnostics, and device drivers remain
subsequent implementation stages.

## Build

Install the freestanding Rust target:

```sh
rustup target add x86_64-unknown-none
cargo build --release -p sovereign-kernel --bin sovereign-kernel
```

The workspace config selects `x86_64-unknown-none` and links with
`kernel/linker.ld`. Bootable image assembly requires the Limine files and
configuration in `boot/`; it is a separate step from compiling the ELF.

## Architecture and collaboration

See [Architecture](docs/ARCHITECTURE.md) and
[Parallel work coordination](docs/PARALLEL_WORK.md) before contributing.
