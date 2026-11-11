//! Firmware ACPI Control Structure (FACS).
//!
//! Unlike an SDT, FACS has no checksum field and must not be parsed with
//! Sdt::parse. The caller is responsible for mapping the complete physical
//! range before passing the borrowed bytes to this parser.

use super::AcpiError;

pub const FACS_SIGNATURE: [u8; 4] = *b"FACS";
pub const FACS_MIN_LENGTH: usize = 64;
pub const FACS_ALIGNMENT: u64 = 64;
const FLAG_S4_BIOS: u32 = 1 << 0;
const FLAG_64BIT_WAKE: u32 = 1 << 1;

/// Validated, read-only FACS metadata.
///
/// Firmware-owned state is exposed as values. Mutating the global lock or
/// waking vectors requires a separately synchronized architecture layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Facs {
    pub physical_address: u64,
    pub length: u32,
    pub hardware_signature: u32,
    pub firmware_waking_vector: u32,
    pub global_lock: u32,
    pub flags: u32,
    pub x_firmware_waking_vector: u64,
    pub version: u8,
    pub ospm_flags: u32,
}

impl Facs {
    /// Parse a FACS at its physical address. FACS has no SDT checksum.
    pub fn parse(physical_address: u64, bytes: &[u8]) -> Result<Self, AcpiError> {
        if physical_address == 0 {
            return Err(AcpiError::UnsupportedAddress);
        }
        if physical_address % FACS_ALIGNMENT != 0 {
            return Err(AcpiError::MisalignedTable);
        }
        if bytes.len() < 8 {
            return Err(AcpiError::BufferTooSmall);
        }
        if bytes.get(0..4) != Some(&FACS_SIGNATURE) {
            return Err(AcpiError::InvalidSignature);
        }

        let length = read_u32(bytes, 4)?;
        let length_usize = usize::try_from(length).map_err(|_| AcpiError::InvalidLength)?;
        if length_usize < FACS_MIN_LENGTH || length_usize > bytes.len() {
            return Err(AcpiError::InvalidLength);
        }

        let data = &bytes[..length_usize];
        Ok(Self {
            physical_address,
            length,
            hardware_signature: read_u32(data, 8)?,
            firmware_waking_vector: read_u32(data, 12)?,
            global_lock: read_u32(data, 16)?,
            flags: read_u32(data, 20)?,
            x_firmware_waking_vector: read_u64(data, 24)?,
            version: data[32],
            ospm_flags: read_u32(data, 36)?,
        })
    }

    pub const fn supports_s4_bios(&self) -> bool {
        self.flags & FLAG_S4_BIOS != 0
    }

    pub const fn supports_64bit_wake(&self) -> bool {
        self.flags & FLAG_64BIT_WAKE != 0
    }

    pub const fn has_64bit_waking_vector(&self) -> bool {
        self.x_firmware_waking_vector != 0
    }
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, AcpiError> {
    let data = bytes.get(offset..offset + 4).ok_or(AcpiError::BufferTooSmall)?;
    Ok(u32::from_le_bytes([data[0], data[1], data[2], data[3]]))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, AcpiError> {
    let data = bytes.get(offset..offset + 8).ok_or(AcpiError::BufferTooSmall)?;
    Ok(u64::from_le_bytes([
        data[0], data[1], data[2], data[3],
        data[4], data[5], data[6], data[7],
    ]))
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> std::vec::Vec<u8> {
        let mut bytes = std::vec![0u8; FACS_MIN_LENGTH];
        bytes[0..4].copy_from_slice(b"FACS");
        bytes[4..8].copy_from_slice(&(FACS_MIN_LENGTH as u32).to_le_bytes());
        bytes[8..12].copy_from_slice(&0x1234_5678u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&0x1000u32.to_le_bytes());
        bytes[16..20].copy_from_slice(&0xA5A5u32.to_le_bytes());
        bytes[20..24].copy_from_slice(&(FLAG_S4_BIOS | FLAG_64BIT_WAKE).to_le_bytes());
        bytes[24..32].copy_from_slice(&0x1_0000_1000u64.to_le_bytes());
        bytes[32] = 2;
        bytes[36..40].copy_from_slice(&1u32.to_le_bytes());
        bytes
    }

    #[test]
    fn parses_facs_without_sdt_checksum() {
        let mut bytes = fixture();
        bytes[9] = 0xFF;
        let facs = Facs::parse(0x4000, &bytes).unwrap();
        assert_eq!(facs.hardware_signature, 0x1234_5678);
        assert_eq!(facs.firmware_waking_vector, 0x1000);
        assert_eq!(facs.x_firmware_waking_vector, 0x1_0000_1000);
        assert!(facs.supports_s4_bios());
        assert!(facs.supports_64bit_wake());
        assert!(facs.has_64bit_waking_vector());
    }

    #[test]
    fn rejects_bad_signature_alignment_and_length() {
        let bytes = fixture();
        assert_eq!(Facs::parse(0x4001, &bytes), Err(AcpiError::MisalignedTable));
        assert_eq!(Facs::parse(0, &bytes), Err(AcpiError::UnsupportedAddress));
        let mut bad_signature = bytes.clone();
        bad_signature[0..4].copy_from_slice(b"BAD!");
        assert_eq!(Facs::parse(0x4000, &bad_signature), Err(AcpiError::InvalidSignature));
        let mut short = bytes;
        short[4..8].copy_from_slice(&32u32.to_le_bytes());
        assert_eq!(Facs::parse(0x4000, &short), Err(AcpiError::InvalidLength));
    }

    #[test]
    fn rejects_truncated_mapping() {
        let bytes = fixture();
        assert_eq!(Facs::parse(0x4000, &bytes[..48]), Err(AcpiError::InvalidLength));
    }
}
