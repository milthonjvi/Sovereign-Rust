//! Block-device subsystem and partition metadata parsers.
//!
//! Hardware transports (AHCI, NVMe, VirtIO Block) remain separate and require
//! PCI, DMA, MMIO, and interrupt services before hardware access is enabled.

pub mod block;
pub mod fat;
pub mod gpt;
pub mod mbr;

pub use block::{BlockDevice, BlockError, BlockGeometry, BlockRange, validate_buffer};
pub use gpt::{enumerate_gpt_partitions, enumerate_gpt_partitions_redundant, read_gpt_header_pair, GptError, GptHeader, GptHeaderPair, GptPartition};
pub use mbr::{enumerate_logical_partitions, LogicalPartition, MbrError, MbrPartition, MbrTable};
pub mod partition;

pub use partition::{
    from_gpt, from_mbr_logical, from_mbr_primary, Partition, PartitionDevice,
    PartitionError, PartitionScheme, PartitionType,
};

pub mod volume;
pub use volume::{VolumeError, VolumeTable};

pub use fat::{parse_bpb, probe_fat, FatBpb, FatError, FatKind};
