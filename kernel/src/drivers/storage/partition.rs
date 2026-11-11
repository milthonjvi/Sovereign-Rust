//! Unified, allocation-free partition views and bounded block devices.

use super::block::{BlockDevice, BlockError, BlockGeometry, BlockRange};

/// On-disk origin of a partition descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PartitionScheme {
    Gpt,
    MbrPrimary,
    MbrLogical,
}

/// Scheme-specific metadata retained without heap allocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PartitionType {
    Gpt {
        type_guid: [u8; 16],
        unique_guid: [u8; 16],
        attributes: u64,
        name: [u16; 36],
    },
    Mbr {
        partition_type: u8,
        bootable: bool,
    },
}

/// A validated half-open block range: [first_lba, first_lba + block_count).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Partition {
    pub scheme: PartitionScheme,
    pub first_lba: u64,
    pub block_count: u64,
    pub kind: PartitionType,
}

impl Partition {
    pub fn validate(self, geometry: BlockGeometry) -> Result<Self, PartitionError> {
        geometry.validate().map_err(PartitionError::Block)?;
        if self.block_count == 0 {
            return Err(PartitionError::Empty);
        }
        geometry
            .validate_range(BlockRange::new(self.first_lba, self.block_count))
            .map_err(PartitionError::Block)?;
        match self.kind {
            PartitionType::Gpt { type_guid, unique_guid, .. } => {
                if type_guid.iter().all(|v| *v == 0) || unique_guid.iter().all(|v| *v == 0) {
                    return Err(PartitionError::InvalidMetadata);
                }
                if self.scheme != PartitionScheme::Gpt {
                    return Err(PartitionError::SchemeMismatch);
                }
            }
            PartitionType::Mbr { partition_type, .. } => {
                if partition_type == 0 || self.scheme == PartitionScheme::Gpt {
                    return Err(PartitionError::SchemeMismatch);
                }
            }
        }
        Ok(self)
    }

    pub const fn end_lba_exclusive(self) -> Option<u64> {
        self.first_lba.checked_add(self.block_count)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PartitionError {
    Block(BlockError),
    Empty,
    InvalidMetadata,
    SchemeMismatch,
    OutputFull,
    InvalidMbrContainer,
}

/// Convert GPT entries into a caller-owned unified descriptor array.
pub fn from_gpt(
    entries: &[Option<super::gpt::GptPartition>],
    geometry: BlockGeometry,
    output: &mut [Option<Partition>],
) -> Result<usize, PartitionError> {
    output.fill(None);
    let result = (|| {
        let mut count = 0;
        for entry in entries.iter().flatten() {
            if count == output.len() { return Err(PartitionError::OutputFull); }
            let blocks = entry.last_lba.checked_sub(entry.first_lba)
                .and_then(|n| n.checked_add(1)).ok_or(PartitionError::InvalidMetadata)?;
            let partition = Partition {
                scheme: PartitionScheme::Gpt,
                first_lba: entry.first_lba,
                block_count: blocks,
                kind: PartitionType::Gpt {
                    type_guid: entry.type_guid,
                    unique_guid: entry.unique_guid,
                    attributes: entry.attributes,
                    name: entry.name,
                },
            }.validate(geometry)?;
            output[count] = Some(partition);
            count += 1;
        }
        Ok(count)
    })();
    if result.is_err() { output.fill(None); }
    result
}

/// Convert non-container MBR primary entries. Extended containers are not
/// exposed as mountable volumes; their EBR logical entries are converted separately.
pub fn from_mbr_primary(
    table: &super::mbr::MbrTable,
    geometry: BlockGeometry,
    output: &mut [Option<Partition>],
) -> Result<usize, PartitionError> {
    output.fill(None);
    let result = (|| {
        let mut count = 0;
        for entry in table.iter() {
            if entry.is_extended() || entry.is_protective_gpt() { continue; }
            if count == output.len() { return Err(PartitionError::OutputFull); }
            let partition = Partition {
                scheme: PartitionScheme::MbrPrimary,
                first_lba: entry.start_lba as u64,
                block_count: entry.sector_count as u64,
                kind: PartitionType::Mbr { partition_type: entry.partition_type, bootable: entry.bootable },
            }.validate(geometry)?;
            output[count] = Some(partition);
            count += 1;
        }
        Ok(count)
    })();
    if result.is_err() { output.fill(None); }
    result
}

/// Convert EBR logical partitions (whose LBAs are already absolute).
pub fn from_mbr_logical(
    entries: &[Option<super::mbr::LogicalPartition>],
    geometry: BlockGeometry,
    output: &mut [Option<Partition>],
) -> Result<usize, PartitionError> {
    output.fill(None);
    let result = (|| {
        let mut count = 0;
        for entry in entries.iter().flatten() {
            if count == output.len() { return Err(PartitionError::OutputFull); }
            let partition = Partition {
                scheme: PartitionScheme::MbrLogical,
                first_lba: entry.start_lba,
                block_count: entry.sector_count as u64,
                kind: PartitionType::Mbr { partition_type: entry.partition_type, bootable: entry.bootable },
            }.validate(geometry)?;
            output[count] = Some(partition);
            count += 1;
        }
        Ok(count)
    })();
    if result.is_err() { output.fill(None); }
    result
}

/// A block-device view constrained to one partition. All caller LBAs are
/// relative to the partition and are translated with checked arithmetic.
pub struct PartitionDevice<'a, D: BlockDevice> {
    parent: &'a mut D,
    partition: Partition,
    geometry: BlockGeometry,
}

impl<'a, D: BlockDevice> PartitionDevice<'a, D> {
    pub fn new(parent: &'a mut D, partition: Partition) -> Result<Self, PartitionError> {
        let parent_geometry = parent.geometry().validate().map_err(PartitionError::Block)?;
        let partition = partition.validate(parent_geometry)?;
        Ok(Self {
            parent,
            partition,
            geometry: BlockGeometry { block_count: partition.block_count, block_size: parent_geometry.block_size },
        })
    }

    pub const fn partition(&self) -> Partition { self.partition }

    fn translate(&self, range: BlockRange) -> Result<BlockRange, BlockError> {
        self.geometry.validate_range(range)?;
        let start = self.partition.first_lba.checked_add(range.start_block)
            .ok_or(BlockError::AddressOverflow)?;
        let translated = BlockRange::new(start, range.block_count);
        self.parent.geometry().validate_range(translated)?;
        Ok(translated)
    }
}

impl<D: BlockDevice> BlockDevice for PartitionDevice<'_, D> {
    fn geometry(&self) -> BlockGeometry { self.geometry }

    fn read_blocks(&mut self, range: BlockRange, buffer: &mut [u8]) -> Result<(), BlockError> {
        let translated = self.translate(range)?;
        super::block::validate_buffer(self.geometry, range, buffer.len())?;
        self.parent.read_blocks(translated, buffer)
    }

    fn write_blocks(&mut self, range: BlockRange, buffer: &[u8]) -> Result<(), BlockError> {
        let translated = self.translate(range)?;
        super::block::validate_buffer(self.geometry, range, buffer.len())?;
        self.parent.write_blocks(translated, buffer)
    }

    fn flush(&mut self) -> Result<(), BlockError> { self.parent.flush() }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MemDisk { bytes: [u8; 4096], reads: usize, writes: usize, flushes: usize }
    impl BlockDevice for MemDisk {
        fn geometry(&self) -> BlockGeometry { BlockGeometry { block_count: 8, block_size: 512 } }
        fn read_blocks(&mut self, r: BlockRange, b: &mut [u8]) -> Result<(), BlockError> {
            super::super::block::validate_buffer(self.geometry(), r, b.len())?;
            self.reads += 1;
            let s = r.start_block as usize * 512;
            b.copy_from_slice(&self.bytes[s..s + b.len()]);
            Ok(())
        }
        fn write_blocks(&mut self, r: BlockRange, b: &[u8]) -> Result<(), BlockError> {
            super::super::block::validate_buffer(self.geometry(), r, b.len())?;
            self.writes += 1;
            let s = r.start_block as usize * 512;
            self.bytes[s..s + b.len()].copy_from_slice(b);
            Ok(())
        }
        fn flush(&mut self) -> Result<(), BlockError> { self.flushes += 1; Ok(()) }
    }

    fn descriptor() -> Partition {
        Partition { scheme: PartitionScheme::MbrPrimary, first_lba: 2, block_count: 3,
            kind: PartitionType::Mbr { partition_type: 0x83, bootable: false } }
    }

    #[test]
    fn partition_device_translates_relative_lbas() {
        let mut bytes = [0; 4096];
        bytes[3 * 512] = 0x7B;
        let mut disk = MemDisk { bytes, reads: 0, writes: 0, flushes: 0 };
        let mut view = PartitionDevice::new(&mut disk, descriptor()).unwrap();
        let mut block = [0u8; 512];
        view.read_blocks(BlockRange::new(1, 1), &mut block).unwrap();
        assert_eq!(block[0], 0x7B, "relative LBA 1 must map to parent LBA 3");
        assert_eq!(view.parent.reads, 1);
    }

    #[test]
    fn partition_device_rejects_escape_and_overflow() {
        let mut disk = MemDisk { bytes: [0; 4096], reads: 0, writes: 0, flushes: 0 };
        let mut view = PartitionDevice::new(&mut disk, descriptor()).unwrap();
        let mut block = [0u8; 512];
        assert_eq!(view.read_blocks(BlockRange::new(2, 2), &mut block), Err(BlockError::OutOfBounds));
        assert_eq!(view.read_blocks(BlockRange::new(u64::MAX, 1), &mut block), Err(BlockError::AddressOverflow));
        assert_eq!(view.parent.reads, 0);
    }

    #[test]
    fn partition_device_bounds_writes_and_forwards_flush() {
        let mut disk = MemDisk { bytes: [0; 4096], reads: 0, writes: 0, flushes: 0 };
        let mut view = PartitionDevice::new(&mut disk, descriptor()).unwrap();
        let block = [0xA5u8; 512];
        view.write_blocks(BlockRange::new(2, 1), &block).unwrap();
        view.flush().unwrap();
        assert_eq!(view.parent.writes, 1);
        assert_eq!(view.parent.flushes, 1);
    }

    #[test]
    fn converts_gpt_and_mbr_descriptors() {
        let geo = BlockGeometry { block_count: 128, block_size: 512 };
        let gpt = [Some(super::super::gpt::GptPartition {
            type_guid: [1; 16], unique_guid: [2; 16], first_lba: 10, last_lba: 19,
            attributes: 5, name: [0; 36],
        })];
        let mut out = [None; 2];
        assert_eq!(from_gpt(&gpt, geo, &mut out), Ok(1));
        assert_eq!(out[0].unwrap().block_count, 10);
        assert_eq!(out[0].unwrap().scheme, PartitionScheme::Gpt);

        let table = super::super::mbr::MbrTable {
            disk_signature: 0,
            partitions: [
                Some(super::super::mbr::MbrPartition {
                    bootable: true, partition_type: 0x83, start_lba: 20, sector_count: 8,
                }),
                None, None, None,
            ],
        };
        assert_eq!(from_mbr_primary(&table, geo, &mut out), Ok(1));
        assert_eq!(out[0].unwrap().first_lba, 20);
        assert_eq!(out[0].unwrap().scheme, PartitionScheme::MbrPrimary);

        let logical = [Some(super::super::mbr::LogicalPartition {
            bootable: false, partition_type: 0x07, start_lba: 40, sector_count: 12,
        })];
        assert_eq!(from_mbr_logical(&logical, geo, &mut out), Ok(1));
        assert_eq!(out[0].unwrap().scheme, PartitionScheme::MbrLogical);
    }

    #[test]
    fn conversion_failure_clears_partial_output() {
        let geo = BlockGeometry { block_count: 128, block_size: 512 };
        let entries = [
            Some(super::super::gpt::GptPartition {
                type_guid: [1; 16], unique_guid: [2; 16], first_lba: 10, last_lba: 19,
                attributes: 0, name: [0; 36],
            }),
            Some(super::super::gpt::GptPartition {
                type_guid: [1; 16], unique_guid: [3; 16], first_lba: 200, last_lba: 210,
                attributes: 0, name: [0; 36],
            }),
        ];
        let mut out = [None; 2];
        assert!(from_gpt(&entries, geo, &mut out).is_err());
        assert!(out.iter().all(Option::is_none));
    }

    #[test]
    fn rejects_invalid_descriptor() {
        let mut disk = MemDisk { bytes: [0; 4096], reads: 0, writes: 0, flushes: 0 };
        let invalid = Partition { first_lba: u64::MAX, ..descriptor() };
        assert!(PartitionDevice::new(&mut disk, invalid).is_err());
    }
}
