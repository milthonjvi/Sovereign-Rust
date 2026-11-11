//! Root System Description Pointer (RSDP) parsing.

use super::{AcpiError, checksum_valid};

pub const RSDP_SIGNATURE: &[u8; 8] = b"RSD PTR ";

/// Validated ACPI root pointer. Addresses remain physical addresses and must
/// be mapped by the kernel before a table parser is called.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rsdp {
    pub revision: u8,
    pub rsdt_address: u32,
    pub xsdt_address: Option<u64>,
    pub length: u32,
}

impl Rsdp {
    /// Parse an RSDP from a contiguous firmware-provided byte range.
    pub fn parse(bytes: &[u8]) -> Result<Self, AcpiError> {
        if bytes.len() < 20 {
            return Err(AcpiError::BufferTooSmall);
        }
        if &bytes[0..8] != RSDP_SIGNATURE {
            return Err(AcpiError::InvalidSignature);
        }
        if !checksum_valid(&bytes[..20]) {
            return Err(AcpiError::InvalidChecksum);
        }

        let revision = bytes[15];
        let rsdt_address = read_u32(bytes, 16)?;
        if revision == 0 {
            return Ok(Self {
                revision,
                rsdt_address,
                xsdt_address: None,
                length: 20,
            });
        }
        if revision < 2 {
            return Err(AcpiError::InvalidRevision);
        }
        if bytes.len() < 36 {
            return Err(AcpiError::BufferTooSmall);
        }

        let length = read_u32(bytes, 20)?;
        let length_usize = usize::try_from(length).map_err(|_| AcpiError::InvalidLength)?;
        if length_usize < 36 || length_usize > bytes.len() {
            return Err(AcpiError::InvalidLength);
        }
        if !checksum_valid(&bytes[..length_usize]) {
            return Err(AcpiError::InvalidChecksum);
        }

        let xsdt = read_u64(bytes, 24)?;
        Ok(Self {
            revision,
            rsdt_address,
            xsdt_address: if xsdt == 0 { None } else { Some(xsdt) },
            length,
        })
    }

    /// Prefer XSDT on ACPI 2.0+, falling back to RSDT when unavailable.
    pub fn root_table_address(&self) -> Result<RootTableAddress, AcpiError> {
        if let Some(address) = self.xsdt_address {
            Ok(RootTableAddress::Xsdt(address))
        } else if self.rsdt_address != 0 {
            Ok(RootTableAddress::Rsdt(self.rsdt_address as u64))
        } else {
            Err(AcpiError::UnsupportedAddress)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RootTableAddress {
    Rsdt(u64),
    Xsdt(u64),
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, AcpiError> {
    let data = bytes.get(offset..offset + 4).ok_or(AcpiError::BufferTooSmall)?;
    Ok(u32::from_le_bytes([data[0], data[1], data[2], data[3]]))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, AcpiError> {
    let data = bytes.get(offset..offset + 8).ok_or(AcpiError::BufferTooSmall)?;
    Ok(u64::from_le_bytes([
        data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
    ]))
}
