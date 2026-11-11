//! Read-only decoding of PCI MSI and MSI-X capabilities.
//!
//! This module discovers capability state and resource locations only. It does
//! not enable MSI/MSI-X, program message addresses, or touch device BAR memory.
use super::capabilities::Capability;
use super::config::{Bar, BarKind, ConfigAccess, ConfigError, PciAddress, PciIdentity, PCI_CONFIG_SIZE};
use super::resources::BarResource;

pub const MSI_CAPABILITY_ID: u8 = 0x05;
pub const MSIX_CAPABILITY_ID: u8 = 0x11;
const MAX_STANDARD_CAPABILITY_OFFSET: u16 = 0xfc;

fn read_standard<A: ConfigAccess>(io: &A, address: PciAddress, register: u16) -> Result<u32, ConfigError> {
    if register >= PCI_CONFIG_SIZE { return Err(ConfigError::OutOfRange); }
    io.read_u32(address, register)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MsiCapability {
    pub offset: u8,
    pub enabled: bool,
    pub supports_64_bit_address: bool,
    pub per_vector_masking: bool,
    pub maximum_vectors: u8,
    pub enabled_vectors: u8,
    pub message_address: u64,
    pub message_data: u16,
    pub mask_bits: Option<u32>,
    pub pending_bits: Option<u32>,
}

impl MsiCapability {
    /// Read the current MSI capability registers without changing device state.
    pub fn read<A: ConfigAccess>(
        io: &A,
        address: PciAddress,
        capability: Capability,
    ) -> Result<Self, ConfigError> {
        if capability.id != MSI_CAPABILITY_ID {
            return Err(ConfigError::OutOfRange);
        }
        let offset = capability.offset as u16;
        if offset < 0x40 || offset > MAX_STANDARD_CAPABILITY_OFFSET || offset & 3 != 0 {
            return Err(ConfigError::OutOfRange);
        }

        let header = read_standard(io, address, offset)?;
        let control = (header >> 16) as u16;
        let is_64 = control & (1 << 7) != 0;
        let masking = control & (1 << 8) != 0;
        let mmc = ((control >> 1) & 0x7) as u8;
        let mme = ((control >> 4) & 0x7) as u8;
        if mme > mmc { return Err(ConfigError::OutOfRange); }
        let address_low = read_standard(io, address, offset + 4)?;
        let (message_address, data_register) = if is_64 {
            let address_high = read_standard(io, address, offset + 8)?;
            (((address_high as u64) << 32) | address_low as u64, offset + 12)
        } else {
            (address_low as u64, offset + 8)
        };
        let message_data = read_standard(io, address, data_register)? as u16;

        let (mask_bits, pending_bits) = if masking {
            let mask = read_standard(io, address, data_register + 4)?;
            let pending = read_standard(io, address, data_register + 8)?;
            (Some(mask), Some(pending))
        } else {
            (None, None)
        };

        Ok(Self {
            offset: capability.offset,
            enabled: control & 1 != 0,
            supports_64_bit_address: is_64,
            per_vector_masking: masking,
            maximum_vectors: 1u8 << mmc,
            enabled_vectors: 1u8 << mme,
            message_address,
            message_data,
            mask_bits,
            pending_bits,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MsixCapability {
    pub offset: u8,
    pub enabled: bool,
    pub function_masked: bool,
    pub table_size: u16,
    pub table_bar: u8,
    pub table_offset: u32,
    pub pending_bit_array_bar: u8,
    pub pending_bit_array_offset: u32,
}

impl MsixCapability {
    /// Read the MSI-X capability registers without enabling or mapping them.
    pub fn read<A: ConfigAccess>(
        io: &A,
        address: PciAddress,
        capability: Capability,
    ) -> Result<Self, ConfigError> {
        if capability.id != MSIX_CAPABILITY_ID {
            return Err(ConfigError::OutOfRange);
        }
        let offset = capability.offset as u16;
        if offset < 0x40
            || offset > MAX_STANDARD_CAPABILITY_OFFSET
            || offset & 3 != 0
            || offset + 8 >= PCI_CONFIG_SIZE
        {
            return Err(ConfigError::OutOfRange);
        }

        let header = read_standard(io, address, offset)?;
        let control = (header >> 16) as u16;
        let table = read_standard(io, address, offset + 4)?;
        let pba = read_standard(io, address, offset + 8)?;
        let table_bar = (table & 0x7) as u8;
        let pba_bar = (pba & 0x7) as u8;
        let table_offset = table & !0x7;
        let pba_offset = pba & !0x7;

        if table_bar >= 6 || pba_bar >= 6 || table_offset & 7 != 0 || pba_offset & 7 != 0 {
            return Err(ConfigError::InvalidBar);
        }

        Ok(Self {
            offset: capability.offset,
            enabled: control & (1 << 15) != 0,
            function_masked: control & (1 << 14) != 0,
            table_size: (control & 0x07ff) + 1,
            table_bar,
            table_offset,
            pending_bit_array_bar: pba_bar,
            pending_bit_array_offset: pba_offset,
        })
    }

    /// Ensure both MSI-X structures refer to BAR slots implemented by this
    /// function. This does not prove that either BAR is memory-backed or mapped.
    pub const fn validate_bar_indices(self, identity: PciIdentity) -> Result<(), ConfigError> {
        let count = identity.bar_register_count();
        if self.table_bar >= count || self.pending_bit_array_bar >= count {
            Err(ConfigError::InvalidBar)
        } else {
            Ok(())
        }
    }

    /// Validate that both BIR values resolve to decoded memory BARs.
    /// Pass the output of `read_bars`; 64-bit BAR upper slots are not accepted
    /// as independent resources because they are not emitted as separate entries.
    pub fn validate_bars(self, bars: &[Bar]) -> Result<(), ConfigError> {
        let table = bars.iter().find(|bar| bar.index == self.table_bar)
            .ok_or(ConfigError::InvalidBar)?;
        let pba = bars.iter().find(|bar| bar.index == self.pending_bit_array_bar)
            .ok_or(ConfigError::InvalidBar)?;
        if table.kind == BarKind::Io || pba.kind == BarKind::Io {
            return Err(ConfigError::InvalidBar);
        }
        Ok(())
    }

    /// Validate that the complete MSI-X table and PBA fit inside their BAR apertures.
    /// This requires sizes obtained from a trusted BAR sizing phase.
    pub fn validate_resources(self, resources: &[BarResource]) -> Result<(), ConfigError> {
        let table = resources.iter().find(|resource| resource.index == self.table_bar)
            .ok_or(ConfigError::InvalidBar)?;
        let pba = resources.iter().find(|resource| resource.index == self.pending_bit_array_bar)
            .ok_or(ConfigError::InvalidBar)?;
        if table.kind == BarKind::Io || pba.kind == BarKind::Io {
            return Err(ConfigError::InvalidBar);
        }
        let table_end = self.table_offset.checked_add(self.table_bytes().ok_or(ConfigError::InvalidBar)?)
            .ok_or(ConfigError::InvalidBar)?;
        let pba_end = self.pending_bit_array_offset
            .checked_add(self.pending_bit_array_bytes().ok_or(ConfigError::InvalidBar)?)
            .ok_or(ConfigError::InvalidBar)?;
        if table_end as u64 > table.size || pba_end as u64 > pba.size {
            return Err(ConfigError::InvalidBar);
        }
        Ok(())
    }

    /// Required byte length of the MSI-X table (16 bytes per vector).
    pub fn table_bytes(self) -> Option<u32> {
        (self.table_size as u32).checked_mul(16)
    }

    /// Required byte length of the pending-bit array (one bit per vector).
    pub fn pending_bit_array_bytes(self) -> Option<u32> {
        (self.table_size as u32).checked_add(63)
            .and_then(|rounded| (rounded / 64).checked_mul(8))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockConfig {
        registers: [u32; 1024],
    }

    impl ConfigAccess for MockConfig {
        fn read_u32(&self, _address: PciAddress, register: u16) -> Result<u32, ConfigError> {
            self.registers.get((register / 4) as usize).copied().ok_or(ConfigError::OutOfRange)
        }
        fn write_u32(&self, _address: PciAddress, _register: u16, _value: u32) -> Result<(), ConfigError> {
            Ok(())
        }
    }

    fn address() -> PciAddress {
        PciAddress { segment: 0, bus: 0, device: 0, function: 0 }
    }

    #[test]
    fn decodes_64_bit_msi_and_mask_registers() {
        let mut registers = [0u32; 1024];
        registers[0x50 / 4] = 0x0185_0005; // MSI, enabled, 64-bit, masking, MMC=2.
        registers[0x54 / 4] = 0xfee0_0000;
        registers[0x58 / 4] = 0;
        registers[0x5c / 4] = 0x0040;
        registers[0x60 / 4] = 0x0000_0003;
        registers[0x64 / 4] = 0x0000_0001;
        let io = MockConfig { registers };
        let msi = MsiCapability::read(&io, address(), Capability { id: MSI_CAPABILITY_ID, offset: 0x50 }).unwrap();
        assert!(msi.enabled);
        assert!(msi.supports_64_bit_address);
        assert!(msi.per_vector_masking);
        assert_eq!(msi.maximum_vectors, 4);
        assert_eq!(msi.message_address, 0xfee0_0000);
        assert_eq!(msi.message_data, 0x40);
        assert_eq!(msi.mask_bits, Some(3));
        assert_eq!(msi.pending_bits, Some(1));
    }

    #[test]
    fn decodes_msix_table_and_pending_array() {
        let mut registers = [0u32; 1024];
        registers[0x60 / 4] = 0xc007_0000; // Enabled, function masked, 8 vectors.
        registers[0x64 / 4] = 0x0000_2002; // BAR 2, offset 0x2000.
        registers[0x68 / 4] = 0x0000_3004; // BAR 4, offset 0x3000.
        let io = MockConfig { registers };
        let msix = MsixCapability::read(&io, address(), Capability { id: MSIX_CAPABILITY_ID, offset: 0x60 }).unwrap();
        assert!(msix.enabled);
        assert!(msix.function_masked);
        assert_eq!(msix.table_size, 8);
        assert_eq!(msix.table_bar, 2);
        assert_eq!(msix.table_offset, 0x2000);
        assert_eq!(msix.pending_bit_array_bar, 4);
        assert_eq!(msix.pending_bit_array_offset, 0x3000);
        assert_eq!(msix.table_bytes(), Some(128));
        assert_eq!(msix.pending_bit_array_bytes(), Some(8));
    }

    #[test]
    fn validates_msix_structures_against_bar_aperture_sizes() {
        let msix = MsixCapability {
            offset: 0x60, enabled: false, function_masked: true, table_size: 8,
            table_bar: 2, table_offset: 0x2000,
            pending_bit_array_bar: 4, pending_bit_array_offset: 0x3000,
        };
        let table_bar = Bar {
            index: 2, kind: BarKind::Memory32, prefetchable: false,
            base: 0x100000, raw_low: 0x100000, raw_high: None,
        };
        let pba_bar = Bar {
            index: 4, kind: BarKind::Memory32, prefetchable: false,
            base: 0x200000, raw_low: 0x200000, raw_high: None,
        };
        let table = BarResource::new(table_bar, 0x4000).unwrap();
        let pba = BarResource::new(pba_bar, 0x4000).unwrap();
        assert_eq!(msix.validate_resources(&[table, pba]), Ok(()));

        let undersized_table = BarResource::new(table_bar, 0x2000).unwrap();
        assert_eq!(msix.validate_resources(&[undersized_table, pba]), Err(ConfigError::InvalidBar));
    }

    #[test]
    fn rejects_msix_capability_whose_registers_cross_config_boundary() {
        let mut registers = [0u32; 1024];
        registers[0xf8 / 4] = 0x0001_0011;
        let io = MockConfig { registers };
        assert_eq!(
            MsixCapability::read(&io, address(), Capability { id: MSIX_CAPABILITY_ID, offset: 0xf8 }),
            Err(ConfigError::OutOfRange)
        );
    }

    #[test]
    fn refuses_to_read_past_conventional_configuration_space() {
        let mut registers = [0u32; 1024];
        registers[0xfc / 4] = 0x0081_0005;
        let io = MockConfig { registers };
        assert_eq!(
            MsiCapability::read(&io, address(), Capability { id: MSI_CAPABILITY_ID, offset: 0xfc }),
            Err(ConfigError::OutOfRange)
        );
        assert_eq!(
            MsixCapability::read(&io, address(), Capability { id: MSIX_CAPABILITY_ID, offset: 0xfc }),
            Err(ConfigError::OutOfRange)
        );
    }

    #[test]
    fn rejects_wrong_capability_and_invalid_bar_index() {
        let io = MockConfig { registers: [0; 1024] };
        assert_eq!(
            MsiCapability::read(&io, address(), Capability { id: MSIX_CAPABILITY_ID, offset: 0x40 }),
            Err(ConfigError::OutOfRange)
        );
        let msix = MsixCapability {
            offset: 0x40, enabled: false, function_masked: true, table_size: 1,
            table_bar: 5, table_offset: 0, pending_bit_array_bar: 4, pending_bit_array_offset: 0,
        };
        let identity = PciIdentity {
            vendor_id: 1, device_id: 2, command: 0, status: 0, revision: 0,
            programming_interface: 0, subclass: 0, class_code: 0, header_type: 0,
        };
        assert_eq!(msix.validate_bar_indices(identity), Err(ConfigError::InvalidBar));
    }
}
