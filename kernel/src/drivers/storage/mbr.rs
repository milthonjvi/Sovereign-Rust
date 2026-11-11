//! Allocation-free Master Boot Record parsing and primary partition enumeration.
use super::block::BlockGeometry;
const MBR_SIGNATURE: [u8; 2] = [0x55, 0xAA];
const PARTITION_TABLE_OFFSET: usize = 446;
const PARTITION_ENTRY_SIZE: usize = 16;
const PARTITION_COUNT: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MbrError {
    BufferTooSmall, InvalidSignature, InvalidBlockGeometry, InvalidBootIndicator,
    InvalidPartitionRange, OverlappingPartitions, AddressOverflow, InvalidExtendedPartition,
    InvalidEbrEntry, InvalidEbrLink, EbrCycle, EbrTraversalLimit, LogicalPartitionOutputFull,
    DeviceReadFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MbrPartition {
    pub bootable: bool,
    pub partition_type: u8,
    pub start_lba: u32,
    pub sector_count: u32,
}

impl MbrPartition {
    pub const fn is_empty(self) -> bool { self.partition_type == 0 || self.sector_count == 0 }
    pub const fn is_extended(self) -> bool {
        self.partition_type == 0x05 || self.partition_type == 0x0F || self.partition_type == 0x85
    }
    pub const fn is_protective_gpt(self) -> bool { self.partition_type == 0xEE }
    pub const fn end_lba_exclusive(self) -> Result<u64, MbrError> {
        match (self.start_lba as u64).checked_add(self.sector_count as u64) {
            Some(end) => Ok(end), None => Err(MbrError::AddressOverflow),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MbrTable {
    pub disk_signature: u32,
    pub partitions: [Option<MbrPartition>; PARTITION_COUNT],
}

impl MbrTable {
    /// Parse and validate the primary partition table against device geometry.
    pub fn parse(sector: &[u8], geometry: BlockGeometry) -> Result<Self, MbrError> {
        let geometry = geometry.validate().map_err(|_| MbrError::InvalidBlockGeometry)?;
        if sector.len() < geometry.block_size as usize || sector.len() < 512 {
            return Err(MbrError::BufferTooSmall);
        }
        if sector[510..512] != MBR_SIGNATURE { return Err(MbrError::InvalidSignature); }
        let disk_signature = read_u32(sector, 440)?;
        let mut partitions = [None; PARTITION_COUNT];

        for (index, slot) in partitions.iter_mut().enumerate() {
            let offset = PARTITION_TABLE_OFFSET + index * PARTITION_ENTRY_SIZE;
            let boot = sector[offset];
            let partition_type = sector[offset + 4];
            let start_lba = read_u32(sector, offset + 8)?;
            let sector_count = read_u32(sector, offset + 12)?;
            if partition_type == 0 && start_lba == 0 && sector_count == 0 {
                if boot != 0 { return Err(MbrError::InvalidBootIndicator); }
                continue;
            }
            if boot != 0 && boot != 0x80 { return Err(MbrError::InvalidBootIndicator); }
            if partition_type == 0 || sector_count == 0 || start_lba == 0 {
                return Err(MbrError::InvalidPartitionRange);
            }
            let partition = MbrPartition { bootable: boot == 0x80, partition_type, start_lba, sector_count };
            if partition.end_lba_exclusive()? > geometry.block_count {
                return Err(MbrError::InvalidPartitionRange);
            }
            *slot = Some(partition);
        }

        // Primary entries must not overlap, including extended-container ranges.
        for left in 0..PARTITION_COUNT {
            let Some(a) = partitions[left] else { continue };
            let a_start = a.start_lba as u64;
            let a_end = a.end_lba_exclusive()?;
            for b in partitions.iter().skip(left + 1).flatten() {
                let b_start = b.start_lba as u64;
                let b_end = b.end_lba_exclusive()?;
                if a_start < b_end && b_start < a_end { return Err(MbrError::OverlappingPartitions); }
            }
        }
        Ok(Self { disk_signature, partitions })
    }

    /// Number of populated primary entries (extended containers included).
    pub fn partition_count(&self) -> usize {
        self.partitions.iter().filter(|entry| entry.is_some()).count()
    }
    /// Return the protective GPT entry, if present.
    pub fn protective_partition(&self) -> Option<MbrPartition> {
        self.partitions.iter().flatten().copied().find(|p| p.is_protective_gpt())
    }
    /// Iterate over populated primary partitions in table order.
    pub fn iter(&self) -> impl Iterator<Item = &MbrPartition> {
        self.partitions.iter().filter_map(Option::as_ref)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LogicalPartition {
    pub bootable: bool,
    pub partition_type: u8,
    pub start_lba: u64,
    pub sector_count: u32,
}
impl LogicalPartition {
    pub const fn end_lba_exclusive(self) -> Result<u64, MbrError> {
        match self.start_lba.checked_add(self.sector_count as u64) {
            Some(end) => Ok(end), None => Err(MbrError::AddressOverflow),
        }
    }
}

/// Follow an extended partition EBR chain without allocation.
/// scratch must be exactly one logical block. The caller supplies cycle-detection
/// storage and output capacity; traversal fails rather than silently truncating.
pub fn enumerate_logical_partitions<D: super::block::BlockDevice>(
    device: &mut D, extended: MbrPartition, scratch: &mut [u8],
    visited_ebr_lbas: &mut [u64], output: &mut [Option<LogicalPartition>],
) -> Result<usize, MbrError> {
    use super::block::{BlockRange, validate_buffer};
    if !extended.is_extended() || extended.sector_count == 0 || extended.start_lba == 0 {
        return Err(MbrError::InvalidExtendedPartition);
    }
    let geometry = device.geometry().validate().map_err(|_| MbrError::InvalidBlockGeometry)?;
    if scratch.len() != geometry.block_size as usize { return Err(MbrError::BufferTooSmall); }
    let base = extended.start_lba as u64;
    let limit = extended.end_lba_exclusive()?;
    if limit > geometry.block_count { return Err(MbrError::InvalidPartitionRange); }

    output.fill(None);
    let mut current = base;
    let mut visited_count = 0usize;
    let mut output_count = 0usize;
    loop {
        if current < base || current >= limit { return Err(MbrError::InvalidEbrLink); }
        if visited_ebr_lbas[..visited_count].contains(&current) { return Err(MbrError::EbrCycle); }
        for partition in output[..output_count].iter().flatten() {
            if current >= partition.start_lba && current < partition.end_lba_exclusive()? {
                return Err(MbrError::OverlappingPartitions);
            }
        }
        if visited_count == visited_ebr_lbas.len() { return Err(MbrError::EbrTraversalLimit); }
        visited_ebr_lbas[visited_count] = current;
        visited_count += 1;

        let range = BlockRange::new(current, 1);
        validate_buffer(geometry, range, scratch.len()).map_err(|_| MbrError::InvalidPartitionRange)?;
        device.read_blocks(range, scratch).map_err(|_| MbrError::DeviceReadFailed)?;
        if scratch[510..512] != MBR_SIGNATURE { return Err(MbrError::InvalidSignature); }

        let first = parse_ebr_entry(scratch, PARTITION_TABLE_OFFSET)?;
        let link = parse_ebr_entry(scratch, PARTITION_TABLE_OFFSET + PARTITION_ENTRY_SIZE)?;
        for index in 2..PARTITION_COUNT {
            if parse_ebr_entry(scratch, PARTITION_TABLE_OFFSET + index * PARTITION_ENTRY_SIZE)?.is_some() {
                return Err(MbrError::InvalidEbrEntry);
            }
        }

        if let Some((bootable, kind, relative, count)) = first {
            if kind == 0 || matches!(kind, 0x05 | 0x0F | 0x85) || count == 0 || relative == 0 {
                return Err(MbrError::InvalidEbrEntry);
            }
            let start = current.checked_add(relative as u64).ok_or(MbrError::AddressOverflow)?;
            let partition = LogicalPartition { bootable, partition_type: kind, start_lba: start, sector_count: count };
            let end = partition.end_lba_exclusive()?;
            if start < base || end > limit || start == current
                || visited_ebr_lbas[..visited_count].iter().any(|lba| *lba >= start && *lba < end)
            { return Err(MbrError::InvalidPartitionRange); }
            if output_count == output.len() { return Err(MbrError::LogicalPartitionOutputFull); }
            for previous in output[..output_count].iter().flatten() {
                if start < previous.end_lba_exclusive()? && previous.start_lba < end {
                    return Err(MbrError::OverlappingPartitions);
                }
            }
            output[output_count] = Some(partition);
            output_count += 1;
        }

        match link {
            None => break,
            Some((bootable, kind, relative, count)) => {
                if bootable || count == 0 || relative == 0 { return Err(MbrError::InvalidEbrLink); }
                let target = base.checked_add(relative as u64).ok_or(MbrError::AddressOverflow)?;
                let link_end = target.checked_add(count as u64).ok_or(MbrError::AddressOverflow)?;
                if !matches!(kind, 0x05 | 0x0F | 0x85) || target < base || target >= limit || link_end > limit {
                    return Err(MbrError::InvalidEbrLink);
                }
                current = target;
            }
        }
    }
    Ok(output_count)
}

fn parse_ebr_entry(sector: &[u8], offset: usize) -> Result<Option<(bool, u8, u32, u32)>, MbrError> {
    let boot = *sector.get(offset).ok_or(MbrError::BufferTooSmall)?;
    let kind = *sector.get(offset + 4).ok_or(MbrError::BufferTooSmall)?;
    let start = read_u32(sector, offset + 8)?;
    let count = read_u32(sector, offset + 12)?;
    if boot == 0 && kind == 0 && start == 0 && count == 0 { return Ok(None); }
    if boot != 0 && boot != 0x80 { return Err(MbrError::InvalidBootIndicator); }
    Ok(Some((boot == 0x80, kind, start, count)))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, MbrError> {
    let raw = bytes.get(offset..offset + 4).ok_or(MbrError::BufferTooSmall)?;
    Ok(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;
    const GEOMETRY: BlockGeometry = BlockGeometry { block_count: 16_384, block_size: 512 };
    fn empty_mbr() -> [u8; 512] {
        let mut sector = [0u8; 512]; sector[510] = 0x55; sector[511] = 0xAA; sector
    }
    fn set_partition(sector: &mut [u8; 512], index: usize, boot: u8, kind: u8, start: u32, count: u32) {
        let offset = PARTITION_TABLE_OFFSET + index * PARTITION_ENTRY_SIZE;
        sector[offset] = boot; sector[offset + 4] = kind;
        sector[offset + 8..offset + 12].copy_from_slice(&start.to_le_bytes());
        sector[offset + 12..offset + 16].copy_from_slice(&count.to_le_bytes());
    }
    #[test]
    fn parses_primary_partitions() {
        let mut sector = empty_mbr();
        sector[440..444].copy_from_slice(&0x1234_5678u32.to_le_bytes());
        set_partition(&mut sector, 0, 0x80, 0x83, 2048, 4096);
        set_partition(&mut sector, 1, 0, 0x07, 6144, 2048);
        let table = MbrTable::parse(&sector, GEOMETRY).unwrap();
        assert_eq!(table.disk_signature, 0x1234_5678);
        assert_eq!(table.partition_count(), 2);
        assert!(table.partitions[0].unwrap().bootable);
        assert_eq!(table.partitions[1].unwrap().partition_type, 0x07);
    }
    #[test]
    fn detects_protective_gpt_entry() {
        let mut sector = empty_mbr(); set_partition(&mut sector, 0, 0, 0xEE, 1, 16_383);
        assert!(MbrTable::parse(&sector, GEOMETRY).unwrap().protective_partition().is_some());
    }
    #[test]
    fn rejects_bad_signature_and_boot_flag() {
        let mut sector = [0u8; 512];
        assert_eq!(MbrTable::parse(&sector, GEOMETRY), Err(MbrError::InvalidSignature));
        sector = empty_mbr(); set_partition(&mut sector, 0, 1, 0x83, 1, 10);
        assert_eq!(MbrTable::parse(&sector, GEOMETRY), Err(MbrError::InvalidBootIndicator));
    }
    #[test]
    fn rejects_out_of_bounds_and_overlapping_entries() {
        let mut sector = empty_mbr(); set_partition(&mut sector, 0, 0, 0x83, 16_380, 10);
        assert_eq!(MbrTable::parse(&sector, GEOMETRY), Err(MbrError::InvalidPartitionRange));
        sector = empty_mbr(); set_partition(&mut sector, 0, 0, 0x83, 100, 100);
        set_partition(&mut sector, 1, 0, 0x07, 150, 100);
        assert_eq!(MbrTable::parse(&sector, GEOMETRY), Err(MbrError::OverlappingPartitions));
    }
    struct MockDisk { first: [u8; 512], second: [u8; 512] }
    impl crate::drivers::storage::block::BlockDevice for MockDisk {
        fn geometry(&self) -> BlockGeometry { GEOMETRY }
        fn read_blocks(&mut self, range: crate::drivers::storage::block::BlockRange, buffer: &mut [u8]) -> Result<(), crate::drivers::storage::block::BlockError> {
            if range.block_count != 1 || buffer.len() != 512 { return Err(crate::drivers::storage::block::BlockError::BufferSizeMismatch); }
            match range.start_block {
                100 => buffer.copy_from_slice(&self.first),
                200 => buffer.copy_from_slice(&self.second),
                _ => return Err(crate::drivers::storage::block::BlockError::OutOfBounds),
            }
            Ok(())
        }
        fn write_blocks(&mut self, _: crate::drivers::storage::block::BlockRange, _: &[u8]) -> Result<(), crate::drivers::storage::block::BlockError> {
            Err(crate::drivers::storage::block::BlockError::UnsupportedOperation)
        }
        fn flush(&mut self) -> Result<(), crate::drivers::storage::block::BlockError> { Ok(()) }
    }
    fn set_raw_entry(sector: &mut [u8; 512], index: usize, boot: u8, kind: u8, start: u32, count: u32) {
        set_partition(sector, index, boot, kind, start, count);
    }
    #[test]
    fn follows_ebr_chain_and_returns_absolute_lbas() {
        let mut first = empty_mbr();
        set_raw_entry(&mut first, 0, 0, 0x83, 1, 20);
        set_raw_entry(&mut first, 1, 0, 0x0F, 100, 400);
        let mut second = empty_mbr();
        set_raw_entry(&mut second, 0, 0x80, 0x07, 1, 30);
        let mut disk = MockDisk { first, second };
        let mut scratch = [0u8; 512];
        let mut visited = [0u64; 4];
        let mut output = [None; 4];
        let count = enumerate_logical_partitions(
            &mut disk,
            MbrPartition { bootable: false, partition_type: 0x0F, start_lba: 100, sector_count: 500 },
            &mut scratch, &mut visited, &mut output,
        ).unwrap();
        assert_eq!(count, 2);
        assert_eq!(output[0].unwrap().start_lba, 101);
        assert_eq!(output[1].unwrap().start_lba, 201);
        assert!(output[1].unwrap().bootable);
    }
    #[test]
    fn rejects_ebr_cycles() {
        let mut first = empty_mbr();
        set_raw_entry(&mut first, 0, 0, 0x83, 1, 20);
        set_raw_entry(&mut first, 1, 0, 0x0F, 100, 400);
        let mut second = empty_mbr();
        set_raw_entry(&mut second, 1, 0, 0x0F, 100, 400);
        let mut disk = MockDisk { first, second };
        let mut scratch = [0u8; 512];
        let mut visited = [0u64; 4];
        let mut output = [None; 4];
        let result = enumerate_logical_partitions(
            &mut disk,
            MbrPartition { bootable: false, partition_type: 0x0F, start_lba: 100, sector_count: 500 },
            &mut scratch, &mut visited, &mut output,
        );
        assert_eq!(result, Err(MbrError::EbrCycle));
    }
}
