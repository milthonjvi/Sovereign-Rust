//! Read-only FAT cluster-chain and file-data operations.
use super::block::{BlockDevice, BlockError, BlockRange};
use super::fat::{FatBpb, FatError, FatKind, FatDirEntry, FatChainValue};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FatVolume {
    pub bpb: FatBpb,
    pub geometry: super::block::BlockGeometry,
}
impl FatVolume {
    pub fn new(bpb: FatBpb, geometry: super::block::BlockGeometry) -> Result<Self, FatError> {
        let geometry = geometry.validate().map_err(FatError::Block)?;
        if bpb.bytes_per_sector as u32 != geometry.block_size || bpb.total_sectors as u64 > geometry.block_count {
            return Err(FatError::InvalidTotalSectors);
        }
        Ok(Self { bpb, geometry })
    }
    pub const fn cluster_size_bytes(&self) -> usize {
        self.bpb.bytes_per_sector as usize * self.bpb.sectors_per_cluster as usize
    }
    pub fn first_sector_of_cluster(&self, cluster: u32) -> Result<u64, FatError> {
        if cluster < 2 || cluster >= self.bpb.cluster_count + 2 { return Err(FatError::InvalidCluster); }
        let relative = (cluster - 2).checked_mul(self.bpb.sectors_per_cluster as u32).ok_or(FatError::InvalidCluster)?;
        let sector = self.bpb.first_data_sector.checked_add(relative).ok_or(FatError::InvalidCluster)?;
        if sector >= self.bpb.total_sectors { return Err(FatError::InvalidCluster); }
        Ok(sector as u64)
    }
}

fn read_fat_bytes<D: BlockDevice>(device: &mut D, volume: &FatVolume, offset: u64, out: &mut [u8], scratch: &mut [u8]) -> Result<(), FatError> {
    let bps = volume.bpb.bytes_per_sector as usize;
    if scratch.len() < bps * 2 { return Err(FatError::FatSectorBufferTooSmall); }
    let sector_index = offset / bps as u64;
    let within = (offset % bps as u64) as usize;
    let count = if within + out.len() > bps { 2u64 } else { 1u64 };
    let fat_start = volume.bpb.reserved_sectors as u64;
    let absolute = fat_start.checked_add(sector_index).ok_or(FatError::FatEntryIo)?;
    let fat_end = fat_start.checked_add(volume.bpb.sectors_per_fat as u64).ok_or(FatError::FatEntryIo)?;
    if absolute.checked_add(count).filter(|end| *end <= fat_end).is_none() { return Err(FatError::FatEntryIo); }
    let bytes = bps.checked_mul(count as usize).ok_or(FatError::FatEntryIo)?;
    device.read_blocks(BlockRange::new(absolute, count), &mut scratch[..bytes]).map_err(FatError::Block)?;
    out.copy_from_slice(&scratch[within..within + out.len()]);
    Ok(())
}

/// Resolve a FAT entry, including FAT12 packed entries that cross sector boundaries.
pub fn read_fat_entry<D: BlockDevice>(device: &mut D, volume: &FatVolume, cluster: u32, scratch: &mut [u8]) -> Result<FatChainValue, FatError> {
    if cluster >= volume.bpb.cluster_count + 2 { return Err(FatError::InvalidCluster); }
    let (offset, width) = match volume.bpb.kind {
        FatKind::Fat12 => ((cluster as u64 * 3) / 2, 2usize),
        FatKind::Fat16 => (cluster as u64 * 2, 2),
        FatKind::Fat32 => (cluster as u64 * 4, 4),
    };
    let mut raw = [0u8; 4];
    read_fat_bytes(device, volume, offset, &mut raw[..width], scratch)?;
    let value = match volume.bpb.kind {
        FatKind::Fat12 => { let v = u16::from_le_bytes([raw[0], raw[1]]); if cluster & 1 == 0 { (v & 0x0fff) as u32 } else { (v >> 4) as u32 } }
        FatKind::Fat16 => u16::from_le_bytes([raw[0], raw[1]]) as u32,
        FatKind::Fat32 => u32::from_le_bytes(raw) & 0x0fff_ffff,
    };
    let (eoc, bad, reserved) = match volume.bpb.kind {
        FatKind::Fat12 => (0x0ff8, 0x0ff7, 0x0ff0),
        FatKind::Fat16 => (0xfff8, 0xfff7, 0xfff0),
        FatKind::Fat32 => (0x0fff_fff8, 0x0fff_fff7, 0x0fff_fff0),
    };
    if value >= eoc { return Ok(FatChainValue::End); }
    if value < 2 || value == bad || value >= reserved || value >= volume.bpb.cluster_count + 2 {
        return Err(FatError::ChainCorrupt);
    }
    Ok(FatChainValue::Next(value))
}

/// Read one cluster. The destination must be exactly one cluster in size.
pub fn read_cluster<D: BlockDevice>(device: &mut D, volume: &FatVolume, cluster: u32, out: &mut [u8]) -> Result<(), FatError> {
    if out.len() != volume.cluster_size_bytes() { return Err(FatError::ClusterBufferTooSmall); }
    let first = volume.first_sector_of_cluster(cluster)?;
    let count = volume.bpb.sectors_per_cluster as u64;
    if first.checked_add(count).filter(|end| *end <= volume.bpb.total_sectors as u64).is_none() { return Err(FatError::InvalidCluster); }
    device.read_blocks(BlockRange::new(first, count), out).map_err(FatError::Block)
}

/// Stream a regular file into an exact-size caller buffer; chain traversal is bounded.
pub fn read_file_data<D: BlockDevice>(device: &mut D, volume: &FatVolume, entry: FatDirEntry, out: &mut [u8], cluster_buf: &mut [u8], fat_scratch: &mut [u8]) -> Result<(), FatError> {
    if entry.is_directory() || out.len() != entry.file_size as usize { return Err(FatError::FileSizeMismatch); }
    if entry.file_size == 0 {
        if entry.first_cluster != 0 { return Err(FatError::ChainCorrupt); }
        return Ok(());
    }
    let mut cluster = entry.first_cluster;
    let mut written = 0usize;
    let mut visited = 0u32;
    while written < out.len() {
        if visited >= volume.bpb.cluster_count { return Err(FatError::ChainTooLong); }
        read_cluster(device, volume, cluster, cluster_buf)?;
        let count = core::cmp::min(cluster_buf.len(), out.len() - written);
        out[written..written + count].copy_from_slice(&cluster_buf[..count]);
        written += count;
        visited += 1;
        if written < out.len() {
            cluster = match read_fat_entry(device, volume, cluster, fat_scratch)? {
                FatChainValue::Next(next) => next,
                FatChainValue::End => return Err(FatError::FileSizeMismatch),
            };
        }
    }
    if matches!(read_fat_entry(device, volume, cluster, fat_scratch)?, FatChainValue::Next(_)) {
        return Err(FatError::FileSizeMismatch);
    }
    Ok(())
}
