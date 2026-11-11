//! FAT12/16/32 boot-sector parsing and read-only volume probing.
//! Parsing is allocation-free; mounting and file operations are separate layers.
use super::block::{BlockDevice, BlockError, BlockRange};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FatKind { Fat12, Fat16, Fat32 }
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FatError {
    Block(BlockError), BufferTooSmall, InvalidJump, InvalidSignature,
    InvalidBytesPerSector, SectorSizeMismatch, InvalidSectorsPerCluster,
    InvalidReservedSectorCount, InvalidFatCount, InvalidRootDirectory,
    InvalidTotalSectors, InvalidFatSize, InvalidClusterCount, FatTooSmall,
    InvalidFat32Metadata, UnsupportedFilesystem, InvalidCluster, FatEntryIo,
    FatSectorBufferTooSmall, ClusterBufferTooSmall, ChainCorrupt, ChainTooLong,
    FileSizeMismatch, InvalidDirectoryEntry, DirectoryFull,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FatBpb {
    pub kind: FatKind, pub bytes_per_sector: u16, pub sectors_per_cluster: u8,
    pub reserved_sectors: u16, pub fat_count: u8, pub root_entry_count: u16,
    pub total_sectors: u32, pub sectors_per_fat: u32,
    pub root_directory_sectors: u32, pub first_data_sector: u32,
    pub data_sectors: u32, pub cluster_count: u32,
    pub root_cluster: Option<u32>, pub fs_info_sector: Option<u16>,
    pub backup_boot_sector: Option<u16>, pub volume_id: u32,
}
fn le16(b: &[u8], o: usize) -> u16 { u16::from_le_bytes([b[o], b[o + 1]]) }
fn le32(b: &[u8], o: usize) -> u32 { u32::from_le_bytes([b[o], b[o+1], b[o+2], b[o+3]]) }

/// Parse a complete FAT boot sector and validate its BPB and data layout.
pub fn parse_bpb(sector: &[u8], device_block_size: u32) -> Result<FatBpb, FatError> {
    if sector.len() < 512 { return Err(FatError::BufferTooSmall); }
    if sector[510] != 0x55 || sector[511] != 0xAA { return Err(FatError::InvalidSignature); }
    if !(sector[0] == 0xE9 || (sector[0] == 0xEB && sector[2] == 0x90)) { return Err(FatError::InvalidJump); }
    let bps = le16(sector, 11);
    if !(512..=4096).contains(&bps) || !bps.is_power_of_two() { return Err(FatError::InvalidBytesPerSector); }
    if bps as u32 != device_block_size || sector.len() < bps as usize { return Err(FatError::SectorSizeMismatch); }
    let spc = sector[13];
    if spc == 0 || !spc.is_power_of_two() || spc > 128 { return Err(FatError::InvalidSectorsPerCluster); }
    let reserved = le16(sector, 14);
    if reserved == 0 { return Err(FatError::InvalidReservedSectorCount); }
    let fats = sector[16];
    if fats == 0 || fats > 4 { return Err(FatError::InvalidFatCount); }
    let root_entries = le16(sector, 17);
    let total16 = le16(sector, 19) as u32;
    let total32 = le32(sector, 32);
    let total = if total16 != 0 { total16 } else { total32 };
    if total == 0 || (total16 != 0 && total32 != 0) { return Err(FatError::InvalidTotalSectors); }
    let fat16 = le16(sector, 22) as u32;
    let fat32 = le32(sector, 36);
    let spf = if fat16 != 0 { fat16 } else { fat32 };
    if spf == 0 { return Err(FatError::InvalidFatSize); }
    let root_bytes = (root_entries as u32).checked_mul(32).ok_or(FatError::InvalidRootDirectory)?;
    let root_secs = root_bytes.checked_add(bps as u32 - 1).ok_or(FatError::InvalidRootDirectory)? / bps as u32;
    let fats_secs = (fats as u32).checked_mul(spf).ok_or(FatError::InvalidFatSize)?;
    let first_data = (reserved as u32).checked_add(fats_secs).and_then(|v| v.checked_add(root_secs)).ok_or(FatError::InvalidTotalSectors)?;
    if first_data >= total { return Err(FatError::InvalidTotalSectors); }
    let data_secs = total - first_data;
    let clusters = data_secs / spc as u32;
    let kind = if clusters < 4_085 { FatKind::Fat12 } else if clusters < 65_525 { FatKind::Fat16 } else { FatKind::Fat32 };
    if (kind == FatKind::Fat32 && (root_entries != 0 || fat16 != 0))
        || (kind != FatKind::Fat32 && (root_entries == 0 || fat16 == 0)) {
        return Err(FatError::InvalidRootDirectory);
    }
    if clusters == 0 || clusters >= 0x0FFF_FFF5 { return Err(FatError::InvalidClusterCount); }
    let bits = match kind { FatKind::Fat12 => 12u64, FatKind::Fat16 => 16, FatKind::Fat32 => 32 };
    let required = (clusters as u64 + 2).checked_mul(bits).and_then(|v| v.checked_add(7)).ok_or(FatError::FatTooSmall)? / 8;
    let available = (spf as u64).checked_mul(bps as u64).ok_or(FatError::FatTooSmall)?;
    if available < required { return Err(FatError::FatTooSmall); }
    let (root_cluster, fs_info, backup, volume_id) = if kind == FatKind::Fat32 {
        let root = le32(sector, 44) & 0x0FFF_FFFF;
        let fs = le16(sector, 48);
        let bk = le16(sector, 50);
        if root < 2 || root >= clusters + 2 || fs == 0xFFFF || bk == 0xFFFF
            || (fs != 0 && fs >= reserved) || (bk != 0 && bk >= reserved) {
            return Err(FatError::InvalidFat32Metadata);
        }
        (Some(root), if fs == 0 { None } else { Some(fs) }, if bk == 0 { None } else { Some(bk) }, le32(sector, 67))
    } else { (None, None, None, le32(sector, 39)) };
    Ok(FatBpb { kind, bytes_per_sector: bps, sectors_per_cluster: spc,
        reserved_sectors: reserved, fat_count: fats, root_entry_count: root_entries,
        total_sectors: total, sectors_per_fat: spf, root_directory_sectors: root_secs,
        first_data_sector: first_data, data_sectors: data_secs, cluster_count: clusters,
        root_cluster, fs_info_sector: fs_info, backup_boot_sector: backup, volume_id })
}

/// Probe the boot sector of a partition-relative block device; performs no writes.
pub fn probe_fat<D: BlockDevice>(device: &mut D, sector_buffer: &mut [u8]) -> Result<FatBpb, FatError> {
    let geometry = device.geometry().validate().map_err(FatError::Block)?;
    if sector_buffer.len() != geometry.block_size as usize { return Err(FatError::BufferTooSmall); }
    device.read_blocks(BlockRange::new(0, 1), sector_buffer).map_err(FatError::Block)?;
    parse_bpb(sector_buffer, geometry.block_size)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fat16_boot() -> [u8; 512] {
        let mut b = [0u8; 512]; b[0..3].copy_from_slice(&[0xEB,0x3C,0x90]);
        b[11..13].copy_from_slice(&512u16.to_le_bytes()); b[13]=1;
        b[14..16].copy_from_slice(&1u16.to_le_bytes()); b[16]=2;
        b[17..19].copy_from_slice(&512u16.to_le_bytes());
        b[19..21].copy_from_slice(&10_000u16.to_le_bytes());
        b[22..24].copy_from_slice(&40u16.to_le_bytes());
        b[39..43].copy_from_slice(&0x1234_5678u32.to_le_bytes());
        b[510]=0x55; b[511]=0xAA; b
    }
    #[test] fn parses_fat16_layout() {
        let p=parse_bpb(&fat16_boot(),512).unwrap();
        assert_eq!(p.kind,FatKind::Fat16); assert_eq!(p.first_data_sector,113);
        assert_eq!(p.cluster_count,9_887); assert_eq!(p.volume_id,0x1234_5678);
    }

    #[test] fn parses_fat12_layout() {
        let mut b=[0u8;512]; b[0..3].copy_from_slice(&[0xEB,0x3C,0x90]);
        b[11..13].copy_from_slice(&512u16.to_le_bytes()); b[13]=1;
        b[14..16].copy_from_slice(&1u16.to_le_bytes()); b[16]=2;
        b[17..19].copy_from_slice(&224u16.to_le_bytes()); b[19..21].copy_from_slice(&2880u16.to_le_bytes());
        b[22..24].copy_from_slice(&9u16.to_le_bytes()); b[510]=0x55; b[511]=0xAA;
        let p=parse_bpb(&b,512).unwrap(); assert_eq!(p.kind,FatKind::Fat12);
    }
    #[test] fn parses_fat32_layout() {
        let mut b=[0u8;512]; b[0..3].copy_from_slice(&[0xEB,0x58,0x90]);
        b[11..13].copy_from_slice(&512u16.to_le_bytes()); b[13]=1;
        b[14..16].copy_from_slice(&32u16.to_le_bytes()); b[16]=2;
        b[32..36].copy_from_slice(&70_000u32.to_le_bytes());
        b[36..40].copy_from_slice(&600u32.to_le_bytes());
        b[44..48].copy_from_slice(&2u32.to_le_bytes());
        b[48..50].copy_from_slice(&1u16.to_le_bytes()); b[50..52].copy_from_slice(&6u16.to_le_bytes());
        b[67..71].copy_from_slice(&0x8765_4321u32.to_le_bytes());
        b[510]=0x55; b[511]=0xAA;
        let p=parse_bpb(&b,512).unwrap(); assert_eq!(p.kind,FatKind::Fat32);
        assert_eq!(p.root_cluster,Some(2)); assert_eq!(p.fs_info_sector,Some(1));
        assert_eq!(p.backup_boot_sector,Some(6)); assert_eq!(p.volume_id,0x8765_4321);
    }
    #[test] fn rejects_bad_signature_jump_sector_and_cluster() {
        let mut b=fat16_boot(); b[510]=0;
        assert_eq!(parse_bpb(&b,512),Err(FatError::InvalidSignature));
        b=fat16_boot(); b[0]=0;
        assert_eq!(parse_bpb(&b,512),Err(FatError::InvalidJump));
        b=fat16_boot(); b[11..13].copy_from_slice(&1024u16.to_le_bytes());
        assert_eq!(parse_bpb(&b,512),Err(FatError::SectorSizeMismatch));
        b=fat16_boot(); b[13]=3;
        assert_eq!(parse_bpb(&b,512),Err(FatError::InvalidSectorsPerCluster));
    }
    #[test] fn rejects_undersized_fat() {
        let mut b=fat16_boot(); b[22..24].copy_from_slice(&1u16.to_le_bytes());
        assert_eq!(parse_bpb(&b,512),Err(FatError::FatTooSmall));
    }
    struct Disk([u8;512],usize);
    impl BlockDevice for Disk {
        fn geometry(&self)->super::super::block::BlockGeometry { super::super::block::BlockGeometry{block_count:1,block_size:512} }
        fn read_blocks(&mut self,r:BlockRange,o:&mut[u8])->Result<(),BlockError>{
            super::super::block::validate_buffer(self.geometry(),r,o.len())?; self.1+=1; o.copy_from_slice(&self.0); Ok(())
        }
        fn write_blocks(&mut self,_:BlockRange,_:&[u8])->Result<(),BlockError>{Err(BlockError::UnsupportedOperation)}
        fn flush(&mut self)->Result<(),BlockError>{Ok(())}
    }
    #[test] fn probe_reads_only_boot_sector(){
        let mut d=Disk(fat16_boot(),0); let mut buf=[0;512];
        assert_eq!(probe_fat(&mut d,&mut buf).unwrap().kind,FatKind::Fat16); assert_eq!(d.1,1);
    }
}

/// Result of decoding a FAT allocation-table entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FatChainValue { Next(u32), End }

/// Decoded short-name directory record (LFN and volume-label records are omitted).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FatDirEntry {
    pub short_name: [u8; 11],
    pub attributes: u8,
    pub first_cluster: u32,
    pub file_size: u32,
}
impl FatDirEntry {
    pub fn parse(raw: &[u8]) -> Result<Option<Self>, FatError> {
        if raw.len() < 32 { return Err(FatError::InvalidDirectoryEntry); }
        if raw[0] == 0x00 || raw[0] == 0xE5 || raw[11] == 0x0F || raw[11] & 0x08 != 0 {
            return Ok(None);
        }
        let mut name = [0u8; 11];
        name.copy_from_slice(&raw[..11]);
        Ok(Some(Self {
            short_name: name,
            attributes: raw[11],
            first_cluster: ((le16(raw, 20) as u32) << 16) | le16(raw, 26) as u32,
            file_size: le32(raw, 28),
        }))
    }
    pub const fn is_directory(&self) -> bool { self.attributes & 0x10 != 0 }
}
