//! Physical and virtual memory management.
//!
//! The bootstrap layer validates the firmware memory map and provides a
//! monotonic physical-frame allocator. Runtime frame reclamation, virtual
//! address spaces, heap allocation, and DMA-safe memory remain separate layers.

pub mod boot_map;
pub mod frame;
