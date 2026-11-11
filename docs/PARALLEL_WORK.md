# Parallel work coordination

This repository is integrated through one coordination point. Parallel chats
must not edit the same files or expand their assignment into another workstream.

## Ownership map

| Workstream | Assigned paths | Must not modify |
|---|---|---|
| Core integration (coordinator) | root manifests, `README.md`, `docs/`, shared interfaces | paths explicitly owned by an active workstream |
| Boot and image pipeline | `boot/**`, `tools/boot/**` | `kernel/src/**`, root manifests, shared docs |
| x86_64 CPU support | `kernel/src/arch/x86_64/**` | boot pipeline, memory, drivers, scheduler |
| Memory management | `kernel/src/memory/**` | architecture implementation, boot pipeline, drivers |
| Interrupts and exceptions | `kernel/src/interrupts/**` | boot pipeline, memory allocator, drivers |
| Device drivers | `kernel/src/drivers/**` | architecture internals, memory allocator, scheduler |
| Scheduler and tasks | `kernel/src/task/**` | boot pipeline, architecture internals, memory allocator |

## Coordination rules

1. One workstream owns a path at a time. No overlapping edits.
2. Do not rename, move, or reformat another workstream's files.
3. Do not modify root manifests or shared interfaces without coordinator approval.
4. Before starting, inspect the latest `main` and check whether assigned paths
   have changed since the task was issued.
5. Commit directly to `main`; do not create branches or pull requests unless
   the repository owner explicitly changes this rule.
6. Keep changes scoped to the assignment. Report exact files, commit, checks,
   and anything not tested.
7. The coordinator resolves interface conflicts and performs integration.
