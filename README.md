# Sovereign Core

Sovereign Core is an operating-system kernel project written primarily in Rust,
targeting **x86_64**.

The design reuses proven operating-system concepts while keeping the kernel
implementation under this project's control. Assembly is intentionally limited
to bootstrapping and CPU transitions where Rust is not an appropriate tool.

## Repository structure

- `kernel/` — `no_std` kernel crate and architecture-independent subsystems.
- `boot/` — boot protocol integration and bootable image construction.
- `tools/` — host-side build and image utilities.
- `docs/` — architecture decisions and parallel-work coordination.

## Current status

The initial workspace and kernel crate are being established. The boot protocol
and executable image pipeline will be selected before implementing the first
bootable image.

See [Architecture](docs/ARCHITECTURE.md) and
[Parallel work coordination](docs/PARALLEL_WORK.md) before contributing.
