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

pub mod fat_read;
pub use fat::{FatChainValue, FatDirEntry};
pub use fat_read::{enumerate_directory, read_cluster, read_fat_entry, read_fat_entry_from_copy, verify_fat_mirrors, read_file_data, read_file_range, read_fs_info, FatFsInfo, FatVolume};

pub mod fat_lfn;
pub use fat_lfn::{consume_lfn_record, short_name_checksum, visit_directory, lookup_path, FatLongName, LfnError};

pub mod cache;
pub use cache::{BlockCache, CacheStats};

pub mod nvme;
pub use nvme::{build_prps, identify_command, read_command, write_command, NvmeCapabilities, NvmeCommand, NvmeCompletion, NvmeCompletionQueue, NvmeError, NvmeNamespace, NvmePrpMapping, NvmeSubmissionQueue};

pub mod ahci;
pub use ahci::{build_prdt, command_header, AhciCapabilities, AhciCommandHeader, AhciError, AhciIdentify, AhciPrdtEntry, RegisterFisH2d};
