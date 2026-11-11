//! Allocation-free Master Boot Record parsing and primary partition enumeration.
use super::block::BlockGeometry;
const MBR_SIGNATURE: [u8; 2] = [0x55, 0xAA];
const PARTITION_TABLE_OFFSET: usize = 446;
const PARTITION_ENTRY_SIZE: usize = 16;
const PARTITION_COUNT: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MbrError {
    BufferTooSmall, InvalidSignature, InvalidBlockGeometry, InvalidBootIndicator,
    InvalidPartitionRange, OverlappingPartitions, AddressOverflow,
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
}
