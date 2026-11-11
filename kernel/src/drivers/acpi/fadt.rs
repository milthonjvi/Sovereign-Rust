//! Fixed ACPI Description Table (FADT) common fields.

use super::{AcpiError, sdt::Sdt};

pub const FADT_SIGNATURE: [u8; 4] = *b"FACP";

/// Common FADT fields needed for early platform and power-management setup.
/// Register access is deliberately left to architecture-specific code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Fadt {
    pub firmware_ctrl: u32,
    pub dsdt: u32,
    pub preferred_power_management_profile: u8,
    pub sci_interrupt: u16,
    pub smi_command_port: u32,
    pub acpi_enable: u8,
    pub acpi_disable: u8,
    pub pm1a_control_block: u32,
    pub pm1b_control_block: u32,
    pub flags: u32,
    pub reset_register: Option<GenericAddress>,
    pub reset_value: Option<u8>,
    pub x_firmware_ctrl: Option<u64>,
    pub x_dsdt: Option<u64>,
}

impl Fadt {
    /// Parse the common FADT fields from a checksum-validated SDT.
    pub fn parse(bytes: &[u8]) -> Result<Self, AcpiError> {
        let table = Sdt::parse(bytes)?;
        if table.signature() != FADT_SIGNATURE {
            return Err(AcpiError::InvalidSignature);
        }
        let b = table.bytes();
        // Flags end at byte 116 in the ACPI-defined fixed portion.
        if b.len() < 116 {
            return Err(AcpiError::InvalidLength);
        }

        let reset_register = if b.len() >= 128 {
            Some(GenericAddress::parse(&b[116..128])?)
        } else {
            None
        };
        let reset_value = b.get(128).copied();
        let x_firmware_ctrl = if b.len() >= 140 { Some(read_u64(b, 132)?) } else { None };
        let x_dsdt = if b.len() >= 148 { Some(read_u64(b, 140)?) } else { None };

        Ok(Self {
            firmware_ctrl: read_u32(b, 36)?,
            dsdt: read_u32(b, 40)?,
            preferred_power_management_profile: b[45],
            sci_interrupt: read_u16(b, 46)?,
            smi_command_port: read_u32(b, 48)?,
            acpi_enable: b[52],
            acpi_disable: b[53],
            pm1a_control_block: read_u32(b, 64)?,
            pm1b_control_block: read_u32(b, 68)?,
            flags: read_u32(b, 112)?,
            reset_register,
            reset_value,
            x_firmware_ctrl,
            x_dsdt,
        })
    }

    pub fn firmware_control_address(&self) -> u64 {
        self.x_firmware_ctrl.filter(|address| *address != 0)
            .unwrap_or(self.firmware_ctrl as u64)
    }

    pub fn dsdt_address(&self) -> u64 {
        self.x_dsdt.filter(|address| *address != 0)
            .unwrap_or(self.dsdt as u64)
    }
}

/// ACPI Generic Address Structure (GAS).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenericAddress {
    pub address_space: u8,
    pub bit_width: u8,
    pub bit_offset: u8,
    pub access_size: u8,
    pub address: u64,
}

impl GenericAddress {
    fn parse(bytes: &[u8]) -> Result<Self, AcpiError> {
        if bytes.len() < 12 {
            return Err(AcpiError::BufferTooSmall);
        }
        Ok(Self {
            address_space: bytes[0],
            bit_width: bytes[1],
            bit_offset: bytes[2],
            access_size: bytes[3],
            address: read_u64(bytes, 4)?,
        })
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, AcpiError> {
    let b = bytes.get(offset..offset + 2).ok_or(AcpiError::BufferTooSmall)?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, AcpiError> {
    let b = bytes.get(offset..offset + 4).ok_or(AcpiError::BufferTooSmall)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, AcpiError> {
    let b = bytes.get(offset..offset + 8).ok_or(AcpiError::BufferTooSmall)?;
    Ok(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
}
