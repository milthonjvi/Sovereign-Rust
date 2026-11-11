//! Fixed ACPI Description Table (FADT) common fields.

use super::{AcpiError, sdt::Sdt};

pub const FADT_SIGNATURE: [u8; 4] = *b"FACP";
const FADT_RESET_REG_SUP: u32 = 1 << 10;
const FADT_HW_REDUCED_ACPI: u32 = 1 << 20;

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
    pub s4bios_request: u8,
    pub pstate_control: u8,
    /// IA-PC boot architecture capability flags (FADT offset 109).
    pub iapc_boot_architecture: u16,
    /// ARM boot architecture flags (FADT offset 129), when present.
    pub arm_boot_architecture: Option<u16>,
    /// FADT minor version (offset 131), when present.
    pub minor_version: Option<u8>,
    pub pm1a_control_block: u32,
    pub pm1b_control_block: u32,
    pub legacy_pm_registers: LegacyPmRegisters,
    pub register_block_lengths: PmRegisterBlockLengths,
    pub flags: u32,
    pub reset_register: Option<GenericAddress>,
    pub reset_value: Option<u8>,
    pub x_firmware_ctrl: Option<u64>,
    pub x_dsdt: Option<u64>,
    pub extended_pm_registers: Option<ExtendedPmRegisters>,
    pub sleep_control_register: Option<GenericAddress>,
    pub sleep_status_register: Option<GenericAddress>,
    /// Hypervisor vendor identity introduced in ACPI 6.2 (FADT offset 268).
    /// Zero means firmware does not identify a hypervisor.
    pub hypervisor_vendor_identity: Option<u64>,
}

/// Platform power-management model advertised by FADT flags.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PmHardwareMode { LegacyFixedHardware, HardwareReduced }

/// Validated source of a fixed PM register block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PmRegisterAddress { SystemIo(u32), Gas(GenericAddress) }

/// Resolved fixed-register map. Reduced-hardware platforms intentionally
/// expose no legacy fixed-register addresses through this structure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PmRegisterMap {
    pub mode: PmHardwareMode,
    pub pm1a_event: Option<PmRegisterAddress>, pub pm1b_event: Option<PmRegisterAddress>,
    pub pm1a_control: Option<PmRegisterAddress>, pub pm1b_control: Option<PmRegisterAddress>,
    pub pm2_control: Option<PmRegisterAddress>, pub pm_timer: Option<PmRegisterAddress>,
    pub gpe0: Option<PmRegisterAddress>, pub gpe1: Option<PmRegisterAddress>,
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
            || (b.len() > 129 && b.len() < 131)
            || (b.len() > 132 && b.len() < 140)
            || (b.len() > 140 && b.len() < 148)
            || (b.len() > 148 && b.len() < 244)
            || (b.len() > 244 && b.len() < 268)
            || (b.len() > 268 && b.len() < 276)
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

        // ACPI 5.0+ optional Sleep Control/Status GAS fields.
        let (sleep_control_register, sleep_status_register) = if b.len() >= 268 {
            (Some(GenericAddress::parse(&b[244..256])?),
             Some(GenericAddress::parse(&b[256..268])?))
        } else { (None, None) };
        let hypervisor_vendor_identity = if b.len() >= 276 {
            Some(read_u64(b, 268)?)
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
            s4bios_request: b[54],
            pstate_control: b[55],
            iapc_boot_architecture: read_u16(b, 109)?,
            arm_boot_architecture: if b.len() >= 131 { Some(read_u16(b, 129)?) } else { None },
            minor_version: b.get(131).copied(),
            pm1a_control_block: read_u32(b, 64)?,
            pm1b_control_block: read_u32(b, 68)?,
            legacy_pm_registers: LegacyPmRegisters {
                pm1a_event: read_u32(b, 56)?,
                pm1b_event: read_u32(b, 60)?,
                pm1a_control: read_u32(b, 64)?,
                pm1b_control: read_u32(b, 68)?,
                pm2_control: read_u32(b, 72)?,
                pm_timer: read_u32(b, 76)?,
                gpe0: read_u32(b, 80)?,
                gpe1: read_u32(b, 84)?,
            },
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
            sleep_control_register,
            sleep_status_register,
            hypervisor_vendor_identity,
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

    /// Return validated sleep-control/status GAS descriptors for
    /// hardware-reduced ACPI. No register access is performed.
    pub fn validated_sleep_registers(
        &self,
    ) -> Result<Option<(GenericAddress, GenericAddress)>, AcpiError> {
        if self.pm_hardware_mode() != PmHardwareMode::HardwareReduced {
            return Ok(None);
        }
        let (control, status) = match (
            self.sleep_control_register,
            self.sleep_status_register,
        ) {
            (None, None) => return Ok(None),
            (Some(control), Some(status)) => (control, status),
            // The two optional sleep registers form one interface. A
            // half-populated pair is malformed rather than partially usable.
            _ => return Err(AcpiError::InvalidGas),
        };
        for gas in [control, status] {
            gas.validate()?;
            if !gas.is_valid_byte_register_address()
                || gas.bit_width != 8
                || gas.bit_offset != 0
                || !gas.has_byte_access_encoding()
            {
                return Err(AcpiError::InvalidGas);
            }
        }
        Ok(Some((control, status)))
    }

    /// Select the PM model advertised by the FADT.
    pub const fn pm_hardware_mode(&self) -> PmHardwareMode {
        if self.flags & FADT_HW_REDUCED_ACPI != 0 { PmHardwareMode::HardwareReduced }
        else { PmHardwareMode::LegacyFixedHardware }
    }

    /// Resolve extended GAS descriptors before legacy System I/O addresses.
    /// This returns metadata only and never accesses a register.
    pub fn resolve_pm_register_map(&self) -> Result<PmRegisterMap, AcpiError> {
        let mode = self.pm_hardware_mode();
        if mode == PmHardwareMode::HardwareReduced {
            return Ok(PmRegisterMap { mode, pm1a_event: None, pm1b_event: None,
                pm1a_control: None, pm1b_control: None, pm2_control: None,
                pm_timer: None, gpe0: None, gpe1: None });
        }
        self.validate_pm_registers()?;
        let ext = self.extended_pm_registers;
        let choose = |gas: Option<GenericAddress>, legacy: u32, length: u8|
            -> Result<Option<PmRegisterAddress>, AcpiError> {
            if let Some(gas) = gas.filter(|g| g.is_present()) {
                return Ok(Some(PmRegisterAddress::Gas(gas)));
            }
            if legacy == 0 || length == 0 {
                return Ok(None);
            }

            // This kernel targets x86_64, whose processor I/O-port space is
            // 16-bit. Validate the complete block before exposing its base.
            let end = legacy.checked_add(length as u32).ok_or(AcpiError::UnsupportedAddress)?;
            if legacy > u16::MAX as u32 || end > 0x1_0000 {
                return Err(AcpiError::UnsupportedAddress);
            }
            Ok(Some(PmRegisterAddress::SystemIo(legacy)))
        };
        let get = |f: fn(&ExtendedPmRegisters) -> GenericAddress| ext.map(f);
        let l = self.register_block_lengths;
        let r = self.legacy_pm_registers;
        Ok(PmRegisterMap { mode,
            pm1a_event: choose(get(|x| x.pm1a_event), r.pm1a_event, l.pm1_event)?,
            pm1b_event: choose(get(|x| x.pm1b_event), r.pm1b_event, l.pm1_event)?,
            pm1a_control: choose(get(|x| x.pm1a_control), r.pm1a_control, l.pm1_control)?,
            pm1b_control: choose(get(|x| x.pm1b_control), r.pm1b_control, l.pm1_control)?,
            pm2_control: choose(get(|x| x.pm2_control), r.pm2_control, l.pm2_control)?,
            pm_timer: choose(get(|x| x.pm_timer), r.pm_timer, l.pm_timer)?,
            gpe0: choose(get(|x| x.gpe0), r.gpe0, l.gpe0)?,
            gpe1: choose(get(|x| x.gpe1), r.gpe1, l.gpe1)?,
        })
    }

    /// Validate the complete PM register metadata without accessing hardware.
    /// Extended GAS widths must describe the byte lengths declared by the FADT.
    pub fn validate_pm_registers(&self) -> Result<(), AcpiError> {
        // ACPI requires OSPM to ignore the legacy fixed-register interface
        // entirely when HW_REDUCED_ACPI is set.
        if self.pm_hardware_mode() == PmHardwareMode::HardwareReduced {
            return Ok(());
        }
        let lengths = self.register_block_lengths;
        let legacy = self.legacy_pm_registers;
        let extended = self.extended_pm_registers;
        lengths.validate()?;
        if let Some(registers) = extended {
            registers.validate_against_lengths(lengths)?;
        }

        let present = |gas: Option<GenericAddress>, address: u32| {
            gas.map(|descriptor| descriptor.is_present()).unwrap_or(false) || address != 0
        };
        // PM1a event and control blocks are mandatory in the full fixed-
        // hardware model. PM1b remains optional.
        if lengths.pm1_event < 4
            || lengths.pm1_control < 2
            || !present(extended.map(|x| x.pm1a_event), legacy.pm1a_event)
            || !present(extended.map(|x| x.pm1a_control), legacy.pm1a_control)
        {
            return Err(AcpiError::InvalidGas);
        }
        if (legacy.pm1b_event != 0 || present(extended.map(|x| x.pm1b_event), 0))
            && lengths.pm1_event == 0
        {
            return Err(AcpiError::InvalidLength);
        }
        if (legacy.pm1b_control != 0 || present(extended.map(|x| x.pm1b_control), 0))
            && lengths.pm1_control == 0
        {
            return Err(AcpiError::InvalidLength);
        }

        // Optional blocks must be described consistently: a nonzero length
        // requires an address, and a legacy address cannot have zero length.
        for (gas, address, length) in [
            (extended.map(|x| x.pm2_control), legacy.pm2_control, lengths.pm2_control),
            (extended.map(|x| x.pm_timer), legacy.pm_timer, lengths.pm_timer),
            (extended.map(|x| x.gpe0), legacy.gpe0, lengths.gpe0),
            (extended.map(|x| x.gpe1), legacy.gpe1, lengths.gpe1),
        ] {
            if present(gas, address) != (length != 0) {
                return Err(AcpiError::InvalidLength);
            }
        }
        Ok(())
    }

    /// Validate reserved and currently-defined encodings in FADT Flags.
    pub fn validate_fixed_feature_flags(&self) -> Result<(), AcpiError> {
        // ACPI 6.5 defines bits 0..23. Persistent CPU cache encoding 3 is reserved.
        if self.flags & 0xFF00_0000 != 0 || ((self.flags >> 22) & 0b11) == 0b11 {
            return Err(AcpiError::MalformedTable);
        }
        Ok(())
    }

    /// Validate reserved bits in the IA-PC and ARM boot-architecture fields.
    /// This is explicit so callers can inspect older or non-x86 firmware data.
    pub fn validate_boot_architecture_flags(&self) -> Result<(), AcpiError> {
        if self.iapc_boot_architecture & !0x003F != 0 {
            return Err(AcpiError::MalformedTable);
        }
        if self.arm_boot_architecture.is_some_and(|flags| flags & !0x0003 != 0) {
            return Err(AcpiError::MalformedTable);
        }
        Ok(())
    }

    pub const fn has_legacy_devices(&self) -> bool { self.iapc_boot_architecture & (1 << 0) != 0 }
    pub const fn has_8042_controller(&self) -> bool { self.iapc_boot_architecture & (1 << 1) != 0 }
    pub const fn vga_hardware_not_present(&self) -> bool { self.iapc_boot_architecture & (1 << 2) != 0 }
    pub const fn msi_not_supported(&self) -> bool { self.iapc_boot_architecture & (1 << 3) != 0 }
    pub const fn pcie_aspm_controls_not_supported(&self) -> bool { self.iapc_boot_architecture & (1 << 4) != 0 }
    pub const fn cmos_rtc_not_present(&self) -> bool { self.iapc_boot_architecture & (1 << 5) != 0 }

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
        if !gas.is_valid_byte_register_address()
            || gas.bit_width != 8
            || gas.bit_offset != 0
            || !gas.has_byte_access_encoding()
        {
            return Err(AcpiError::InvalidGas);
        }
        Ok(Some((gas, value)))
    }
}

/// Legacy System I/O port addresses from the FADT fixed-register fields.
///
/// These are raw firmware descriptors, not permission to issue port I/O.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LegacyPmRegisters {
    pub pm1a_event: u32,
    pub pm1b_event: u32,
    pub pm1a_control: u32,
    pub pm1b_control: u32,
    pub pm2_control: u32,
    pub pm_timer: u32,
    pub gpe0: u32,
    pub gpe1: u32,
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
        if (self.pm1_event != 0 && self.pm1_event < 4)
            || (self.pm1_control != 0 && self.pm1_control < 2)
            || (self.pm2_control != 0 && self.pm2_control < 1)
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
    /// Validate populated GAS descriptors and their FADT-declared block sizes.
    pub fn validate_against_lengths(
        &self,
        lengths: PmRegisterBlockLengths,
    ) -> Result<(), AcpiError> {
        self.validate()?;
        let checks = [
            (self.pm1a_event, lengths.pm1_event, 0u8),
            (self.pm1b_event, lengths.pm1_event, 0),
            (self.pm1a_control, lengths.pm1_control, 0),
            (self.pm1b_control, lengths.pm1_control, 0),
            (self.pm2_control, lengths.pm2_control, 1),
            (self.pm_timer, lengths.pm_timer, 0),
            (self.gpe0, lengths.gpe0, 2),
            (self.gpe1, lengths.gpe1, 2),
        ];
        for (gas, length, kind) in checks {
            if !gas.is_present() { continue; }
            if length == 0 { return Err(AcpiError::InvalidGas); }
            match kind {
                1 if gas.bit_width != 8 || gas.bit_offset != 0 => return Err(AcpiError::InvalidGas),
                2 => {},
                _ if gas.bit_width as u16 != (length as u16) * 8 => return Err(AcpiError::InvalidGas),
                _ => {},
            }
            let end = gas.address.checked_add(length as u64)
                .ok_or(AcpiError::UnsupportedAddress)?;
            if gas.address_space == GenericAddress::ADDRESS_SPACE_SYSTEM_IO
                && (gas.address > u16::MAX as u64 || end > 0x1_0000)
            {
                return Err(AcpiError::UnsupportedAddress);
            }
        }
        Ok(())
    }

    /// Validate each populated GAS descriptor without performing register I/O.
    pub fn validate(&self) -> Result<(), AcpiError> {
        for gas in [
            self.pm1a_event, self.pm1b_event, self.pm1a_control, self.pm1b_control,
            self.pm2_control, self.pm_timer,
        ] {
            if gas.is_present() {
                gas.validate()?;
                if !gas.is_system_memory_or_io() { return Err(AcpiError::InvalidGas); }
            }
        }
        // ACPI directs OSPM to ignore GAS width, offset and access-size fields
        // for X_GPE0_BLK and X_GPE1_BLK.
        for gas in [self.gpe0, self.gpe1] {
            if gas.is_present() && !gas.is_system_memory_or_io() {
                return Err(AcpiError::InvalidGas);
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
    pub const ADDRESS_SPACE_PCI_CONFIG: u8 = 2;

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

    /// Undefined access size is permitted; consumers infer byte access from
    /// the fixed register's declared 8-bit width.
    pub const fn has_byte_access_encoding(self) -> bool {
        self.access_size == Self::ACCESS_UNDEFINED || self.access_size == Self::ACCESS_BYTE
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

    /// Whether this GAS uses an address space permitted for FADT reset and
    /// HW-reduced sleep registers. PCI configuration is restricted by ACPI to
    /// segment group 0, bus 0; its GAS address encodes device, function, offset.
    pub const fn is_system_memory_io_or_pci_config_bus0(self) -> bool {
        if self.is_system_memory_or_io() {
            return true;
        }
        if self.address_space != Self::ADDRESS_SPACE_PCI_CONFIG {
            return false;
        }

        let device = (self.address >> 32) & 0xFFFF;
        let function = (self.address >> 16) & 0xFFFF;
        let offset = self.address & 0xFFFF;
        (self.address >> 48) == 0 && device <= 31 && function <= 7 && offset <= 0x0FFF
    }

    /// Validate a fixed 8-bit register's address space and address encoding.
    /// x86 System I/O addresses are limited to the architectural 16-bit port
    /// space; PCI Configuration addresses use the ACPI bus-zero encoding.
    pub const fn is_valid_byte_register_address(self) -> bool {
        if self.address_space == Self::ADDRESS_SPACE_SYSTEM_MEMORY {
            return true;
        }
        if self.address_space == Self::ADDRESS_SPACE_SYSTEM_IO {
            return self.address <= u16::MAX as u64;
        }
        self.address_space == Self::ADDRESS_SPACE_PCI_CONFIG
            && self.is_system_memory_io_or_pci_config_bus0()
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
    fn parses_fadt_boot_architecture_and_minor_version_fields() {
        let mut b = make_fadt(132, 0, None, None);
        b[54] = 0xA5; b[55] = 0x5A;
        b[109..111].copy_from_slice(&0x003Fu16.to_le_bytes());
        b[129..131].copy_from_slice(&0x0003u16.to_le_bytes());
        b[131] = 0x21;
        b[9] = 0;
        b[9] = 0u8.wrapping_sub(b.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        let fadt = Fadt::parse(&b).unwrap();
        assert_eq!(fadt.s4bios_request, 0xA5);
        assert_eq!(fadt.pstate_control, 0x5A);
        assert_eq!(fadt.iapc_boot_architecture, 0x003F);
        assert_eq!(fadt.arm_boot_architecture, Some(0x0003));
        assert_eq!(fadt.minor_version, Some(0x21));
        assert!(fadt.has_legacy_devices()); assert!(fadt.has_8042_controller());
        assert!(fadt.vga_hardware_not_present()); assert!(fadt.msi_not_supported());
        assert!(fadt.pcie_aspm_controls_not_supported()); assert!(fadt.cmos_rtc_not_present());
        assert_eq!(fadt.validate_boot_architecture_flags(), Ok(()));
        let old = Fadt::parse(&make_fadt(129, 0, None, None)).unwrap();
        assert_eq!(old.arm_boot_architecture, None);
        assert_eq!(old.minor_version, None);
    }

    #[test]
    fn validates_fixed_feature_flag_reserved_bits_and_cache_encoding() {
        let mut b = make_fadt(116, 0, None, None);
        b[112..116].copy_from_slice(&(1u32 << 23).to_le_bytes());
        b[9] = 0;
        b[9] = 0u8.wrapping_sub(b.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        let fadt = Fadt::parse(&b).unwrap();
        assert_eq!(fadt.validate_fixed_feature_flags(), Ok(()));

        for flags in [(1u32 << 24), (0b11u32 << 22)] {
            let mut malformed = make_fadt(116, flags, None, None);
            malformed[9] = 0;
            malformed[9] = 0u8.wrapping_sub(
                malformed.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)),
            );
            let fadt = Fadt::parse(&malformed).unwrap();
            assert_eq!(fadt.validate_fixed_feature_flags(), Err(AcpiError::MalformedTable));
        }
    }

    #[test]
    fn rejects_reserved_fadt_boot_architecture_bits_when_explicitly_validated() {
        let mut b = make_fadt(132, 0, None, None);
        b[109..111].copy_from_slice(&0x0040u16.to_le_bytes());
        b[129..131].copy_from_slice(&0x0004u16.to_le_bytes());
        b[9] = 0;
        b[9] = 0u8.wrapping_sub(b.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        let fadt = Fadt::parse(&b).unwrap();
        assert_eq!(fadt.validate_boot_architecture_flags(), Err(AcpiError::MalformedTable));
    }

    #[test]
    fn rejects_fadt_lengths_that_truncate_known_fields() {
        for length in [117usize, 127, 130, 133, 139, 141, 147, 149, 243, 269, 275] {
            let bytes = make_fadt(length, 0, None, None);
            assert_eq!(
                Fadt::parse(&bytes),
                Err(AcpiError::InvalidLength),
                "unexpected acceptance of FADT length {length}"
            );
        }

        for length in [116usize, 128, 129, 131, 132, 140, 148, 244, 268, 276] {
            let bytes = make_fadt(length, 0, None, None);
            assert!(Fadt::parse(&bytes).is_ok(), "valid field boundary {length}");
        }
    }

    #[test]
    fn accepts_pm2_control_blocks_of_one_or_more_bytes() {
        let valid = PmRegisterBlockLengths {
            pm1_event: 4, pm1_control: 2, pm2_control: 1,
            pm_timer: 4, gpe0: 8, gpe1: 2,
        };
        assert_eq!(valid.validate(), Ok(()));
        assert_eq!(
            PmRegisterBlockLengths { pm2_control: 2, ..valid }.validate(),
            Ok(()),
        );
        assert_eq!(
            PmRegisterBlockLengths { pm2_control: 0, ..valid }.validate(),
            Ok(()),
        );
    }

    #[test]
    fn validates_fixed_register_block_lengths() {
        let valid = PmRegisterBlockLengths {
            pm1_event: 4, pm1_control: 2, pm2_control: 1,
            pm_timer: 4, gpe0: 8, gpe1: 2,
        };
        assert_eq!(valid.validate(), Ok(()));
        assert_eq!(PmRegisterBlockLengths { pm1_event: 3, ..valid }.validate(), Err(AcpiError::InvalidLength));
        assert_eq!(PmRegisterBlockLengths { pm1_control: 1, ..valid }.validate(), Err(AcpiError::InvalidLength));
        assert_eq!(PmRegisterBlockLengths { pm1_event: 6, ..valid }.validate(), Ok(()));
        assert_eq!(PmRegisterBlockLengths { pm1_control: 4, ..valid }.validate(), Ok(()));
        assert_eq!(PmRegisterBlockLengths { pm2_control: 2, ..valid }.validate(), Ok(()));
        assert_eq!(PmRegisterBlockLengths { pm_timer: 3, ..valid }.validate(), Err(AcpiError::InvalidLength));
        assert_eq!(PmRegisterBlockLengths { gpe0: 3, ..valid }.validate(), Err(AcpiError::InvalidLength));
        assert_eq!(PmRegisterBlockLengths {
            pm1_event: 0, pm1_control: 0, pm2_control: 0,
            pm_timer: 0, gpe0: 0, gpe1: 0,
        }.validate(), Ok(()));
    }

    #[test]
    fn parses_sleep_control_and_status_gas_fields() {
        let mut b = make_fadt(268, FADT_HW_REDUCED_ACPI, None, None);
        for (offset, address) in [(244usize, 0x1000u64), (256, 0x1001)] {
            b[offset] = GenericAddress::ADDRESS_SPACE_SYSTEM_MEMORY;
            b[offset + 1] = 8;
            b[offset + 3] = GenericAddress::ACCESS_BYTE;
            b[offset + 4..offset + 12].copy_from_slice(&address.to_le_bytes());
        }
        b[9] = 0; b[9] = 0u8.wrapping_sub(b.iter().fold(0u8, |s,x| s.wrapping_add(*x)));
        let fadt = Fadt::parse(&b).unwrap();
        let (control, status) = fadt.validated_sleep_registers().unwrap().unwrap();
        assert_eq!(control.address, 0x1000);
        assert_eq!(status.address, 0x1001);
    }

    #[test]
    fn parses_hypervisor_vendor_identity_and_rejects_truncation() {
        let mut b = make_fadt(276, 0, None, None);
        b[268..276].copy_from_slice(&0x534F_5645_5245_4947u64.to_le_bytes());
        b[9] = 0;
        b[9] = 0u8.wrapping_sub(b.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        let fadt = Fadt::parse(&b).unwrap();
        assert_eq!(fadt.hypervisor_vendor_identity, Some(0x534F_5645_5245_4947));

        for length in [269usize, 275] {
            let truncated = make_fadt(length, 0, None, None);
            assert_eq!(Fadt::parse(&truncated), Err(AcpiError::InvalidLength));
        }
        let old = Fadt::parse(&make_fadt(268, 0, None, None)).unwrap();
        assert_eq!(old.hypervisor_vendor_identity, None);
    }

    #[test]
    fn hardware_reduced_validation_ignores_legacy_pm_register_metadata() {
        let mut b = make_fadt(244, FADT_HW_REDUCED_ACPI, None, None);
        b[88] = 3; // Invalid PM1 event block length in legacy mode.
        b[89] = 7; // Invalid PM1 control block length in legacy mode.
        b[90] = 9; // Invalid PM2 control block length in legacy mode.
        b[91] = 3; // Invalid PM timer length in legacy mode.
        b[9] = 0;
        b[9] = 0u8.wrapping_sub(b.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        let fadt = Fadt::parse(&b).unwrap();
        assert_eq!(fadt.validate_pm_registers(), Ok(()));
        assert_eq!(fadt.resolve_pm_register_map().unwrap().pm1a_control, None);
    }

    #[test]
    fn validates_pm2_gas_as_one_byte_register_independent_of_block_length() {
        let mut b = make_fadt(244, 0, None, None);
        b[90] = 2;
        b[196] = GenericAddress::ADDRESS_SPACE_SYSTEM_IO;
        b[197] = 8;
        b[198] = 0;
        b[199] = GenericAddress::ACCESS_UNDEFINED;
        b[200..208].copy_from_slice(&0xB2u64.to_le_bytes());
        b[9] = 0;
        b[9] = 0u8.wrapping_sub(b.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        let fadt = Fadt::parse(&b).unwrap();
        assert_eq!(fadt.validate_pm_registers(), Ok(()));

        b[198] = 1;
        b[9] = 0;
        b[9] = 0u8.wrapping_sub(b.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        let malformed = Fadt::parse(&b).unwrap();
        assert_eq!(malformed.validate_pm_registers(), Err(AcpiError::InvalidGas));
    }

    #[test]
    fn rejects_truncated_sleep_register_fields() {
        let b = make_fadt(260, FADT_HW_REDUCED_ACPI, None, None);
        assert_eq!(Fadt::parse(&b), Err(AcpiError::InvalidLength));
    }

    #[test]
    fn reduced_hardware_sleep_registers_are_optional_but_must_be_a_pair() {
        let b = make_fadt(244, FADT_HW_REDUCED_ACPI, None, None);
        let fadt = Fadt::parse(&b).unwrap();
        assert_eq!(fadt.validated_sleep_registers(), Ok(None));

        let mut partial = make_fadt(268, FADT_HW_REDUCED_ACPI, None, None);
        partial[244] = GenericAddress::ADDRESS_SPACE_SYSTEM_MEMORY;
        partial[245] = 8;
        partial[247] = GenericAddress::ACCESS_BYTE;
        partial[248..256].copy_from_slice(&0x1000u64.to_le_bytes());
        partial[9] = 0;
        partial[9] = 0u8.wrapping_sub(
            partial.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)),
        );
        let fadt = Fadt::parse(&partial).unwrap();
        assert_eq!(fadt.validated_sleep_registers(), Err(AcpiError::InvalidGas));
    }

    #[test]
    fn requires_pm1a_event_and_control_in_full_hardware_mode() {
        let mut b = make_fadt(244, 0, None, None);
        b[56..60].copy_from_slice(&0x100u32.to_le_bytes());
        b[88] = 4;
        b[64..68].copy_from_slice(&0x104u32.to_le_bytes());
        b[89] = 2;
        b[9] = 0;
        b[9] = 0u8.wrapping_sub(b.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        let valid = Fadt::parse(&b).unwrap();
        assert_eq!(valid.validate_pm_registers(), Ok(()));

        b[56..60].fill(0);
        b[9] = 0;
        b[9] = 0u8.wrapping_sub(b.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        let missing_event = Fadt::parse(&b).unwrap();
        assert_eq!(missing_event.validate_pm_registers(), Err(AcpiError::InvalidGas));
    }

    #[test]
    fn requires_optional_pm_block_addresses_to_match_lengths() {
        let mut b = make_fadt(244, 0, None, None);
        b[56..60].copy_from_slice(&0x100u32.to_le_bytes());
        b[88] = 4;
        b[64..68].copy_from_slice(&0x104u32.to_le_bytes());
        b[89] = 2;
        b[90] = 1; // PM2 length without PM2 address.
        b[9] = 0;
        b[9] = 0u8.wrapping_sub(b.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        let fadt = Fadt::parse(&b).unwrap();
        assert_eq!(fadt.validate_pm_registers(), Err(AcpiError::InvalidLength));
    }

    #[test]
    fn resolves_extended_pm_gas_before_legacy_address() {
        let mut b = make_fadt(244, 0, None, None);
        b[56..60].copy_from_slice(&0x400u32.to_le_bytes()); b[88] = 4;
        b[64..68].copy_from_slice(&0x404u32.to_le_bytes()); b[89] = 2;
        b[172] = 0; b[173] = 16; b[175] = GenericAddress::ACCESS_WORD;
        b[176..184].copy_from_slice(&0xFEE0_1000u64.to_le_bytes());
        b[9] = 0; b[9] = 0u8.wrapping_sub(b.iter().fold(0u8, |s,x| s.wrapping_add(*x)));
        let map = Fadt::parse(&b).unwrap().resolve_pm_register_map().unwrap();
        assert_eq!(map.pm1a_control, Some(PmRegisterAddress::Gas(GenericAddress {
            address_space: 0, bit_width: 16, bit_offset: 0,
            access_size: GenericAddress::ACCESS_WORD, address: 0xFEE0_1000 })));
    }

    #[test]
    fn resolves_legacy_pm_when_extended_gas_is_absent() {
        let mut b = make_fadt(244, 0, None, None);
        b[56..60].copy_from_slice(&0x400u32.to_le_bytes()); b[88] = 4;
        b[64..68].copy_from_slice(&0x404u32.to_le_bytes()); b[89] = 2;
        b[9] = 0; b[9] = 0u8.wrapping_sub(b.iter().fold(0u8, |s,x| s.wrapping_add(*x)));
        let map = Fadt::parse(&b).unwrap().resolve_pm_register_map().unwrap();
        assert_eq!(map.pm1a_control, Some(PmRegisterAddress::SystemIo(0x404)));
    }

    #[test]
    fn rejects_legacy_io_blocks_outside_the_x86_port_space() {
        for (base, length) in [(0x1_0000u32, 2u8), (0xFFFF, 2)] {
            let mut b = make_fadt(244, 0, None, None);
            b[56..60].copy_from_slice(&0x100u32.to_le_bytes());
            b[88] = 4;
            b[64..68].copy_from_slice(&base.to_le_bytes());
            b[89] = length;
            b[9] = 0;
            b[9] = 0u8.wrapping_sub(b.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
            let fadt = Fadt::parse(&b).unwrap();
            assert_eq!(
                fadt.resolve_pm_register_map(),
                Err(AcpiError::UnsupportedAddress),
            );
        }

        let mut b = make_fadt(244, 0, None, None);
        b[56..60].copy_from_slice(&0x100u32.to_le_bytes());
        b[88] = 4;
        b[64..68].copy_from_slice(&0xFFFEu32.to_le_bytes());
        b[89] = 2;
        b[9] = 0;
        b[9] = 0u8.wrapping_sub(b.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        let fadt = Fadt::parse(&b).unwrap();
        assert_eq!(
            fadt.resolve_pm_register_map().unwrap().pm1a_control,
            Some(PmRegisterAddress::SystemIo(0xFFFE)),
        );
    }

    #[test]
    fn reduced_hardware_never_exposes_legacy_fixed_registers() {
        let mut b = make_fadt(244, FADT_HW_REDUCED_ACPI, None, None);
        b[64..68].copy_from_slice(&0x404u32.to_le_bytes()); b[89] = 2;
        b[9] = 0; b[9] = 0u8.wrapping_sub(b.iter().fold(0u8, |s,x| s.wrapping_add(*x)));
        let map = Fadt::parse(&b).unwrap().resolve_pm_register_map().unwrap();
        assert_eq!(map.mode, PmHardwareMode::HardwareReduced);
        assert_eq!(map.pm1a_control, None); assert_eq!(map.pm1a_event, None);
    }

    #[test]
    fn malformed_extended_gas_is_not_hidden_by_legacy_fallback() {
        let mut b = make_fadt(244, 0, None, None);
        b[64..68].copy_from_slice(&0x404u32.to_le_bytes()); b[89] = 2;
        b[172] = 2; b[173] = 16; b[175] = GenericAddress::ACCESS_WORD;
        b[176..184].copy_from_slice(&0x1000u64.to_le_bytes());
        b[9] = 0; b[9] = 0u8.wrapping_sub(b.iter().fold(0u8, |s,x| s.wrapping_add(*x)));
        let fadt = Fadt::parse(&b).unwrap();
        assert_eq!(fadt.resolve_pm_register_map(), Err(AcpiError::InvalidGas));
    }

    #[test]
    fn parses_all_legacy_pm_register_addresses() {
        let mut bytes = make_fadt(116, 0, None, None);
        for (offset, address) in [
            (56, 0x100u32), (60, 0x104), (64, 0x108), (68, 0x10C),
            (72, 0x110), (76, 0x114), (80, 0x118), (84, 0x11C),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&address.to_le_bytes());
        }
        bytes[9] = 0;
        let checksum = 0u8.wrapping_sub(bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        bytes[9] = checksum;
        let fadt = Fadt::parse(&bytes).unwrap();
        assert_eq!(fadt.legacy_pm_registers.pm1a_event, 0x100);
        assert_eq!(fadt.legacy_pm_registers.pm1b_event, 0x104);
        assert_eq!(fadt.legacy_pm_registers.pm1a_control, 0x108);
        assert_eq!(fadt.legacy_pm_registers.pm1b_control, 0x10C);
        assert_eq!(fadt.legacy_pm_registers.pm2_control, 0x110);
        assert_eq!(fadt.legacy_pm_registers.pm_timer, 0x114);
        assert_eq!(fadt.legacy_pm_registers.gpe0, 0x118);
        assert_eq!(fadt.legacy_pm_registers.gpe1, 0x11C);
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
    fn checks_extended_gas_width_against_fadt_block_length() {
        let empty = GenericAddress {
            address_space: GenericAddress::ADDRESS_SPACE_SYSTEM_IO,
            bit_width: 0, bit_offset: 0,
            access_size: GenericAddress::ACCESS_UNDEFINED, address: 0,
        };
        let control = GenericAddress {
            address_space: GenericAddress::ADDRESS_SPACE_SYSTEM_IO,
            bit_width: 16, bit_offset: 0,
            access_size: GenericAddress::ACCESS_WORD, address: 0x404,
        };
        let registers = ExtendedPmRegisters {
            pm1a_event: empty, pm1b_event: empty,
            pm1a_control: control, pm1b_control: empty,
            pm2_control: empty, pm_timer: empty, gpe0: empty, gpe1: empty,
        };
        let lengths = PmRegisterBlockLengths {
            pm1_event: 4, pm1_control: 2, pm2_control: 0,
            pm_timer: 0, gpe0: 0, gpe1: 0,
        };
        assert_eq!(registers.validate_against_lengths(lengths), Ok(()));
        assert_eq!(registers.validate_against_lengths(
            PmRegisterBlockLengths { pm1_control: 4, ..lengths }
        ), Err(AcpiError::InvalidGas));
        assert_eq!(registers.validate_against_lengths(
            PmRegisterBlockLengths { pm1_control: 0, ..lengths }
        ), Err(AcpiError::InvalidGas));
    }

    #[test]
    fn rejects_extended_system_io_blocks_outside_x86_port_space() {
        let empty = GenericAddress {
            address_space: GenericAddress::ADDRESS_SPACE_SYSTEM_IO,
            bit_width: 0, bit_offset: 0,
            access_size: GenericAddress::ACCESS_UNDEFINED, address: 0,
        };
        let mut registers = ExtendedPmRegisters {
            pm1a_event: empty, pm1b_event: empty,
            pm1a_control: GenericAddress {
                address_space: GenericAddress::ADDRESS_SPACE_SYSTEM_IO,
                bit_width: 16, bit_offset: 0,
                access_size: GenericAddress::ACCESS_WORD, address: 0xFFFF,
            },
            pm1b_control: empty, pm2_control: empty,
            pm_timer: empty, gpe0: empty, gpe1: empty,
        };
        let lengths = PmRegisterBlockLengths {
            pm1_event: 0, pm1_control: 2, pm2_control: 0,
            pm_timer: 0, gpe0: 0, gpe1: 0,
        };
        assert_eq!(
            registers.validate_against_lengths(lengths),
            Err(AcpiError::UnsupportedAddress),
        );
        registers.pm1a_control.address = 0xFFFE;
        assert_eq!(registers.validate_against_lengths(lengths), Ok(()));
    }

    #[test]
    fn ignores_gpe_gas_width_offset_and_access_size_as_required() {
        let empty = GenericAddress {
            address_space: GenericAddress::ADDRESS_SPACE_SYSTEM_IO,
            bit_width: 0, bit_offset: 0,
            access_size: GenericAddress::ACCESS_UNDEFINED, address: 0,
        };
        let gpe0 = GenericAddress {
            address_space: GenericAddress::ADDRESS_SPACE_SYSTEM_IO,
            bit_width: 0, bit_offset: 0xFF,
            access_size: 0xFF, address: 0x500,
        };
        let registers = ExtendedPmRegisters {
            pm1a_event: empty, pm1b_event: empty,
            pm1a_control: empty, pm1b_control: empty,
            pm2_control: empty, pm_timer: empty,
            gpe0, gpe1: empty,
        };
        let lengths = PmRegisterBlockLengths {
            pm1_event: 0, pm1_control: 0, pm2_control: 0,
            pm_timer: 0, gpe0: 2, gpe1: 0,
        };
        assert_eq!(registers.validate_against_lengths(lengths), Ok(()));
    }

    #[test]
    fn rejects_fixed_byte_registers_outside_x86_io_port_space() {
        let mut gas = valid_reset_gas();
        gas.address = 0x1_0000;
        let fadt = Fadt::parse(&make_fadt(
            148, FADT_RESET_REG_SUP, Some(gas), Some(6),
        )).unwrap();
        assert_eq!(fadt.validated_reset_register(), Err(AcpiError::InvalidGas));

        let mut b = make_fadt(268, FADT_HW_REDUCED_ACPI, None, None);
        for (offset, address) in [(244usize, 0x1_0000u64), (256, 0x1000)] {
            b[offset] = GenericAddress::ADDRESS_SPACE_SYSTEM_IO;
            b[offset + 1] = 8;
            b[offset + 3] = GenericAddress::ACCESS_BYTE;
            b[offset + 4..offset + 12].copy_from_slice(&address.to_le_bytes());
        }
        b[9] = 0;
        b[9] = 0u8.wrapping_sub(b.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        let fadt = Fadt::parse(&b).unwrap();
        assert_eq!(fadt.validated_sleep_registers(), Err(AcpiError::InvalidGas));
    }

    #[test]
    fn accepts_undefined_access_size_for_byte_reset_register() {
        let mut gas = valid_reset_gas();
        gas.access_size = GenericAddress::ACCESS_UNDEFINED;
        let fadt = Fadt::parse(&make_fadt(
            148, FADT_RESET_REG_SUP, Some(gas), Some(6),
        )).unwrap();
        assert_eq!(fadt.validated_reset_register(), Ok(Some((gas, 6))));
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
    fn accepts_bus_zero_pci_configuration_reset_register() {
        let gas = GenericAddress {
            address_space: GenericAddress::ADDRESS_SPACE_PCI_CONFIG,
            bit_width: 8,
            bit_offset: 0,
            access_size: GenericAddress::ACCESS_BYTE,
            // Offset 0x23, function 2, device 7, segment 0 / bus 0.
            address: 0x0000_0007_0002_0023,
        };
        let fadt = Fadt::parse(&make_fadt(
            148, FADT_RESET_REG_SUP, Some(gas), Some(6),
        )).unwrap();
        assert_eq!(fadt.validated_reset_register(), Ok(Some((gas, 6))));

        for invalid_address in [
            0x0001_0007_0002_0023u64, // Reserved highest word.
            0x0000_0020_0002_0023,    // Device number exceeds 31.
            0x0000_0007_0008_0023,    // Function number exceeds 7.
            0x0000_0007_0002_1000,    // Configuration offset exceeds 0xFFF.
        ] {
            let mut invalid = gas;
            invalid.address = invalid_address;
            let fadt = Fadt::parse(&make_fadt(
                148, FADT_RESET_REG_SUP, Some(invalid), Some(6),
            )).unwrap();
            assert_eq!(fadt.validated_reset_register(), Err(AcpiError::InvalidGas));
        }
    }

    #[test]
    fn accepts_bus_zero_pci_configuration_sleep_registers() {
        let mut b = make_fadt(268, FADT_HW_REDUCED_ACPI, None, None);
        for (offset, address) in [
            (244usize, 0x0000_0007_0002_0023u64),
            (256usize, 0x0000_0007_0002_0024u64),
        ] {
            b[offset] = GenericAddress::ADDRESS_SPACE_PCI_CONFIG;
            b[offset + 1] = 8;
            b[offset + 3] = GenericAddress::ACCESS_BYTE;
            b[offset + 4..offset + 12].copy_from_slice(&address.to_le_bytes());
        }
        b[9] = 0;
        b[9] = 0u8.wrapping_sub(b.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        let fadt = Fadt::parse(&b).unwrap();
        let (control, status) = fadt.validated_sleep_registers().unwrap().unwrap();
        assert_eq!(control.address_space, GenericAddress::ADDRESS_SPACE_PCI_CONFIG);
        assert_eq!(status.address_space, GenericAddress::ADDRESS_SPACE_PCI_CONFIG);

        b[244 + 4..244 + 12].copy_from_slice(&0x0001_0007_0002_0023u64.to_le_bytes());
        b[9] = 0;
        b[9] = 0u8.wrapping_sub(b.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        let malformed = Fadt::parse(&b).unwrap();
        assert_eq!(malformed.validated_sleep_registers(), Err(AcpiError::InvalidGas));
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
        invalid.address_space = GenericAddress::ADDRESS_SPACE_PCI_CONFIG;
        invalid.address = 0x0001_0000_0000_0CF9; // Reserved PCI GAS bits must be zero.
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
