//! System Description Table (SDT) header and root-table parsing.

use super::{AcpiError, Signature, checksum_valid};

pub const SDT_HEADER_LEN: usize = 36;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SdtHeader {
    pub signature: Signature,
    pub length: u32,
    pub revision: u8,
    pub checksum: u8,
    pub oem_id: [u8; 6],
    pub oem_table_id: [u8; 8],
    pub oem_revision: u32,
    pub creator_id: u32,
    pub creator_revision: u32,
}

impl SdtHeader {
    pub fn parse(bytes: &[u8]) -> Result<Self, AcpiError> {
        if bytes.len() < SDT_HEADER_LEN {
            return Err(AcpiError::BufferTooSmall);
        }
        let length = read_u32(bytes, 4)?;
        let length_usize = usize::try_from(length).map_err(|_| AcpiError::InvalidLength)?;
        if length_usize < SDT_HEADER_LEN || length_usize > bytes.len() {
            return Err(AcpiError::InvalidLength);
        }
        if !checksum_valid(&bytes[..length_usize]) {
            return Err(AcpiError::InvalidChecksum);
        }

        let mut signature = [0; 4];
        signature.copy_from_slice(&bytes[0..4]);
        let mut oem_id = [0; 6];
        oem_id.copy_from_slice(&bytes[10..16]);
        let mut oem_table_id = [0; 8];
        oem_table_id.copy_from_slice(&bytes[16..24]);

        Ok(Self {
            signature,
            length,
            revision: bytes[8],
            checksum: bytes[9],
            oem_id,
            oem_table_id,
            oem_revision: read_u32(bytes, 24)?,
            creator_id: read_u32(bytes, 28)?,
            creator_revision: read_u32(bytes, 32)?,
        })
    }
}

/// Borrowed, checksum-validated SDT. The payload excludes the 36-byte header.
#[derive(Clone, Copy, Debug)]
pub struct Sdt<'a> {
    pub header: SdtHeader,
    bytes: &'a [u8],
}

impl<'a> Sdt<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, AcpiError> {
        let header = SdtHeader::parse(bytes)?;
        let length = header.length as usize;
        Ok(Self { header, bytes: &bytes[..length] })
    }

    pub fn signature(&self) -> Signature {
        self.header.signature
    }

    pub fn payload(&self) -> &'a [u8] {
        &self.bytes[SDT_HEADER_LEN..]
    }

    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }
}

/// Validated RSDT/XSDT entries. The caller supplies the mapped table bytes.
#[derive(Clone, Copy, Debug)]
pub struct RootTable<'a> {
    pub table: Sdt<'a>,
    entry_width: usize,
}

impl<'a> RootTable<'a> {
    pub fn parse_rsdt(bytes: &'a [u8]) -> Result<Self, AcpiError> {
        Self::parse_with_signature(bytes, *b"RSDT", 4)
    }

    pub fn parse_xsdt(bytes: &'a [u8]) -> Result<Self, AcpiError> {
        Self::parse_with_signature(bytes, *b"XSDT", 8)
    }

    fn parse_with_signature(
        bytes: &'a [u8],
        expected: Signature,
        entry_width: usize,
    ) -> Result<Self, AcpiError> {
        let table = Sdt::parse(bytes)?;
        if table.signature() != expected {
            return Err(AcpiError::InvalidSignature);
        }
        if table.payload().len() % entry_width != 0 {
            return Err(AcpiError::MalformedTable);
        }
        Ok(Self { table, entry_width })
    }

    pub fn entries(&self) -> RootEntries<'a> {
        RootEntries { bytes: self.table.payload(), offset: 0, width: self.entry_width }
    }
}

pub struct RootEntries<'a> {
    bytes: &'a [u8],
    offset: usize,
    width: usize,
}

impl Iterator for RootEntries<'_> {
    type Item = u64;

    fn next(&mut self) -> Option<Self::Item> {
        let end = self.offset.checked_add(self.width)?;
        let entry = self.bytes.get(self.offset..end)?;
        self.offset = end;
        Some(if self.width == 4 {
            u32::from_le_bytes([entry[0], entry[1], entry[2], entry[3]]) as u64
        } else {
            u64::from_le_bytes([
                entry[0], entry[1], entry[2], entry[3],
                entry[4], entry[5], entry[6], entry[7],
            ])
        })
    }
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, AcpiError> {
    let data = bytes.get(offset..offset + 4).ok_or(AcpiError::BufferTooSmall)?;
    Ok(u32::from_le_bytes([data[0], data[1], data[2], data[3]]))
}
