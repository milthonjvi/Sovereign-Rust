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
        if sequence == 0 || sequence > 20 || raw[0] & 0xA0 != 0 {
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
        if let Some(pos) = self.units[..end].iter().position(|v| *v == 0) {
            if self.units[pos + 1..end].iter().any(|v| *v != 0xFFFF) {
                self.reset();
                return Err(LfnError::InvalidSequence);
            }
            end = pos;
        } else if self.units[..end].iter().any(|v| *v == 0xFFFF) {
            self.reset();
            return Err(LfnError::InvalidSequence);
        }
        if end == 0 { self.reset(); return Err(LfnError::InvalidSequence); }
        if end > 255 { self.reset(); return Err(LfnError::NameTooLong); }
        if self.units[end - 1] == b' ' as u16 || self.units[end - 1] == b'.' as u16 {
            self.reset();
            return Err(LfnError::InvalidSequence);
        }
        let mut check = 0usize;
        while check < end {
            let unit = self.units[check];
            if unit < 0x20 || matches!(unit, 0x22 | 0x2A | 0x2F | 0x3A | 0x3C | 0x3E | 0x3F | 0x5C | 0x7C) {
                self.reset();
                return Err(LfnError::InvalidSequence);
            }
            check += 1;
        }
        let mut index = 0usize;
        while index < end {
            let unit = self.units[index];
            if (0xD800..=0xDBFF).contains(&unit) {
                if index + 1 >= end || !(0xDC00..=0xDFFF).contains(&self.units[index + 1]) {
                    self.reset();
                    return Err(LfnError::InvalidUtf16);
                }
                index += 2;
                continue;
            }
            if (0xDC00..=0xDFFF).contains(&unit) {
                self.reset();
                return Err(LfnError::InvalidUtf16);
            }
            index += 1;
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
    consume_lfn_record_for_kind(state, raw, super::fat::FatKind::Fat32)
}
pub fn consume_lfn_record_for_kind(state: &mut FatLongName, raw: &[u8], kind: super::fat::FatKind) -> Result<Option<([u16; 260], usize, FatDirEntry)>, FatError> {
    if raw.len() < 32 { state.reset(); return Err(FatError::InvalidDirectoryEntry); }
    if raw[11] == 0x0F {
        state.push_record(raw).map_err(|_| FatError::InvalidDirectoryEntry)?;
        return Ok(None);
    }
    let entry = match FatDirEntry::parse_for_kind(raw, kind)? { Some(entry) => entry, None => { state.reset(); return Ok(None); } };
    match state.finish(&entry.short_name) {
        Ok(Some((name, len))) => Ok(Some((name, len, entry))),
        Ok(None) | Err(_) => {
            state.reset();
            Ok(Some(([0u16; 260], 0, entry)))
        }
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
            if let Some(entry) = FatDirEntry::parse_for_kind(raw, volume.bpb.kind)? {
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


fn ascii_fold(unit: u16) -> u16 {
    if (b'a' as u16..=b'z' as u16).contains(&unit) { unit - 32 } else { unit }
}
fn long_name_matches(name: &[u16], query: &[u16]) -> bool {
    name.len() == query.len() && name.iter().zip(query).all(|(a,b)| ascii_fold(*a) == ascii_fold(*b))
}
fn short_name_matches(alias: &[u8; 11], query: &[u16]) -> bool {
    let base_len = alias[..8].iter().rposition(|b| *b != b' ').map_or(0, |i| i + 1);
    let ext_len = alias[8..].iter().rposition(|b| *b != b' ').map_or(0, |i| i + 1);
    let expected_len = base_len + if ext_len != 0 { 1 + ext_len } else { 0 };
    if expected_len != query.len() { return false; }
    let mut out = 0usize;
    for byte in &alias[..base_len] {
        let value = if out == 0 && *byte == 0x05 { 0xE5 } else { *byte };
        if ascii_fold(value as u16) != ascii_fold(query[out]) { return false; }
        out += 1;
    }
    if ext_len != 0 {
        if query[out] != b'.' as u16 { return false; }
        out += 1;
        for byte in &alias[8..8 + ext_len] {
            if ascii_fold(*byte as u16) != ascii_fold(query[out]) { return false; }
            out += 1;
        }
    }
    true
}

/// Resolve a slash- or backslash-separated path using LFN names and 8.3 aliases.
/// The path is UTF-16, matching VFAT on-disk names. ASCII letters are compared
/// case-insensitively; non-ASCII code units are compared exactly. Dot components
/// are rejected rather than interpreted without a parent-directory stack.
pub fn lookup_path<D: super::block::BlockDevice>(
    device: &mut D,
    volume: &super::fat_read::FatVolume,
    path: &[u16],
    cluster_buffer: &mut [u8],
    fat_scratch: &mut [u8],
) -> Result<Option<FatDirEntry>, FatError> {
    use super::fat::{FatKind};
    if path.is_empty() { return Ok(None); }
    let mut directory = if volume.bpb.kind == FatKind::Fat32 { volume.bpb.root_cluster } else { None };
    let mut cursor = 0usize;
    let mut result = None;
    while cursor < path.len() {
        while cursor < path.len() && (path[cursor] == b'/' as u16 || path[cursor] == b'\\' as u16) { cursor += 1; }
        if cursor == path.len() { break; }
        let start = cursor;
        while cursor < path.len() && path[cursor] != b'/' as u16 && path[cursor] != b'\\' as u16 { cursor += 1; }
        let component = &path[start..cursor];
        if component.is_empty() || (component.len() == 1 && component[0] == b'.' as u16)
            || (component.len() == 2 && component[0] == b'.' as u16 && component[1] == b'.' as u16) {
            return Err(FatError::InvalidDirectoryEntry);
        }
        let mut found = None;
        visit_directory(device, volume, directory, cluster_buffer, fat_scratch, |entry, long_name| {
            let matches = if !long_name.is_empty() {
                long_name_matches(long_name, component) || short_name_matches(&entry.short_name, component)
            } else { short_name_matches(&entry.short_name, component) };
            if matches { found = Some(*entry); false } else { true }
        })?;
        let entry = match found { Some(value) => value, None => return Ok(None) };
        result = Some(entry);
        let had_separator = cursor < path.len();
        let mut remaining = cursor;
        while remaining < path.len() && (path[remaining] == b'/' as u16 || path[remaining] == b'\\' as u16) { remaining += 1; }
        if had_separator {
            if !entry.is_directory() || entry.first_cluster < 2 { return Ok(None); }
            if remaining < path.len() { directory = Some(entry.first_cluster); }
        }
    }
    Ok(result)
}
