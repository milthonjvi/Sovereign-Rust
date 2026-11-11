//! Allocation-free validation of GUID Partition Table headers.
use super::block::BlockGeometry;
const SIGNATURE: &[u8; 8] = b"EFI PART";
const MIN_HEADER: usize = 92;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GptError { BufferTooSmall, InvalidSignature, UnsupportedRevision, InvalidHeaderSize, HeaderCrcMismatch, InvalidLba, InvalidUsableRange, InvalidEntryGeometry, AddressOverflow, TableOutOfBounds, InvalidBlockGeometry, EntryArrayCrcMismatch, InvalidPartitionEntry, OverlappingPartitions, DuplicatePartitionGuid, PartitionOutputFull, DeviceReadFailed, NoValidHeader, HeaderPairMismatch }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GptHeader { pub revision:u32, pub header_size:u32, pub current_lba:u64, pub backup_lba:u64, pub first_usable_lba:u64, pub last_usable_lba:u64, pub disk_guid:[u8;16], pub entries_lba:u64, pub entry_count:u32, pub entry_size:u32, pub entries_crc32:u32 }

impl GptHeader {
    /// Parse a complete logical block containing a GPT header.
    pub fn parse(sector:&[u8], expected_lba:u64, geometry:BlockGeometry)->Result<Self,GptError> {
        let geometry=geometry.validate().map_err(|_|GptError::InvalidBlockGeometry)?;
        if sector.len()!=geometry.block_size as usize || sector.len()<MIN_HEADER { return Err(GptError::BufferTooSmall); }
        if &sector[..8]!=SIGNATURE { return Err(GptError::InvalidSignature); }
        let revision=u32_at(sector,8)?;
        if revision>>16!=1 { return Err(GptError::UnsupportedRevision); }
        let header_size=u32_at(sector,12)?;
        let size=header_size as usize;
        if size<MIN_HEADER || size>sector.len() { return Err(GptError::InvalidHeaderSize); }
        if crc32_zeroed(&sector[..size],16..20)!=u32_at(sector,16)? { return Err(GptError::HeaderCrcMismatch); }
        let current_lba=u64_at(sector,24)?; let backup_lba=u64_at(sector,32)?;
        let first_usable_lba=u64_at(sector,40)?; let last_usable_lba=u64_at(sector,48)?;
        let mut disk_guid=[0u8;16]; disk_guid.copy_from_slice(&sector[56..72]);
        let entries_lba=u64_at(sector,72)?; let entry_count=u32_at(sector,80)?;
        let entry_size=u32_at(sector,84)?; let entries_crc32=u32_at(sector,88)?;
        if current_lba!=expected_lba || current_lba>=geometry.block_count || backup_lba>=geometry.block_count || current_lba==backup_lba { return Err(GptError::InvalidLba); }
        if first_usable_lba>last_usable_lba || last_usable_lba>=geometry.block_count || (current_lba>=first_usable_lba && current_lba<=last_usable_lba) || (backup_lba>=first_usable_lba && backup_lba<=last_usable_lba) { return Err(GptError::InvalidUsableRange); }
        if entry_count==0 || entry_size<128 || entry_size%8!=0 { return Err(GptError::InvalidEntryGeometry); }
        let bytes=(entry_count as u64).checked_mul(entry_size as u64).ok_or(GptError::AddressOverflow)?;
        let block=geometry.block_size as u64;
        let blocks=bytes.checked_add(block-1).ok_or(GptError::AddressOverflow)?/block;
        let end=entries_lba.checked_add(blocks).ok_or(GptError::AddressOverflow)?;
        if entries_lba==0 || end>geometry.block_count
            || (entries_lba <= current_lba && current_lba < end)
            || (entries_lba <= backup_lba && backup_lba < end)
            || (entries_lba <= last_usable_lba && first_usable_lba < end)
        { return Err(GptError::TableOutOfBounds); }
        Ok(Self { revision,header_size,current_lba,backup_lba,first_usable_lba,last_usable_lba,disk_guid,entries_lba,entry_count,entry_size,entries_crc32 })
    }
}
fn u32_at(b:&[u8],o:usize)->Result<u32,GptError>{Ok(u32::from_le_bytes(b.get(o..o+4).ok_or(GptError::BufferTooSmall)?.try_into().map_err(|_|GptError::BufferTooSmall)?))}
fn u64_at(b:&[u8],o:usize)->Result<u64,GptError>{Ok(u64::from_le_bytes(b.get(o..o+8).ok_or(GptError::BufferTooSmall)?.try_into().map_err(|_|GptError::BufferTooSmall)?))}
fn crc32_zeroed(b:&[u8],zero:core::ops::Range<usize>)->u32{let mut c=!0u32;for(i,v)in b.iter().enumerate(){c^=if zero.contains(&i){0}else{*v as u32};for _ in 0..8{c=(c>>1)^(0xedb8_8320&0u32.wrapping_sub(c&1));}}!c}

/// Result of probing both redundant GPT header locations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GptHeaderPair {
    pub primary: Option<GptHeader>,
    pub backup: Option<GptHeader>,
}
impl GptHeaderPair {
    /// Prefer the primary header, falling back to the backup when necessary.
    pub const fn selected(self) -> Option<GptHeader> {
        match self.primary { Some(header) => Some(header), None => self.backup }
    }
}

/// Read and validate the primary (LBA 1) and backup (last LBA) GPT headers.
/// A single valid copy is sufficient for recovery; two valid copies must agree
/// on disk identity, usable range, and partition-array geometry/CRC.
pub fn read_gpt_header_pair<D: super::block::BlockDevice>(
    device: &mut D,
    scratch: &mut [u8],
) -> Result<GptHeaderPair, GptError> {
    use super::block::{BlockRange, validate_buffer};
    let geo = device.geometry().validate().map_err(|_| GptError::InvalidBlockGeometry)?;
    if scratch.len() != geo.block_size as usize { return Err(GptError::BufferTooSmall); }
    if geo.block_count < 3 { return Err(GptError::InvalidLba); }

    let primary = read_header_at(device, geo, 1, scratch);
    let backup_lba = geo.block_count - 1;
    let backup = read_header_at(device, geo, backup_lba, scratch);
    let pair = GptHeaderPair { primary: primary.ok(), backup: backup.ok() };
    if pair.primary.is_none() && pair.backup.is_none() { return Err(GptError::NoValidHeader); }

    if let (Some(p), Some(b)) = (pair.primary, pair.backup) {
        if p.current_lba != b.backup_lba || p.backup_lba != b.current_lba
            || p.disk_guid != b.disk_guid
            || p.first_usable_lba != b.first_usable_lba
            || p.last_usable_lba != b.last_usable_lba
            || p.entry_count != b.entry_count || p.entry_size != b.entry_size
            || p.entries_crc32 != b.entries_crc32 {
            return Err(GptError::HeaderPairMismatch);
        }
    }
    Ok(pair)
}

fn read_header_at<D: super::block::BlockDevice>(
    device: &mut D, geo: BlockGeometry, lba: u64, scratch: &mut [u8],
) -> Result<GptHeader, GptError> {
    use super::block::{BlockRange, validate_buffer};
    let range = BlockRange::new(lba, 1);
    validate_buffer(geo, range, scratch.len()).map_err(|_| GptError::InvalidLba)?;
    device.read_blocks(range, scratch).map_err(|_| GptError::DeviceReadFailed)?;
    GptHeader::parse(scratch, lba, geo)
}

/// Enumerate from the primary table first, then retry using the backup table
/// if the primary copy is unreadable, corrupt, or structurally invalid.
pub fn enumerate_gpt_partitions_redundant<D: super::block::BlockDevice>(
    device: &mut D,
    pair: GptHeaderPair,
    block_buffer: &mut [u8],
    entry_buffer: &mut [u8],
    output: &mut [Option<GptPartition>],
) -> Result<usize, GptError> {
    if let Some(primary) = pair.primary {
        if let Ok(count) = enumerate_gpt_partitions(device, primary, block_buffer, entry_buffer, output) {
            return Ok(count);
        }
    }
    if let Some(backup) = pair.backup {
        return enumerate_gpt_partitions(device, backup, block_buffer, entry_buffer, output);
    }
    Err(GptError::NoValidHeader)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GptPartition {
    pub type_guid: [u8; 16],
    pub unique_guid: [u8; 16],
    pub first_lba: u64,
    pub last_lba: u64,
    pub attributes: u64,
    pub name: [u16; 36],
}

/// Verify the entry-array CRC and enumerate allocated entries using caller buffers.
pub fn enumerate_gpt_partitions<D: super::block::BlockDevice>(
    device: &mut D,
    header: GptHeader,
    block_buffer: &mut [u8],
    entry_buffer: &mut [u8],
    output: &mut [Option<GptPartition>],
) -> Result<usize, GptError> {
    use super::block::{BlockRange, validate_buffer};
    let geo = device.geometry().validate().map_err(|_| GptError::InvalidBlockGeometry)?;
    if block_buffer.len() != geo.block_size as usize || entry_buffer.len() != header.entry_size as usize
        || header.entry_size < 128 || header.entry_size % 8 != 0 || header.entry_count == 0 {
        return Err(GptError::BufferTooSmall);
    }
    if header.current_lba >= geo.block_count || header.backup_lba >= geo.block_count
        || header.current_lba == header.backup_lba
        || header.first_usable_lba > header.last_usable_lba
        || header.last_usable_lba >= geo.block_count {
        return Err(GptError::InvalidLba);
    }
    let bytes = (header.entry_count as u64).checked_mul(header.entry_size as u64).ok_or(GptError::AddressOverflow)?;
    let block_size = geo.block_size as u64;
    let table_blocks = bytes.checked_add(block_size - 1).ok_or(GptError::AddressOverflow)? / block_size;
    let table_end = header.entries_lba.checked_add(table_blocks).ok_or(GptError::AddressOverflow)?;
    if header.entries_lba == 0 || table_end > geo.block_count
        || (header.entries_lba <= header.current_lba && header.current_lba < table_end)
        || (header.entries_lba <= header.backup_lba && header.backup_lba < table_end)
        || (header.entries_lba <= header.last_usable_lba && header.first_usable_lba < table_end) {
        return Err(GptError::TableOutOfBounds);
    }

    output.fill(None);
    let mut crc = !0u32;
    let mut offset = 0u64;
    while offset < bytes {
        let block_index = offset / geo.block_size as u64;
        let within = (offset % geo.block_size as u64) as usize;
        let n = core::cmp::min(block_buffer.len() - within, (bytes - offset) as usize);
        let lba = header.entries_lba.checked_add(block_index).ok_or(GptError::AddressOverflow)?;
        let range = BlockRange::new(lba, 1);
        validate_buffer(geo, range, block_buffer.len()).map_err(|_| GptError::TableOutOfBounds)?;
        device.read_blocks(range, block_buffer).map_err(|_| GptError::DeviceReadFailed)?;
        crc = crc32_update(crc, &block_buffer[within..within + n]);
        offset += n as u64;
    }
    if !crc != header.entries_crc32 { return Err(GptError::EntryArrayCrcMismatch); }

    // Second sequential pass: decode entries while reading each table block once.
    // This avoids one device read per GPT entry, including entries crossing block edges.
    let mut count = 0usize;
    let mut entry_fill = 0usize;
    let mut table_offset = 0u64;
    while table_offset < bytes {
        let block_index = table_offset / block_size;
        let within = (table_offset % block_size) as usize;
        let n = core::cmp::min(block_buffer.len() - within, (bytes - table_offset) as usize);
        let lba = header.entries_lba.checked_add(block_index).ok_or(GptError::AddressOverflow)?;
        let range = BlockRange::new(lba, 1);
        validate_buffer(geo, range, block_buffer.len()).map_err(|_| GptError::TableOutOfBounds)?;
        device.read_blocks(range, block_buffer).map_err(|_| GptError::DeviceReadFailed)?;
        let mut consumed = 0usize;
        while consumed < n {
            let take = core::cmp::min(header.entry_size as usize - entry_fill, n - consumed);
            entry_buffer[entry_fill..entry_fill + take]
                .copy_from_slice(&block_buffer[within + consumed..within + consumed + take]);
            entry_fill += take;
            consumed += take;
            if entry_fill == header.entry_size as usize {
                decode_gpt_entry(entry_buffer, header, output, &mut count)?;
                entry_fill = 0;
            }
        }
        table_offset += n as u64;
    }
    if entry_fill != 0 { return Err(GptError::InvalidEntryGeometry); }

fn decode_gpt_entry(
    entry: &[u8], header: GptHeader, output: &mut [Option<GptPartition>], count: &mut usize,
) -> Result<(), GptError> {
    let mut type_guid = [0u8; 16];
    type_guid.copy_from_slice(&entry[..16]);
    if type_guid.iter().all(|b| *b == 0) { return Ok(()); }
    let mut unique_guid = [0u8; 16];
    unique_guid.copy_from_slice(&entry[16..32]);
    let first_lba = u64_at(entry, 32)?;
    let last_lba = u64_at(entry, 40)?;
    if unique_guid.iter().all(|b| *b == 0) || first_lba > last_lba
        || first_lba < header.first_usable_lba || last_lba > header.last_usable_lba {
        return Err(GptError::InvalidPartitionEntry);
    }
    let mut name = [0u16; 36];
    for (j, unit) in name.iter_mut().enumerate() {
        *unit = u16::from_le_bytes([entry[56 + j * 2], entry[57 + j * 2]]);
    }
    for previous in output[..*count].iter().flatten() {
        if first_lba <= previous.last_lba && previous.first_lba <= last_lba {
            return Err(GptError::OverlappingPartitions);
        }
        if unique_guid == previous.unique_guid { return Err(GptError::DuplicatePartitionGuid); }
    }
    if *count == output.len() { return Err(GptError::PartitionOutputFull); }
    output[*count] = Some(GptPartition {
        type_guid, unique_guid, first_lba, last_lba,
        attributes: u64_at(entry, 48)?, name,
    });
    *count += 1;
    Ok(())
}

fn read_entry_bytes<D: super::block::BlockDevice>(
    device: &mut D, geo: BlockGeometry, table_lba: u64, offset: u64,
    dst: &mut [u8], scratch: &mut [u8],
) -> Result<(), GptError> {
    use super::block::{BlockRange, validate_buffer};
    let mut copied = 0usize;
    while copied < dst.len() {
        let pos = offset.checked_add(copied as u64).ok_or(GptError::AddressOverflow)?;
        let block = pos / geo.block_size as u64;
        let within = (pos % geo.block_size as u64) as usize;
        let n = core::cmp::min(scratch.len() - within, dst.len() - copied);
        let range = BlockRange::new(table_lba.checked_add(block).ok_or(GptError::AddressOverflow)?, 1);
        validate_buffer(geo, range, scratch.len()).map_err(|_| GptError::TableOutOfBounds)?;
        device.read_blocks(range, scratch).map_err(|_| GptError::DeviceReadFailed)?;
        dst[copied..copied+n].copy_from_slice(&scratch[within..within+n]);
        copied += n;
    }
    Ok(())
}

fn crc32_update(mut crc: u32, data: &[u8]) -> u32 {
    for byte in data {
        crc ^= *byte as u32;
        for _ in 0..8 { crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1)); }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::block::{BlockDevice, BlockError, BlockRange};
    #[test]
    fn rejects_signature() {
        let b = [0u8; 512];
        assert_eq!(GptHeader::parse(&b, 1, BlockGeometry { block_count: 4096, block_size: 512 }), Err(GptError::InvalidSignature));
    }
    #[test]
    fn crc32_standard_vector() {
        assert_eq!(crc32_zeroed(b"123456789", 9..9), 0xcbf4_3926);
        assert_eq!(!crc32_update(!0, b"123456789"), 0xcbf4_3926);
    }
    struct Disk { table: [u8; 512] }
    impl BlockDevice for Disk {
        fn geometry(&self) -> BlockGeometry { BlockGeometry { block_count: 64, block_size: 512 } }
        fn read_blocks(&mut self, range: BlockRange, buffer: &mut [u8]) -> Result<(), BlockError> {
            if range.start_block != 2 || range.block_count != 1 || buffer.len() != 512 { return Err(BlockError::OutOfBounds); }
            buffer.copy_from_slice(&self.table);
            Ok(())
        }
        fn write_blocks(&mut self, _: BlockRange, _: &[u8]) -> Result<(), BlockError> { Err(BlockError::UnsupportedOperation) }
        fn flush(&mut self) -> Result<(), BlockError> { Ok(()) }
    }
    #[test]
    fn validates_crc_and_enumerates_entries() {
        let mut table = [0u8; 512];
        table[0..16].copy_from_slice(&[1; 16]);
        table[16..32].copy_from_slice(&[2; 16]);
        table[32..40].copy_from_slice(&10u64.to_le_bytes());
        table[40..48].copy_from_slice(&19u64.to_le_bytes());
        table[48..56].copy_from_slice(&7u64.to_le_bytes());
        table[56..60].copy_from_slice(&[b'T' as u8, 0, b'E' as u8, 0]);
        let crc = !crc32_update(!0, &table[..256]);
        let mut disk = Disk { table };
        let header = GptHeader {
            revision: 0x0001_0000, header_size: 92, current_lba: 1, backup_lba: 63,
            first_usable_lba: 4, last_usable_lba: 60, disk_guid: [3; 16],
            entries_lba: 2, entry_count: 2, entry_size: 128, entries_crc32: crc,
        };
        let mut block = [0u8; 512];
        let mut entry = [0u8; 128];
        let mut output = [None; 2];
        let count = enumerate_gpt_partitions(&mut disk, header, &mut block, &mut entry, &mut output).unwrap();
        assert_eq!(count, 1);
        let partition = output[0].unwrap();
        assert_eq!(partition.first_lba, 10);
        assert_eq!(partition.last_lba, 19);
        assert_eq!(partition.attributes, 7);
    }
}
