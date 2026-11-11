# Sovereign Core architecture

## Initial decisions

- **Target:** x86_64.
- **Primary language:** Rust.
- **Assembly:** restricted to bootstrapping and CPU transitions that cannot be
  expressed safely or reliably through Rust/compiler intrinsics.
- **Structure:** a small `no_std` kernel crate with architecture-specific code
  isolated from portable kernel policy.
- **Dependencies:** keep the initial kernel dependency-free. Review value,
  maintenance status, license, and boot/runtime impact before adding any.
- **Boot path:** not locked yet. Select a boot protocol before implementing the
  first executable kernel image; do not build a custom bootloader by default.

## Repository layout

- `kernel/`: kernel crate and internal subsystems.
- `boot/`: boot protocol integration, configuration, and image assembly.
- `tools/`: host-side build and image utilities.
- `docs/`: architecture decisions and contributor coordination.

## Design constraints

1. Keep hardware-specific code behind explicit architecture boundaries.
2. Keep drivers decoupled from scheduler, memory-management, and filesystem internals.
3. Keep unsafe Rust small, documented, and localized.
4. Prefer independently testable components and explicit interfaces.
5. Do not add placeholder subsystems without an implementation or consumer.
6. A successful host-side build is not proof of bootability. Report boot validation
   separately and ultimately exercise it in QEMU and real hardware.
