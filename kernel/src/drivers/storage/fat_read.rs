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
        let bps = bpb.bytes_per_sector as u32;
        if bps != geometry.block_size || !(512..=4096).contains(&bps) || !bps.is_power_of_two()
            || bpb.total_sectors as u64 > geometry.block_count || bpb.total_sectors == 0
            || bpb.reserved_sectors == 0 || bpb.fat_count == 0 || bpb.sectors_per_fat == 0
            || bpb.sectors_per_cluster == 0 || !bpb.sectors_per_cluster.is_power_of_two()
            || bpb.sectors_per_cluster > 128 || bpb.active_fat >= bpb.fat_count {
            return Err(FatError::InvalidTotalSectors);
        }
        let fat_span = (bpb.fat_count as u32).checked_mul(bpb.sectors_per_fat)
            .ok_or(FatError::InvalidFatSize)?;
        let root_bytes = (bpb.root_entry_count as u32).checked_mul(32).ok_or(FatError::InvalidRootDirectory)?;
        let expected_root = root_bytes.checked_add(bps - 1).ok_or(FatError::InvalidRootDirectory)? / bps;
        if expected_root != bpb.root_directory_sectors { return Err(FatError::InvalidRootDirectory); }
        let expected_first = (bpb.reserved_sectors as u32).checked_add(fat_span)
            .and_then(|v| v.checked_add(bpb.root_directory_sectors))
            .ok_or(FatError::InvalidTotalSectors)?;
        if expected_first != bpb.first_data_sector || expected_first >= bpb.total_sectors {
            return Err(FatError::InvalidTotalSectors);
        }
        let data_sectors = bpb.total_sectors - expected_first;
        let clusters = data_sectors / bpb.sectors_per_cluster as u32;
        let kind = if clusters < 4_085 { FatKind::Fat12 } else if clusters < 65_525 { FatKind::Fat16 } else { FatKind::Fat32 };
        if clusters == 0 || clusters >= 0x0FFF_FFF5 || data_sectors != bpb.data_sectors || clusters != bpb.cluster_count || kind != bpb.kind
            || (kind == FatKind::Fat32 && (bpb.root_entry_count != 0 || bpb.root_cluster.map_or(true, |v| v < 2 || v >= clusters + 2)))
            || (kind != FatKind::Fat32 && (bpb.root_entry_count == 0 || bpb.root_cluster.is_some()))
            || (bpb.fat_mirroring_enabled && bpb.active_fat != 0)
            || bpb.fs_info_sector.map_or(false, |v| v >= bpb.reserved_sectors)
            || bpb.backup_boot_sector.map_or(false, |v| v >= bpb.reserved_sectors) {
            return Err(FatError::InvalidClusterCount);
        }
        let bits = match kind { FatKind::Fat12 => 12u64, FatKind::Fat16 => 16, FatKind::Fat32 => 32 };
        let required = (clusters as u64 + 2).checked_mul(bits).and_then(|v| v.checked_add(7))
            .ok_or(FatError::FatTooSmall)? / 8;
        let available = (bpb.sectors_per_fat as u64).checked_mul(bps as u64).ok_or(FatError::FatTooSmall)?;
        if available < required { return Err(FatError::FatTooSmall); }
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

fn read_fat_bytes<D: BlockDevice>(device: &mut D, volume: &FatVolume, offset: u64, fat_index: u8, out: &mut [u8], scratch: &mut [u8]) -> Result<(), FatError> {
    let bps = volume.bpb.bytes_per_sector as usize;
    if scratch.len() < bps * 2 { return Err(FatError::FatSectorBufferTooSmall); }
    let sector_index = offset / bps as u64;
    let within = (offset % bps as u64) as usize;
    let count = if within + out.len() > bps { 2u64 } else { 1u64 };
    if fat_index >= volume.bpb.fat_count { return Err(FatError::FatEntryIo); }
    let fat_start = (volume.bpb.reserved_sectors as u64)
        .checked_add((fat_index as u64).checked_mul(volume.bpb.sectors_per_fat as u64).ok_or(FatError::FatEntryIo)?)
        .ok_or(FatError::FatEntryIo)?;
    let absolute = fat_start.checked_add(sector_index).ok_or(FatError::FatEntryIo)?;
    let fat_end = fat_start.checked_add(volume.bpb.sectors_per_fat as u64).ok_or(FatError::FatEntryIo)?;
    if absolute.checked_add(count).filter(|end| *end <= fat_end).is_none() { return Err(FatError::FatEntryIo); }
    let bytes = bps.checked_mul(count as usize).ok_or(FatError::FatEntryIo)?;
    device.read_blocks(BlockRange::new(absolute, count), &mut scratch[..bytes]).map_err(FatError::Block)?;
    out.copy_from_slice(&scratch[within..within + out.len()]);
    Ok(())
}

/// Resolve a FAT entry, including FAT12 packed entries that cross sector boundaries.
pub fn read_fat_entry_from_copy<D: BlockDevice>(device: &mut D, volume: &FatVolume, fat_index: u8, cluster: u32, scratch: &mut [u8]) -> Result<FatChainValue, FatError> {
    if cluster >= volume.bpb.cluster_count + 2 { return Err(FatError::InvalidCluster); }
    let (offset, width) = match volume.bpb.kind {
        FatKind::Fat12 => ((cluster as u64 * 3) / 2, 2usize),
        FatKind::Fat16 => (cluster as u64 * 2, 2),
        FatKind::Fat32 => (cluster as u64 * 4, 4),
    };
    let mut raw = [0u8; 4];
    read_fat_bytes(device, volume, offset, fat_index, &mut raw[..width], scratch)?;
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


/// Enumerate short-name records from a directory into a fixed-capacity caller array.
/// Pass None for the FAT12/16 fixed root directory, or Some(cluster) for a
/// FAT32 root/subdirectory. Long-name fragments are intentionally skipped.
pub fn enumerate_directory<D: BlockDevice>(
    device: &mut D,
    volume: &FatVolume,
    start_cluster: Option<u32>,
    entries: &mut [Option<FatDirEntry>],
    cluster_buf: &mut [u8],
    fat_scratch: &mut [u8],
) -> Result<usize, FatError> {
    entries.fill(None);
    let mut count = 0usize;
    let bps = volume.bpb.bytes_per_sector as usize;
    let mut stopped = false;
    if start_cluster.is_none() {
        if volume.bpb.kind == FatKind::Fat32 || cluster_buf.len() < bps {
            return Err(FatError::InvalidDirectoryEntry);
        }
        let first = volume.bpb.reserved_sectors as u64
            + volume.bpb.fat_count as u64 * volume.bpb.sectors_per_fat as u64;
        for sector in 0..volume.bpb.root_directory_sectors as u64 {
            device.read_blocks(BlockRange::new(first + sector, 1), &mut cluster_buf[..bps])
                .map_err(FatError::Block)?;
            for raw in cluster_buf[..bps].chunks_exact(32) {
                if raw[0] == 0 { stopped = true; break; }
                if let Some(entry) = FatDirEntry::parse(raw)? {
                    if count == entries.len() { entries.fill(None); return Err(FatError::DirectoryFull); }
                    entries[count] = Some(entry);
                    count += 1;
                }
            }
            if stopped { break; }
        }
        return Ok(count);
    }
    if cluster_buf.len() != volume.cluster_size_bytes() { return Err(FatError::ClusterBufferTooSmall); }
    let mut cluster = start_cluster.unwrap();
    let mut visited = 0u32;
    loop {
        if visited >= volume.bpb.cluster_count { entries.fill(None); return Err(FatError::ChainTooLong); }
        read_cluster(device, volume, cluster, cluster_buf)?;
        for raw in cluster_buf.chunks_exact(32) {
            if raw[0] == 0 { stopped = true; break; }
            if let Some(entry) = FatDirEntry::parse(raw)? {
                if count == entries.len() { entries.fill(None); return Err(FatError::DirectoryFull); }
                entries[count] = Some(entry);
                count += 1;
            }
        }
        if stopped { break; }
        visited += 1;
        match read_fat_entry(device, volume, cluster, fat_scratch)? {
            FatChainValue::Next(next) => cluster = next,
            FatChainValue::End => break,
        }
    }
    Ok(count)
}

/// Read from the primary FAT copy (copy zero).
pub fn read_fat_entry<D: BlockDevice>(device: &mut D, volume: &FatVolume, cluster: u32, scratch: &mut [u8]) -> Result<FatChainValue, FatError> {
    read_fat_entry_from_copy(device, volume, volume.bpb.active_fat, cluster, scratch)
}

/// Compare a cluster-chain entry across all FAT copies and report divergence.
pub fn verify_fat_mirrors<D: BlockDevice>(
    device: &mut D, volume: &FatVolume, cluster: u32, scratch: &mut [u8],
) -> Result<FatChainValue, FatError> {
    let primary = read_fat_entry_from_copy(device, volume, volume.bpb.active_fat, cluster, scratch)?;
    if !volume.bpb.fat_mirroring_enabled { return Ok(primary); }
    let mut copy = 0u8;
    while copy < volume.bpb.fat_count {
        if copy != volume.bpb.active_fat {
            let candidate = read_fat_entry_from_copy(device, volume, copy, cluster, scratch)?;
            if candidate != primary { return Err(FatError::FatMirrorMismatch); }
        }
        copy += 1;
    }
    Ok(primary)
}


#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FatFsInfo {
    pub free_clusters: Option<u32>,
    pub next_free_cluster: Option<u32>,
}

/// Validate and decode the FAT32 FSInfo sector. Unknown (0xFFFFFFFF) counters
/// are represented as None rather than trusted or guessed.
pub fn read_fs_info<D: BlockDevice>(
    device: &mut D,
    volume: &FatVolume,
    sector_buffer: &mut [u8],
) -> Result<FatFsInfo, FatError> {
    if volume.bpb.kind != FatKind::Fat32 { return Err(FatError::InvalidFat32Metadata); }
    let sector = volume.bpb.fs_info_sector.ok_or(FatError::InvalidFat32Metadata)?;
    if sector_buffer.len() != volume.bpb.bytes_per_sector as usize {
        return Err(FatError::FsInfoBufferTooSmall);
    }
    if sector >= volume.bpb.reserved_sectors {
        return Err(FatError::InvalidFat32Metadata);
    }
    device.read_blocks(BlockRange::new(sector as u64, 1), sector_buffer).map_err(FatError::Block)?;
    if u32::from_le_bytes([sector_buffer[0],sector_buffer[1],sector_buffer[2],sector_buffer[3]]) != 0x4161_5252
        || u32::from_le_bytes([sector_buffer[484],sector_buffer[485],sector_buffer[486],sector_buffer[487]]) != 0x6141_7272
        || u32::from_le_bytes([sector_buffer[508],sector_buffer[509],sector_buffer[510],sector_buffer[511]]) != 0xAA55_0000 {
        return Err(FatError::InvalidFsInfo);
    }
    let free = u32::from_le_bytes([sector_buffer[488],sector_buffer[489],sector_buffer[490],sector_buffer[491]]);
    let next = u32::from_le_bytes([sector_buffer[492],sector_buffer[493],sector_buffer[494],sector_buffer[495]]);
    if (free != u32::MAX && free > volume.bpb.cluster_count)
        || (next != u32::MAX && (next < 2 || next >= volume.bpb.cluster_count + 2)) {
        return Err(FatError::InvalidFsInfo);
    }
    Ok(FatFsInfo {
        free_clusters: if free == u32::MAX { None } else { Some(free) },
        next_free_cluster: if next == u32::MAX { None } else { Some(next) },
    })
}


/// Read an arbitrary byte range of a regular file without allocating a full-file
/// buffer. Traversal is bounded by the volume cluster count and all arithmetic
/// is checked before any disk access.
pub fn read_file_range<D: BlockDevice>(
    device: &mut D,
    volume: &FatVolume,
    entry: FatDirEntry,
    offset: u64,
    output: &mut [u8],
    cluster_buffer: &mut [u8],
    fat_scratch: &mut [u8],
) -> Result<(), FatError> {
    if entry.is_directory() { return Err(FatError::FileSizeMismatch); }
    let end = offset.checked_add(output.len() as u64).ok_or(FatError::FileSizeMismatch)?;
    if end > entry.file_size as u64 { return Err(FatError::FileSizeMismatch); }
    if output.is_empty() { return Ok(()); }
    if entry.file_size == 0 || entry.first_cluster < 2 { return Err(FatError::ChainCorrupt); }
    if cluster_buffer.len() != volume.cluster_size_bytes() { return Err(FatError::ClusterBufferTooSmall); }
    let cluster_size = cluster_buffer.len() as u64;
    let mut cluster_index = offset / cluster_size;
    let mut within = (offset % cluster_size) as usize;
    let mut cluster = entry.first_cluster;
    let mut traversed = 0u32;
    while cluster_index != 0 {
        if traversed >= volume.bpb.cluster_count { return Err(FatError::ChainTooLong); }
        cluster = match read_fat_entry(device, volume, cluster, fat_scratch)? {
            FatChainValue::Next(next) => next,
            FatChainValue::End => return Err(FatError::FileSizeMismatch),
        };
        cluster_index -= 1;
        traversed += 1;
    }
    let mut written = 0usize;
    while written < output.len() {
        if traversed >= volume.bpb.cluster_count { return Err(FatError::ChainTooLong); }
        read_cluster(device, volume, cluster, cluster_buffer)?;
        let available = cluster_buffer.len() - within;
        let take = core::cmp::min(available, output.len() - written);
        output[written..written + take].copy_from_slice(&cluster_buffer[within..within + take]);
        written += take;
        traversed += 1;
        within = 0;
        if written < output.len() {
            cluster = match read_fat_entry(device, volume, cluster, fat_scratch)? {
                FatChainValue::Next(next) => next,
                FatChainValue::End => return Err(FatError::FileSizeMismatch),
            };
        }
    }
    Ok(())
}


#[cfg(test)]
mod range_tests {
    use super::*;
    use super::super::block::BlockGeometry;
    struct MemoryDisk([u8; 3072]);
    impl BlockDevice for MemoryDisk {
        fn geometry(&self) -> BlockGeometry { BlockGeometry { block_count: 6, block_size: 512 } }
        fn read_blocks(&mut self, range: BlockRange, out: &mut [u8]) -> Result<(), BlockError> {
            super::super::block::validate_buffer(self.geometry(), range, out.len())?;
            let start = range.start_block as usize * 512;
            out.copy_from_slice(&self.0[start..start + out.len()]);
            Ok(())
        }
        fn write_blocks(&mut self, _: BlockRange, _: &[u8]) -> Result<(), BlockError> { Err(BlockError::UnsupportedOperation) }
        fn flush(&mut self) -> Result<(), BlockError> { Ok(()) }
    }
    #[test]
    fn reads_range_across_cluster_boundary() {
        let bpb = FatBpb {
            kind: FatKind::Fat12, bytes_per_sector: 512, sectors_per_cluster: 1,
            reserved_sectors: 1, fat_count: 1, root_entry_count: 16,
            total_sectors: 6, sectors_per_fat: 1, root_directory_sectors: 1,
            first_data_sector: 3, data_sectors: 3, cluster_count: 3,
            root_cluster: None, fs_info_sector: None, backup_boot_sector: None,
            volume_id: 0, fat_mirroring_enabled: true, active_fat: 0,
        };
        let volume = FatVolume::new(bpb, BlockGeometry { block_count: 6, block_size: 512 }).unwrap();
        let mut disk = MemoryDisk([0;3072]);
        disk.0[512+3]=0x03; disk.0[512+4]=0xF0; disk.0[512+5]=0xFF;
        disk.0[3*512..4*512].fill(0x11);
        disk.0[4*512..5*512].fill(0x22);
        let entry = FatDirEntry { short_name: *b"FILE    BIN", attributes: 0x20, first_cluster: 2, file_size: 700 };
        let mut cluster = [0u8;512];
        let mut fat = [0u8;1024];
        let mut output = [0u8;20];
        read_file_range(&mut disk, &volume, entry, 500, &mut output, &mut cluster, &mut fat).unwrap();
        assert_eq!(&output[..12], &[0x11;12]);
        assert_eq!(&output[12..], &[0x22;8]);
    }
}
