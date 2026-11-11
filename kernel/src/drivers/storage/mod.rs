//! Block-device subsystem.
//!
//! Transport-independent block I/O contracts live here. AHCI, NVMe, and
//! VirtIO Block transports remain separate and require PCI, DMA, MMIO, and
//! interrupt services before hardware access is enabled.

pub mod block;

pub use block::{BlockDevice, BlockError, BlockGeometry, BlockRange, validate_buffer};
