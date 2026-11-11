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
    pub register_block_lengths: PmRegisterBlockLengths,
    pub flags: u32,
    pub reset_register: Option<GenericAddress>,
    pub reset_value: Option<u8>,
    pub x_firmware_ctrl: Option<u64>,
    pub x_dsdt: Option<u64>,
    pub extended_pm_registers: Option<ExtendedPmRegisters>,
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

        // Known FADT fields are optional by table length, but a table must
        // never end in the middle of a multi-byte field. This also prevents
        // partially supplied GAS and 64-bit pointers from being mistaken for
        // absent fields.
        if (b.len() > 116 && b.len() < 128)
            || (b.len() > 132 && b.len() < 140)
            || (b.len() > 140 && b.len() < 148)
            || (b.len() > 148 && b.len() < 244)
        {
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
        // ACPI FADT extended PM register GAS fields occupy bytes 148..244.
        let extended_pm_registers = if b.len() >= 244 {
            Some(ExtendedPmRegisters {
                pm1a_event: GenericAddress::parse(&b[148..160])?,
                pm1b_event: GenericAddress::parse(&b[160..172])?,
                pm1a_control: GenericAddress::parse(&b[172..184])?,
                pm1b_control: GenericAddress::parse(&b[184..196])?,
                pm2_control: GenericAddress::parse(&b[196..208])?,
                pm_timer: GenericAddress::parse(&b[208..220])?,
                gpe0: GenericAddress::parse(&b[220..232])?,
                gpe1: GenericAddress::parse(&b[232..244])?,
            })
        } else {
            None
        };

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
            register_block_lengths: PmRegisterBlockLengths {
                pm1_event: b[88],
                pm1_control: b[89],
                pm2_control: b[90],
                pm_timer: b[91],
                gpe0: b[92],
                gpe1: b[93],
            },
            flags,
            reset_register,
            reset_value,
            x_firmware_ctrl,
            x_dsdt,
            extended_pm_registers,
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

    /// Validate the FADT fixed-register block lengths from ACPI 6.5 §5.2.9.
    /// This is metadata validation only; it does not probe or access hardware.
    pub fn validate_register_block_lengths(&self) -> Result<(), AcpiError> {
        self.register_block_lengths.validate()
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

/// Byte lengths declared by the legacy FADT fixed-register block fields.
///
/// These lengths also describe the corresponding extended GAS blocks when a
/// nonzero extended address is supplied. Optional PM2/timer/GPE blocks may be
/// absent; PM1 event/control lengths may be zero on reduced-hardware platforms.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PmRegisterBlockLengths {
    pub pm1_event: u8,
    pub pm1_control: u8,
    pub pm2_control: u8,
    pub pm_timer: u8,
    pub gpe0: u8,
    pub gpe1: u8,
}

impl PmRegisterBlockLengths {
    /// Validate size constraints defined by the ACPI register model.
    pub fn validate(self) -> Result<(), AcpiError> {
        if (self.pm1_event != 0 && (self.pm1_event < 4 || self.pm1_event % 2 != 0))
            || (self.pm1_control != 0 && self.pm1_control < 2)
            || (self.pm_timer != 0 && self.pm_timer != 4)
            || self.gpe0 % 2 != 0
            || self.gpe1 % 2 != 0
        {
            return Err(AcpiError::InvalidLength);
        }
        Ok(())
    }
}

/// Extended FADT power-management register descriptors (ACPI GAS).
///
/// A zero address denotes an absent register block. These descriptors do not
/// imply that the corresponding hardware block is enabled or safe to access.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExtendedPmRegisters {
    pub pm1a_event: GenericAddress,
    pub pm1b_event: GenericAddress,
    pub pm1a_control: GenericAddress,
    pub pm1b_control: GenericAddress,
    pub pm2_control: GenericAddress,
    pub pm_timer: GenericAddress,
    pub gpe0: GenericAddress,
    pub gpe1: GenericAddress,
}

impl ExtendedPmRegisters {
    /// Validate each populated GAS descriptor without performing register I/O.
    pub fn validate(&self) -> Result<(), AcpiError> {
        for gas in [
            self.pm1a_event, self.pm1b_event, self.pm1a_control, self.pm1b_control,
            self.pm2_control, self.pm_timer, self.gpe0, self.gpe1,
        ] {
            if gas.is_present() {
                gas.validate()?;
                // FADT fixed power-management blocks are only defined in
                // System Memory or System I/O address space.
                if !gas.is_system_memory_or_io() {
                    return Err(AcpiError::InvalidGas);
                }
            }
        }
        Ok(())
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

    /// A zero GAS address denotes an unimplemented register block.
    pub const fn is_present(self) -> bool {
        self.address != 0
    }

    /// Validate GAS field encoding and arithmetic without dereferencing address.
    pub fn validate(self) -> Result<(), AcpiError> {
        if self.address == 0 || self.bit_width == 0 {
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
    fn parses_and_validates_extended_pm_gas_registers() {
        let mut bytes = std::vec![0u8; 244];
        bytes[0..4].copy_from_slice(b"FACP");
        bytes[4..8].copy_from_slice(&(244u32).to_le_bytes());
        bytes[8] = 6;
        // X_PM1a_CNT_BLK at offset 172: System I/O, 16-bit register, word access.
        bytes[172] = GenericAddress::ADDRESS_SPACE_SYSTEM_IO;
        bytes[173] = 16;
        bytes[175] = GenericAddress::ACCESS_WORD;
        bytes[176..184].copy_from_slice(&0x404u64.to_le_bytes());
        let checksum = 0u8.wrapping_sub(bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        bytes[9] = checksum;

        let fadt = Fadt::parse(&bytes).unwrap();
        let registers = fadt.extended_pm_registers.unwrap();
        assert_eq!(registers.pm1a_control.address, 0x404);
        assert_eq!(registers.pm1a_control.validate(), Ok(()));
        assert_eq!(registers.validate(), Ok(()));

        let mut malformed = bytes;
        malformed[174] = 8; // bit offset + width exceeds the 16-bit access.
        malformed[9] = 0;
        let checksum = 0u8.wrapping_sub(malformed.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        malformed[9] = checksum;
        let fadt = Fadt::parse(&malformed).unwrap();
        assert_eq!(fadt.extended_pm_registers.unwrap().validate(), Err(AcpiError::InvalidGas));
    }

    #[test]
    fn rejects_fadt_lengths_that_truncate_known_fields() {
        for length in [117usize, 127, 133, 139, 141, 147, 149, 243] {
            let bytes = make_fadt(length, 0, None, None);
            assert_eq!(
                Fadt::parse(&bytes),
                Err(AcpiError::InvalidLength),
                "unexpected acceptance of FADT length {length}"
            );
        }

        for length in [116usize, 128, 129, 132, 140, 148, 244] {
            let bytes = make_fadt(length, 0, None, None);
            assert!(Fadt::parse(&bytes).is_ok(), "valid field boundary {length}");
        }
    }

    #[test]
    fn validates_fixed_register_block_lengths() {
        let valid = PmRegisterBlockLengths {
            pm1_event: 4, pm1_control: 2, pm2_control: 1,
            pm_timer: 4, gpe0: 8, gpe1: 2,
        };
        assert_eq!(valid.validate(), Ok(()));
        assert_eq!(PmRegisterBlockLengths { pm1_event: 2, ..valid }.validate(), Err(AcpiError::InvalidLength));
        assert_eq!(PmRegisterBlockLengths { pm1_event: 5, ..valid }.validate(), Err(AcpiError::InvalidLength));
        assert_eq!(PmRegisterBlockLengths { pm1_control: 1, ..valid }.validate(), Err(AcpiError::InvalidLength));
        assert_eq!(PmRegisterBlockLengths { pm_timer: 3, ..valid }.validate(), Err(AcpiError::InvalidLength));
        assert_eq!(PmRegisterBlockLengths { gpe0: 3, ..valid }.validate(), Err(AcpiError::InvalidLength));
        assert_eq!(PmRegisterBlockLengths {
            pm1_event: 0, pm1_control: 0, pm2_control: 0,
            pm_timer: 0, gpe0: 0, gpe1: 0,
        }.validate(), Ok(()));
    }

    #[test]
    fn parses_fadt_register_block_lengths() {
        let mut bytes = make_fadt(244, 0, None, None);
        bytes[88..94].copy_from_slice(&[4, 2, 1, 4, 8, 2]);
        bytes[9] = 0;
        let checksum = 0u8.wrapping_sub(bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        bytes[9] = checksum;
        let fadt = Fadt::parse(&bytes).unwrap();
        assert_eq!(fadt.register_block_lengths.pm1_event, 4);
        assert_eq!(fadt.validate_register_block_lengths(), Ok(()));

        bytes[91] = 3;
        bytes[9] = 0;
        let checksum = 0u8.wrapping_sub(bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        bytes[9] = checksum;
        let fadt = Fadt::parse(&bytes).unwrap();
        assert_eq!(fadt.validate_register_block_lengths(), Err(AcpiError::InvalidLength));
    }

    #[test]
    fn treats_zero_address_pm_gas_as_absent() {
        let empty = GenericAddress {
            address_space: 0xFF,
            bit_width: 0,
            bit_offset: 0xFF,
            access_size: 0xFF,
            address: 0,
        };
        assert!(!empty.is_present());
        let registers = ExtendedPmRegisters {
            pm1a_event: empty,
            pm1b_event: empty,
            pm1a_control: empty,
            pm1b_control: empty,
            pm2_control: empty,
            pm_timer: empty,
            gpe0: empty,
            gpe1: empty,
        };
        assert_eq!(registers.validate(), Ok(()));
    }

    #[test]
    fn rejects_non_memory_io_extended_pm_registers() {
        let mut bytes = std::vec![0u8; 244];
        bytes[0..4].copy_from_slice(b"FACP");
        bytes[4..8].copy_from_slice(&(244u32).to_le_bytes());
        bytes[8] = 6;
        // A populated PM GAS using PCI configuration space is not valid here.
        bytes[148] = 2;
        bytes[149] = 8;
        bytes[151] = GenericAddress::ACCESS_BYTE;
        bytes[152..160].copy_from_slice(&0x100u64.to_le_bytes());
        let checksum = 0u8.wrapping_sub(bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        bytes[9] = checksum;
        let fadt = Fadt::parse(&bytes).unwrap();
        assert_eq!(
            fadt.extended_pm_registers.unwrap().validate(),
            Err(AcpiError::InvalidGas)
        );
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
    fn prefers_nonzero_extended_facs_pointer_and_falls_back_to_legacy() {
        let mut bytes = make_fadt(148, 0, None, None);
        bytes[36..40].copy_from_slice(&0x4000u32.to_le_bytes());
        bytes[132..140].copy_from_slice(&0x1_0000_4000u64.to_le_bytes());
        bytes[9] = 0;
        let checksum = 0u8.wrapping_sub(bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        bytes[9] = checksum;
        let fadt = Fadt::parse(&bytes).unwrap();
        assert_eq!(fadt.firmware_control_address(), 0x1_0000_4000);

        let mut legacy = make_fadt(148, 0, None, None);
        legacy[36..40].copy_from_slice(&0x5000u32.to_le_bytes());
        legacy[132..140].copy_from_slice(&0u64.to_le_bytes());
        legacy[9] = 0;
        let checksum = 0u8.wrapping_sub(legacy.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        legacy[9] = checksum;
        let fadt = Fadt::parse(&legacy).unwrap();
        assert_eq!(fadt.firmware_control_address(), 0x5000);
    }

    #[test]
    fn validates_gas_width_offset_access_and_address() {
        assert_eq!(valid_reset_gas().validate(), Ok(()));
        let mut gas = valid_reset_gas();
        gas.bit_width = 16;
        assert_eq!(gas.validate(), Err(AcpiError::InvalidGas));
        gas = valid_reset_gas();
        gas.bit_offset = 8;
        gas.bit_width = 8;
        gas.access_size = GenericAddress::ACCESS_DWORD;
        assert_eq!(gas.validate(), Ok(()));
        gas.bit_offset = 31;
        gas.bit_width = 2;
        assert_eq!(gas.validate(), Err(AcpiError::InvalidGas));
        gas = valid_reset_gas();
        gas.access_size = 5;
        assert_eq!(gas.validate(), Err(AcpiError::InvalidGas));
        gas = valid_reset_gas();
        gas.address = 0;
        assert_eq!(gas.validate(), Err(AcpiError::InvalidGas));
    }
}
