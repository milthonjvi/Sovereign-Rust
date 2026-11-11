//! Physical and virtual memory management.
//!
//! Planned layers: boot memory-map validation, physical frame allocator, virtual
//! address-space/page-table management, kernel heap, and DMA-safe allocation.
//! Allocators must not be enabled before their backing memory is validated.

pub mod boot_map;
