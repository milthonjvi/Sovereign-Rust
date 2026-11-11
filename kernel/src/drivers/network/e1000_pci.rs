//! Read-only binding between PCI enumeration and the Intel E1000 NIC driver.
//!
//! This module consumes PCI identities and decoded BAR values. It does not
//! perform configuration writes, BAR sizing, MMIO mapping, or bus-master enable.

use crate::drivers::pci::{Bar, BarKind, PciAddress, PciFunction};
use super::e1000::{supports_device, DEVICE_82540EM, DEVICE_82545EM, DEVICE_82574L, PCI_VENDOR_INTEL};

pub const PCI_COMMAND_MEMORY_SPACE: u16 = 1 << 1;
pub const PCI_COMMAND_BUS_MASTER: u16 = 1 << 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum E1000PciError {
    UnsupportedDevice,
    MissingMmioBar,
    InvalidMmioBar,
    MemorySpaceDisabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum E1000Model {
    I82540Em,
    I82545Em,
    I82574L,
}

impl E1000Model {
    pub const fn from_device_id(device_id: u16) -> Option<Self> {
        match device_id {
            DEVICE_82540EM => Some(Self::I82540Em),
            DEVICE_82545EM => Some(Self::I82545Em),
            DEVICE_82574L => Some(Self::I82574L),
            _ => None,
        }
    }
}

/// Read-only information captured from PCI enumeration for one supported NIC.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct E1000PciDevice {
    pub address: PciAddress,
    pub model: E1000Model,
    pub memory_space_enabled: bool,
    pub bus_master_enabled: bool,
}

impl E1000PciDevice {
    pub fn from_function(function: &PciFunction) -> Result<Self, E1000PciError> {
        let identity = function.identity;
        if identity.class_code != 0x02 || identity.subclass != 0x00
            || !supports_device(identity.vendor_id, identity.device_id)
        {
            return Err(E1000PciError::UnsupportedDevice);
        }
        let model = E1000Model::from_device_id(identity.device_id)
            .ok_or(E1000PciError::UnsupportedDevice)?;
        Ok(Self {
            address: function.address,
            model,
            memory_space_enabled: identity.command & PCI_COMMAND_MEMORY_SPACE != 0,
            bus_master_enabled: identity.command & PCI_COMMAND_BUS_MASTER != 0,
        })
    }

    /// Select the E1000's MMIO register BAR from caller-supplied decoded BARs.
    /// The PCI workstream remains responsible for reading/decoding BARs.
    pub fn mmio_bar(&self, bars: &[Bar]) -> Result<Bar, E1000PciError> {
        if !self.memory_space_enabled {
            return Err(E1000PciError::MemorySpaceDisabled);
        }
        let bar0 = bars.iter().copied()
            .find(|bar| bar.index == 0)
            .ok_or(E1000PciError::MissingMmioBar)?;
        if bar0.kind == BarKind::Io || bar0.base == 0 || bar0.prefetchable {
            return Err(E1000PciError::InvalidMmioBar);
        }
        Ok(bar0)
    }
}

/// True only for the Intel vendor ID; exposed for diagnostics and tests.
pub const fn is_intel_e1000_vendor(vendor_id: u16) -> bool {
    vendor_id == PCI_VENDOR_INTEL
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drivers::pci::PciIdentity;

    fn function(vendor_id: u16, device_id: u16, command: u16) -> PciFunction {
        PciFunction {
            address: PciAddress { segment: 0, bus: 0, device: 3, function: 0 },
            identity: PciIdentity {
                vendor_id, device_id, command, status: 0, revision: 0,
                programming_interface: 0, subclass: 0, class_code: 2, header_type: 0,
            },
        }
    }

    #[test]
    fn matches_supported_intel_models_and_bus_master_state() {
        let nic = E1000PciDevice::from_function(&function(0x8086, DEVICE_82540EM, PCI_COMMAND_BUS_MASTER | PCI_COMMAND_MEMORY_SPACE)).unwrap();
        assert_eq!(nic.model, E1000Model::I82540Em);
        assert!(nic.bus_master_enabled);
        assert!(nic.memory_space_enabled);
        assert_eq!(E1000PciDevice::from_function(&function(0x1234, DEVICE_82540EM, 0)),
            Err(E1000PciError::UnsupportedDevice));
        assert_eq!(E1000PciDevice::from_function(&function(0x8086, 0xffff, 0)),
            Err(E1000PciError::UnsupportedDevice));
    }

    #[test]
    fn requires_pci_memory_space_decoding_before_mmio_bar_use() {
        let nic = E1000PciDevice::from_function(&function(0x8086, DEVICE_82540EM, 0)).unwrap();
        let bars = [Bar { index: 0, kind: BarKind::Memory32, prefetchable: false, base: 0xfebc0000, raw_low: 0xfebc0000, raw_high: None }];
        assert_eq!(nic.mmio_bar(&bars), Err(E1000PciError::MemorySpaceDisabled));
    }

    #[test]
    fn rejects_supported_device_id_outside_ethernet_class() {
        let mut function = function(0x8086, DEVICE_82540EM, 0);
        function.identity.class_code = 0x03;
        assert_eq!(E1000PciDevice::from_function(&function), Err(E1000PciError::UnsupportedDevice));
    }

    #[test]
    fn selects_a_decoded_mmio_bar_without_touching_pci_configuration() {
        let nic = E1000PciDevice::from_function(&function(0x8086, DEVICE_82574L, PCI_COMMAND_MEMORY_SPACE)).unwrap();
        let bars = [
            Bar { index: 1, kind: BarKind::Io, prefetchable: false, base: 0xc000, raw_low: 0xc001, raw_high: None },
            Bar { index: 0, kind: BarKind::Memory32, prefetchable: false, base: 0xfebc0000, raw_low: 0xfebc0000, raw_high: None },
        ];
        assert_eq!(nic.mmio_bar(&bars).unwrap().index, 0);
    }

    #[test]
    fn does_not_accept_a_nonzero_memory_bar_instead_of_bar0() {
        let nic = E1000PciDevice::from_function(&function(0x8086, DEVICE_82540EM, PCI_COMMAND_MEMORY_SPACE)).unwrap();
        let bars = [Bar { index: 1, kind: BarKind::Memory32, prefetchable: false, base: 0xfebc0000, raw_low: 0xfebc0000, raw_high: None }];
        assert_eq!(nic.mmio_bar(&bars), Err(E1000PciError::MissingMmioBar));
    }

    #[test]
    fn rejects_unassigned_or_missing_mmio_bars() {
        let nic = E1000PciDevice::from_function(&function(0x8086, DEVICE_82545EM, PCI_COMMAND_MEMORY_SPACE)).unwrap();
        let bars = [Bar { index: 0, kind: BarKind::Memory32, prefetchable: false, base: 0, raw_low: 0, raw_high: None }];
        assert_eq!(nic.mmio_bar(&bars), Err(E1000PciError::InvalidMmioBar));
    }

    #[test]
    fn rejects_prefetchable_register_bar() {
        let nic = E1000PciDevice::from_function(&function(0x8086, DEVICE_82540EM, 0)).unwrap();
        let bars = [Bar { index: 0, kind: BarKind::Memory32, prefetchable: true, base: 0xfebc0000, raw_low: 0xfebc0008, raw_high: None }];
        assert_eq!(nic.mmio_bar(&bars), Err(E1000PciError::InvalidMmioBar));
    }
}
