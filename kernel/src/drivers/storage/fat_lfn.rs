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
