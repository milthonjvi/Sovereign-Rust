//! Physical and virtual memory management.
//!
//! Bootstrap memory-map validation and monotonic physical-frame allocation
//! are separate from x86_64 page-table inspection. The latter uses the
//! recursive window established by Sovereign Boot; it does not assume an HHDM.
//! Runtime frame reclamation, page-table mutation, heap allocation, and
//! DMA-safe memory remain separate layers until their ownership contracts exist.

pub mod boot_map;
pub mod frame;
pub mod paging;
