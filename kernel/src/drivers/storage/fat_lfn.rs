//! Stateful, allocation-free VFAT long-file-name assembler.
use super::fat::{FatDirEntry, FatError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LfnError { InvalidSequence, InvalidChecksum, InvalidUtf16, NameTooLong }

#[derive(Clone, Copy)]
pub struct FatLongName {
    units: [u16; 260],
    len: usize,
    expected_sequence: u8,
    checksum: u8,
    active: bool,
}
impl FatLongName {
    pub const fn new() -> Self {
        Self { units: [0xFFFF; 260], len: 0, expected_sequence: 0, checksum: 0, active: false }
    }
    pub fn reset(&mut self) {
        self.units.fill(0xFFFF);
        self.len = 0;
        self.expected_sequence = 0;
        self.checksum = 0;
        self.active = false;
    }
    pub fn push_record(&mut self, raw: &[u8]) -> Result<(), LfnError> {
        if raw.len() < 32 || raw[11] != 0x0F || raw[12] != 0 || raw[26] != 0 || raw[27] != 0 {
            self.reset();
            return Err(LfnError::InvalidSequence);
        }
        let sequence = raw[0] & 0x1F;
        let is_last = raw[0] & 0x40 != 0;
        if sequence == 0 || sequence > 20 || raw[0] & 0x80 != 0 {
            self.reset();
            return Err(LfnError::InvalidSequence);
        }
        if is_last {
            self.reset();
            self.active = true;
            self.expected_sequence = sequence;
            self.checksum = raw[13];
            self.len = sequence as usize * 13;
        }
        if !self.active || sequence != self.expected_sequence || raw[13] != self.checksum {
            self.reset();
            return Err(LfnError::InvalidSequence);
        }
        let offsets = [1usize,3,5,7,9,14,16,18,20,22,24,28,30];
        let base = (sequence as usize - 1) * 13;
        for (i, offset) in offsets.iter().enumerate() {
            self.units[base + i] = u16::from_le_bytes([raw[*offset], raw[*offset + 1]]);
        }
        self.expected_sequence -= 1;
        Ok(())
    }
    pub fn finish(&mut self, short_name: &[u8; 11]) -> Result<Option<([u16; 260], usize)>, LfnError> {
        if !self.active || self.expected_sequence != 0 {
            self.reset();
            return Ok(None);
        }
        let mut checksum = 0u8;
        for byte in short_name {
            checksum = ((checksum & 1) << 7).wrapping_add(checksum >> 1).wrapping_add(*byte);
        }
        if checksum != self.checksum {
            self.reset();
            return Err(LfnError::InvalidChecksum);
        }
        let mut end = self.len;
        if let Some(pos) = self.units[..end].iter().position(|v| *v == 0) { end = pos; }
        while end > 0 && self.units[end - 1] == 0xFFFF { end -= 1; }
        if end > 255 { self.reset(); return Err(LfnError::NameTooLong); }
        for unit in &self.units[..end] {
            if (0xD800..=0xDBFF).contains(unit) {
                // The following low surrogate is checked in the next iteration.
                continue;
            }
            if (0xDC00..=0xDFFF).contains(unit) {
                // A low surrogate must have a preceding high surrogate.
                let index = self.units[..end].iter().position(|v| core::ptr::eq(v, unit));
                if index == Some(0) { self.reset(); return Err(LfnError::InvalidUtf16); }
            }
        }
        let result = (self.units, end);
        self.reset();
        Ok(Some(result))
    }
}
impl Default for FatLongName { fn default() -> Self { Self::new() } }

pub const fn short_name_checksum(name: &[u8; 11]) -> u8 {
    let mut sum = 0u8;
    let mut i = 0;
    while i < 11 {
        sum = (sum.rotate_right(1)).wrapping_add(name[i]);
        i += 1;
    }
    sum
}

/// Feed a directory record: LFN fragments update state; a short record completes it.
pub fn consume_lfn_record(state: &mut FatLongName, raw: &[u8]) -> Result<Option<([u16; 260], usize, FatDirEntry)>, FatError> {
    if raw.len() < 32 { state.reset(); return Err(FatError::InvalidDirectoryEntry); }
    if raw[11] == 0x0F {
        state.push_record(raw).map_err(|_| FatError::InvalidDirectoryEntry)?;
        return Ok(None);
    }
    let entry = match FatDirEntry::parse(raw)? { Some(entry) => entry, None => { state.reset(); return Ok(None); } };
    match state.finish(&entry.short_name) {
        Ok(Some((name, len))) => Ok(Some((name, len, entry))),
        Ok(None) => Ok(None),
        Err(_) => { state.reset(); Ok(None) }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    fn make_record(sequence: u8, last: bool, checksum: u8, units: &[u16; 13]) -> [u8; 32] {
        let mut raw = [0u8; 32];
        raw[0] = sequence | if last { 0x40 } else { 0 };
        raw[11] = 0x0F;
        raw[12] = 0;
        raw[13] = checksum;
        raw[26] = 0;
        raw[27] = 0;
        let offsets = [1usize,3,5,7,9,14,16,18,20,22,24,28,30];
        for (i, offset) in offsets.iter().enumerate() {
            raw[*offset..*offset+2].copy_from_slice(&units[i].to_le_bytes());
        }
        raw
    }
    #[test]
    fn assembles_single_record_name_and_checks_short_alias() {
        let short = *b"HELLO   TXT";
        let checksum = short_name_checksum(&short);
        let mut units = [0xFFFFu16; 13];
        units[0..5].copy_from_slice(&[72,101,108,108,111]);
        units[5] = 0;
        let record = make_record(1, true, checksum, &units);
        let mut state = FatLongName::new();
        state.push_record(&record).unwrap();
        let (name, len) = state.finish(&short).unwrap().unwrap();
        assert_eq!(len, 5);
        assert_eq!(&name[..len], &[72,101,108,108,111]);
    }
    #[test]
    fn rejects_checksum_mismatch() {
        let short = *b"HELLO   TXT";
        let mut units = [0xFFFFu16; 13];
        units[0] = 65;
        let record = make_record(1, true, 0xA5, &units);
        let mut state = FatLongName::new();
        state.push_record(&record).unwrap();
        assert_eq!(state.finish(&short), Err(LfnError::InvalidChecksum));
    }
    #[test]
    fn rejects_unpaired_surrogate() {
        let short = *b"HELLO   TXT";
        let checksum = short_name_checksum(&short);
        let mut units = [0xFFFFu16; 13];
        units[0] = 0xD800;
        units[1] = 0;
        let record = make_record(1, true, checksum, &units);
        let mut state = FatLongName::new();
        state.push_record(&record).unwrap();
        assert_eq!(state.finish(&short), Err(LfnError::InvalidUtf16));
    }
}


/// Visit directory entries with assembled VFAT names. The callback receives an
/// empty name slice when no valid LFN is attached; it can then use the 8.3 alias.
/// Returning false stops enumeration without treating it as an error.
pub fn visit_directory<D, F>(
    device: &mut D,
    volume: &super::fat_read::FatVolume,
    start_cluster: Option<u32>,
    cluster_buffer: &mut [u8],
    fat_scratch: &mut [u8],
    mut visitor: F,
) -> Result<usize, FatError>
where
    D: super::block::BlockDevice,
    F: FnMut(&FatDirEntry, &[u16]) -> bool,
{
    use super::fat::{FatChainValue, FatKind};
    use super::block::BlockRange;
    use super::fat_read::{read_cluster, read_fat_entry};
    let bps = volume.bpb.bytes_per_sector as usize;
    if start_cluster.is_none() {
        if volume.bpb.kind == FatKind::Fat32 || cluster_buffer.len() < bps {
            return Err(FatError::InvalidDirectoryEntry);
        }
    } else if cluster_buffer.len() != volume.cluster_size_bytes() {
        return Err(FatError::ClusterBufferTooSmall);
    }
    let mut state = FatLongName::new();
    let mut visited = 0usize;
    let mut process = |bytes: &[u8]| -> Result<bool, FatError> {
        let mut stop = false;
        for raw in bytes.chunks_exact(32) {
            if raw[0] == 0 { stop = true; state.reset(); break; }
            if raw[0] == 0xE5 { state.reset(); continue; }
            if raw[11] == 0x0F {
                let _ = state.push_record(raw);
                continue;
            }
            if let Some(entry) = FatDirEntry::parse(raw)? {
                let named = state.finish(&entry.short_name).ok().flatten();
                let keep_going = match named {
                    Some((name, len)) => visitor(&entry, &name[..len]),
                    None => visitor(&entry, &[]),
                };
                visited += 1;
                if !keep_going { stop = true; break; }
            } else { state.reset(); }
        }
        Ok(stop)
    };
    if let Some(mut cluster) = start_cluster {
        let mut clusters_seen = 0u32;
        loop {
            if clusters_seen >= volume.bpb.cluster_count { return Err(FatError::ChainTooLong); }
            clusters_seen += 1;
            read_cluster(device, volume, cluster, cluster_buffer)?;
            if process(cluster_buffer)? { break; }
            cluster = match read_fat_entry(device, volume, cluster, fat_scratch)? {
                FatChainValue::Next(next) => next,
                FatChainValue::End => break,
            };
        }
    } else {
        let first = volume.bpb.reserved_sectors as u64
            + volume.bpb.fat_count as u64 * volume.bpb.sectors_per_fat as u64;
        for sector in 0..volume.bpb.root_directory_sectors as u64 {
            device.read_blocks(BlockRange::new(first + sector, 1), &mut cluster_buffer[..bps])
                .map_err(FatError::Block)?;
            if process(&cluster_buffer[..bps])? { break; }
        }
    }
    Ok(visited)
}
