//! Allocation-free validation of GUID Partition Table headers.
use super::block::BlockGeometry;
const SIGNATURE: &[u8; 8] = b"EFI PART";
const MIN_HEADER: usize = 92;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GptError { BufferTooSmall, InvalidSignature, UnsupportedRevision, InvalidHeaderSize, HeaderCrcMismatch, InvalidLba, InvalidUsableRange, InvalidEntryGeometry, AddressOverflow, TableOutOfBounds, InvalidBlockGeometry, EntryArrayCrcMismatch, InvalidPartitionEntry, OverlappingPartitions, DuplicatePartitionGuid, PartitionOutputFull, DeviceReadFailed }

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
    let bytes = (header.entry_count as u64).checked_mul(header.entry_size as u64).ok_or(GptError::AddressOverflow)?;
    let end_byte = header.entries_lba.checked_mul(geo.block_size as u64)
        .and_then(|v| v.checked_add(bytes)).ok_or(GptError::AddressOverflow)?;
    let disk_bytes = geo.block_count.checked_mul(geo.block_size as u64).ok_or(GptError::AddressOverflow)?;
    if end_byte > disk_bytes { return Err(GptError::TableOutOfBounds); }

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

    let mut count = 0usize;
    for i in 0..header.entry_count as u64 {
        read_entry_bytes(device, geo, header.entries_lba,
            i.checked_mul(header.entry_size as u64).ok_or(GptError::AddressOverflow)?,
            entry_buffer, block_buffer)?;
        let mut type_guid = [0u8; 16];
        type_guid.copy_from_slice(&entry_buffer[..16]);
        if type_guid.iter().all(|b| *b == 0) { continue; }
        let mut unique_guid = [0u8; 16];
        unique_guid.copy_from_slice(&entry_buffer[16..32]);
        let first_lba = u64_at(entry_buffer, 32)?;
        let last_lba = u64_at(entry_buffer, 40)?;
        if unique_guid.iter().all(|b| *b == 0) || first_lba > last_lba
            || first_lba < header.first_usable_lba || last_lba > header.last_usable_lba {
            return Err(GptError::InvalidPartitionEntry);
        }
        let mut name = [0u16; 36];
        for (j, unit) in name.iter_mut().enumerate() {
            *unit = u16::from_le_bytes([entry_buffer[56 + j * 2], entry_buffer[57 + j * 2]]);
        }
        for previous in output[..count].iter().flatten() {
            if first_lba <= previous.last_lba && previous.first_lba <= last_lba {
                return Err(GptError::OverlappingPartitions);
            }
            if unique_guid == previous.unique_guid { return Err(GptError::DuplicatePartitionGuid); }
        }
        if count == output.len() { return Err(GptError::PartitionOutputFull); }
        output[count] = Some(GptPartition {
            type_guid, unique_guid, first_lba, last_lba,
            attributes: u64_at(entry_buffer, 48)?, name,
        });
        count += 1;
    }
    Ok(count)
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

#[cfg(test)] mod tests { use super::*; #[test] fn rejects_signature(){let b=[0u8;512];assert_eq!(GptHeader::parse(&b,1,BlockGeometry{block_count:4096,block_size:512}),Err(GptError::InvalidSignature));} #[test] fn crc32_standard_vector(){assert_eq!(crc32_zeroed(b"123456789",9..9),0xcbf4_3926);} }
