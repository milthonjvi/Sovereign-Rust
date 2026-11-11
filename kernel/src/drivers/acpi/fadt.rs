//! Fixed ACPI Description Table (FADT) common fields.

use super::{AcpiError, sdt::Sdt};

pub const FADT_SIGNATURE: [u8; 4] = *b"FACP";
const FADT_RESET_REG_SUP: u32 = 1 << 10;

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
    /// Parse common FADT fields from a checksum-validated SDT.
    pub fn parse(bytes: &[u8]) -> Result<Self, AcpiError> {
        let table = Sdt::parse(bytes)?;
        if table.signature() != FADT_SIGNATURE {
            return Err(AcpiError::InvalidSignature);
        }
        let b = table.bytes();
        if b.len() < 116 {
            return Err(AcpiError::InvalidLength);
        }

        let flags = read_u32(b, 112)?;
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
            flags,
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

    /// Whether firmware advertises a usable ACPI reset register.
    pub const fn reset_supported(&self) -> bool {
        self.flags & FADT_RESET_REG_SUP != 0
    }

    /// Return the reset GAS only when the FADT advertises it and its encoding
    /// matches the ACPI reset-register requirements. This does not perform I/O.
    pub fn validated_reset_register(&self) -> Result<Option<(GenericAddress, u8)>, AcpiError> {
        if !self.reset_supported() {
            return Ok(None);
        }
        let gas = self.reset_register.ok_or(AcpiError::InvalidGas)?;
        let value = self.reset_value.ok_or(AcpiError::InvalidGas)?;
        gas.validate()?;
        if !gas.is_system_memory_or_io()
            || gas.bit_width != 8
            || gas.bit_offset != 0
            || gas.access_size != GenericAddress::ACCESS_BYTE
        {
            return Err(AcpiError::InvalidGas);
        }
        Ok(Some((gas, value)))
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
    pub const ADDRESS_SPACE_SYSTEM_MEMORY: u8 = 0;
    pub const ADDRESS_SPACE_SYSTEM_IO: u8 = 1;

    pub const ACCESS_UNDEFINED: u8 = 0;
    pub const ACCESS_BYTE: u8 = 1;
    pub const ACCESS_WORD: u8 = 2;
    pub const ACCESS_DWORD: u8 = 3;
    pub const ACCESS_QWORD: u8 = 4;

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

    /// Validate GAS field encoding and arithmetic without dereferencing address.
    pub fn validate(self) -> Result<(), AcpiError> {
        if self.address == 0 || self.bit_width == 0 || self.bit_offset > 7 {
            return Err(AcpiError::InvalidGas);
        }
        if self.access_size > Self::ACCESS_QWORD {
            return Err(AcpiError::InvalidGas);
        }
        if self.access_size != Self::ACCESS_UNDEFINED {
            let access_bits = match self.access_size {
                Self::ACCESS_BYTE => 8u16,
                Self::ACCESS_WORD => 16,
                Self::ACCESS_DWORD => 32,
                Self::ACCESS_QWORD => 64,
                _ => return Err(AcpiError::InvalidGas),
            };
            if (self.bit_offset as u16) + (self.bit_width as u16) > access_bits {
                return Err(AcpiError::InvalidGas);
            }
        }
        Ok(())
    }

    pub const fn is_system_memory_or_io(self) -> bool {
        self.address_space == Self::ADDRESS_SPACE_SYSTEM_MEMORY
            || self.address_space == Self::ADDRESS_SPACE_SYSTEM_IO
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

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;

    fn make_fadt(length: usize, flags: u32, gas: Option<GenericAddress>, reset_value: Option<u8>)
        -> std::vec::Vec<u8>
    {
        let mut bytes = std::vec![0u8; length];
        bytes[0..4].copy_from_slice(b"FACP");
        bytes[4..8].copy_from_slice(&(length as u32).to_le_bytes());
        bytes[8] = 6;
        if length >= 116 {
            bytes[112..116].copy_from_slice(&flags.to_le_bytes());
        }
        if let Some(gas) = gas {
            bytes[116] = gas.address_space;
            bytes[117] = gas.bit_width;
            bytes[118] = gas.bit_offset;
            bytes[119] = gas.access_size;
            bytes[120..128].copy_from_slice(&gas.address.to_le_bytes());
        }
        if let Some(value) = reset_value {
            if length > 128 { bytes[128] = value; }
        }
        let checksum = 0u8.wrapping_sub(bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        bytes[9] = checksum;
        bytes
    }

    fn valid_reset_gas() -> GenericAddress {
        GenericAddress {
            address_space: GenericAddress::ADDRESS_SPACE_SYSTEM_IO,
            bit_width: 8,
            bit_offset: 0,
            access_size: GenericAddress::ACCESS_BYTE,
            address: 0xCF9,
        }
    }

    #[test]
    fn validates_reset_register_only_when_advertised() {
        let unsupported = Fadt::parse(&make_fadt(148, 0, Some(valid_reset_gas()), Some(6))).unwrap();
        assert!(!unsupported.reset_supported());
        assert_eq!(unsupported.validated_reset_register(), Ok(None));

        let supported = Fadt::parse(&make_fadt(148, FADT_RESET_REG_SUP,
            Some(valid_reset_gas()), Some(6))).unwrap();
        assert_eq!(supported.validated_reset_register(), Ok(Some((valid_reset_gas(), 6))));
    }

    #[test]
    fn rejects_missing_or_unsafe_reset_register() {
        let missing = Fadt::parse(&make_fadt(116, FADT_RESET_REG_SUP, None, None)).unwrap();
        assert_eq!(missing.validated_reset_register(), Err(AcpiError::InvalidGas));

        let mut invalid = valid_reset_gas();
        invalid.address_space = 2; // PCI configuration space is not a reset GAS target.
        let fadt = Fadt::parse(&make_fadt(148, FADT_RESET_REG_SUP, Some(invalid), Some(6))).unwrap();
        assert_eq!(fadt.validated_reset_register(), Err(AcpiError::InvalidGas));
    }

    #[test]
    fn validates_gas_width_offset_access_and_address() {
        assert_eq!(valid_reset_gas().validate(), Ok(()));
        let mut gas = valid_reset_gas();
        gas.bit_width = 16;
        assert_eq!(gas.validate(), Err(AcpiError::InvalidGas));
        gas = valid_reset_gas();
        gas.bit_offset = 8;
        assert_eq!(gas.validate(), Err(AcpiError::InvalidGas));
        gas = valid_reset_gas();
        gas.access_size = 5;
        assert_eq!(gas.validate(), Err(AcpiError::InvalidGas));
        gas = valid_reset_gas();
        gas.address = 0;
        assert_eq!(gas.validate(), Err(AcpiError::InvalidGas));
    }
}
