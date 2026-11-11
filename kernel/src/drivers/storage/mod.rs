//! Block-device subsystem and partition metadata parsers.
//!
//! Hardware transports (AHCI, NVMe, VirtIO Block) remain separate and require
//! PCI, DMA, MMIO, and interrupt services before hardware access is enabled.

pub mod block;
pub mod gpt;
pub mod mbr;

pub use block::{BlockDevice, BlockError, BlockGeometry, BlockRange, validate_buffer};
pub use gpt::{enumerate_gpt_partitions, GptError, GptHeader, GptPartition};
pub use mbr::{enumerate_logical_partitions, LogicalPartition, MbrError, MbrPartition, MbrTable};
