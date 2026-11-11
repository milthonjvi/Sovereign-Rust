//! Multiple APIC Description Table (MADT) parsing.

use super::{AcpiError, sdt::Sdt};

pub const MADT_SIGNATURE: [u8; 4] = *b"APIC";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Madt<'a> {
    pub local_apic_address: u32,
    pub flags: u32,
    records: &'a [u8],
}

impl<'a> Madt<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, AcpiError> {
        let table = Sdt::parse(bytes)?;
        if table.signature() != MADT_SIGNATURE {
            return Err(AcpiError::InvalidSignature);
        }
        let payload = table.payload();
        if payload.len() < 8 {
            return Err(AcpiError::BufferTooSmall);
        }
        Ok(Self {
            local_apic_address: read_u32(payload, 0)?,
            flags: read_u32(payload, 4)?,
            records: &payload[8..],
        })
    }

    pub fn records(&self) -> MadtRecords<'a> {
        MadtRecords { bytes: self.records, offset: 0 }
    }

    pub fn legacy_pic_compatible(&self) -> bool {
        self.flags & 1 != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MadtRecord<'a> {
    pub record_type: u8,
    pub data: &'a [u8],
}

impl MadtRecord<'_> {
    pub fn local_apic(&self) -> Option<LocalApic> {
        if self.record_type != 0 || self.data.len() < 6 { return None; }
        Some(LocalApic {
            processor_uid: self.data[0],
            apic_id: self.data[1],
            flags: u32::from_le_bytes([self.data[2], self.data[3], self.data[4], self.data[5]]),
        })
    }

    pub fn io_apic(&self) -> Option<IoApic> {
        if self.record_type != 1 || self.data.len() < 10 { return None; }
        Some(IoApic {
            id: self.data[0],
            address: u32::from_le_bytes([self.data[2], self.data[3], self.data[4], self.data[5]]),
            global_system_interrupt_base: u32::from_le_bytes([
                self.data[6], self.data[7], self.data[8], self.data[9],
            ]),
        })
    }

    pub fn interrupt_source_override(&self) -> Option<InterruptSourceOverride> {
        if self.record_type != 2 || self.data.len() < 8 { return None; }
        Some(InterruptSourceOverride {
            bus: self.data[0],
            source: self.data[1],
            global_system_interrupt: u32::from_le_bytes([
                self.data[2], self.data[3], self.data[4], self.data[5],
            ]),
            flags: u16::from_le_bytes([self.data[6], self.data[7]]),
        })
    }

    pub fn local_x2apic(&self) -> Option<LocalX2Apic> {
        if self.record_type != 9 || self.data.len() < 14 { return None; }
        Some(LocalX2Apic {
            x2apic_id: u32::from_le_bytes([self.data[4], self.data[5], self.data[6], self.data[7]]),
            flags: u32::from_le_bytes([self.data[8], self.data[9], self.data[10], self.data[11]]),
            processor_uid: u32::from_le_bytes([self.data[12], self.data[13], self.data[14], self.data[15]]),
        })
    }
}

pub struct MadtRecords<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Iterator for MadtRecords<'a> {
    type Item = Result<MadtRecord<'a>, AcpiError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset == self.bytes.len() { return None; }
        if self.bytes.len().saturating_sub(self.offset) < 2 {
            self.offset = self.bytes.len();
            return Some(Err(AcpiError::MalformedTable));
        }
        let kind = self.bytes[self.offset];
        let length = self.bytes[self.offset + 1] as usize;
        if length < 2 || length > self.bytes.len() - self.offset {
            self.offset = self.bytes.len();
            return Some(Err(AcpiError::MalformedTable));
        }
        let start = self.offset + 2;
        let end = self.offset + length;
        self.offset = end;
        Some(Ok(MadtRecord { record_type: kind, data: &self.bytes[start..end] }))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LocalApic { pub processor_uid: u8, pub apic_id: u8, pub flags: u32 }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IoApic {
    pub id: u8,
    pub address: u32,
    pub global_system_interrupt_base: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InterruptSourceOverride {
    pub bus: u8,
    pub source: u8,
    pub global_system_interrupt: u32,
    pub flags: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LocalX2Apic {
    pub x2apic_id: u32,
    pub flags: u32,
    pub processor_uid: u32,
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, AcpiError> {
    let data = bytes.get(offset..offset + 4).ok_or(AcpiError::BufferTooSmall)?;
    Ok(u32::from_le_bytes([data[0], data[1], data[2], data[3]]))
}
