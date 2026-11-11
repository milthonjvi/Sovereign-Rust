# Parallel work coordination

This repository has one integration owner: this project's coordinating chat.
Other chats may implement isolated workstreams, but must not edit overlapping
paths or redefine shared interfaces independently.

## Ownership map

| Workstream | Assigned paths | Must not modify |
|---|---|---|
| Core integration, Sovereign Boot UEFI, and x86_64 bootstrap (coordinator) | root manifests, `README.md`, `docs/`, `loader/**`, shared interfaces, `kernel/src/arch/x86_64/**`, `kernel/src/boot/**` | active workstream-owned files |
| Boot and image pipeline | `boot/**`, `tools/boot/**` | `loader/**`, `kernel/src/**`, root manifests, shared docs |
| Advanced x86_64 CPU support | coordinator-owned `kernel/src/arch/x86_64/**` (coordinate before edits) | boot pipeline, memory, drivers, scheduler |
| Memory management | `kernel/src/memory/**` | architecture implementation, boot pipeline, drivers |
| Interrupts and exceptions | `kernel/src/interrupts/**` | boot pipeline, memory allocator, drivers |
| Device bus and firmware | `kernel/src/drivers/pci/**`, `kernel/src/drivers/acpi/**`, `kernel/src/firmware/**` | driver API, other driver families, architecture internals |
| Display and input | `kernel/src/drivers/framebuffer/**`, `kernel/src/drivers/input/**` | driver API, PCI, USB, architecture internals |
| Storage and USB | `kernel/src/drivers/storage/**`, `kernel/src/drivers/usb/**` | driver API, PCI, memory allocator, interrupt internals |
| Network and serial | `kernel/src/drivers/network/**`, `kernel/src/drivers/serial/**` | driver API, PCI, architecture internals |
| Timer devices | `kernel/src/drivers/timer/**` | driver API, timekeeping policy, interrupt internals |
| I/O APIC controller and GSI topology (coordinator) | `kernel/src/drivers/ioapic.rs`, `kernel/src/drivers/ioapic_topology.rs` | ACPI, PCI, interrupt dispatch, shared driver API |
| Scheduler and tasks | `kernel/src/task/**` | boot pipeline, architecture internals, memory allocator |
| Other kernel services | `kernel/src/sync/**`, `time/**`, `fs/**`, `ipc/**`, `syscall/**`, `power/**`, `security/**`, `logging/**` | shared APIs and other assigned workstreams |

## Shared-interface rules

1. `kernel/src/drivers/api.rs` is coordinator-owned. Do not change it without
   explicit approval; propose required additions with a concrete use case.
2. `kernel/src/lib.rs` and all Cargo manifests are coordinator-owned.
3. Drivers consume kernel services through narrow interfaces; they must not
   implement their own memory allocator, interrupt controller, scheduler, or VFS.
4. The Sovereign UEFI path publishes the validated handoff through
   `kernel::boot::sovereign::boot_info()` and the strided UEFI memory map
   through `kernel::boot::sovereign::memory_map()`. It also publishes ACPI
   table addresses through `acpi_tables()` and parsed MADT routing metadata
   through `madt_topology()`. It identity-maps UEFI ACPI reclaimable (type 9)
   and NVS (type 10) ranges; consumers must validate each physical range and
   must not assume a higher-half direct map exists. The memory manager must
   copy required descriptors before reclaiming the loader-owned map buffer.
5. Hardware-specific unsafe code stays inside the relevant architecture or
   driver module and must document its safety invariants.
6. No driver may touch MMIO, port I/O, DMA, or interrupt registers until the
   relevant platform primitives and resource ownership are available.

## Coordination rules

1. One workstream owns a path at a time. No overlapping edits.
2. Do not rename, move, or reformat another workstream's files.
3. Before starting, inspect the latest `main` and check whether assigned paths
   have changed since the task was issued.
4. Commit directly to `main`; do not create branches or pull requests unless
   the repository owner explicitly changes this rule.
5. Keep changes scoped to the assignment. Report exact files, commit, checks,
   hardware/emulator tested, and anything not tested.
6. The coordinator resolves interface conflicts and performs integration.
7. A module declaration or compile-only check is not evidence of working
   hardware support. State implementation and validation status precisely.
