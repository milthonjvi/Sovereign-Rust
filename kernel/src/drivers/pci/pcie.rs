//! Read-only decoding of the PCI Express capability and negotiated link state.

use super::capabilities::Capability;
use super::config::{ConfigAccess, ConfigError, PciAddress, PCI_CONFIG_SIZE};

pub const PCIE_CAPABILITY_ID: u8 = 0x10;
const MAX_STANDARD_CAPABILITY_OFFSET: u16 = 0xec;

fn read_standard<A: ConfigAccess>(io: &A, address: PciAddress, register: u16) -> Result<u32, ConfigError> {
    if register >= PCI_CONFIG_SIZE || register & 3 != 0 {
        return Err(ConfigError::OutOfRange);
    }
    io.read_u32(address, register)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PciePortType {
    Endpoint,
    LegacyEndpoint,
    RootPort,
    UpstreamPort,
    DownstreamPort,
    PciExpressToPciBridge,
    PciToPciExpressBridge,
    RootComplexIntegratedEndpoint,
    RootComplexEventCollector,
    Unknown(u8),
}

impl PciePortType {
    const fn decode(value: u8) -> Self {
        match value {
            0 => Self::Endpoint,
            1 => Self::LegacyEndpoint,
            4 => Self::RootPort,
            5 => Self::UpstreamPort,
            6 => Self::DownstreamPort,
            7 => Self::PciExpressToPciBridge,
            8 => Self::PciToPciExpressBridge,
            9 => Self::RootComplexIntegratedEndpoint,
            10 => Self::RootComplexEventCollector,
            other => Self::Unknown(other),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PcieLinkSpeed {
    Gen1_2_5Gt,
    Gen2_5Gt,
    Gen3_8Gt,
    Gen4_16Gt,
    Gen5_32Gt,
    Gen6_64Gt,
    Unknown(u8),
}

impl PcieLinkSpeed {
    const fn decode(value: u8) -> Self {
        match value {
            1 => Self::Gen1_2_5Gt,
            2 => Self::Gen2_5Gt,
            3 => Self::Gen3_8Gt,
            4 => Self::Gen4_16Gt,
            5 => Self::Gen5_32Gt,
            6 => Self::Gen6_64Gt,
            other => Self::Unknown(other),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcieDeviceCapabilities {
    pub max_payload_bytes: u16,
    pub phantom_function_bits: u8,
    pub extended_tag_supported: bool,
    pub l0s_acceptable_latency: u8,
    pub l1_acceptable_latency: u8,
    pub role_based_error_reporting: bool,
    pub slot_power_limit_value: u8,
    pub slot_power_limit_scale: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcieLinkCapabilities {
    pub max_speed: PcieLinkSpeed,
    pub max_width: u8,
    pub aspm_support: u8,
    pub l0s_exit_latency: u8,
    pub l1_exit_latency: u8,
    pub clock_power_management: bool,
    pub surprise_down_error_reporting: bool,
    pub dll_link_active_reporting: bool,
    pub link_bandwidth_notification: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcieLinkStatus {
    pub current_speed: PcieLinkSpeed,
    pub negotiated_width: u8,
    pub link_training: bool,
    pub slot_clock_configured: bool,
    pub dll_link_active: bool,
    pub bandwidth_management_status: bool,
    pub autonomous_bandwidth_status: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcieCapability {
    pub offset: u8,
    pub version: u8,
    pub port_type: PciePortType,
    pub slot_implemented: bool,
    pub interrupt_message_number: u8,
    pub device: PcieDeviceCapabilities,
    pub link: PcieLinkCapabilities,
    pub link_status: PcieLinkStatus,
}

impl PcieCapability {
    /// Read PCIe device capabilities and current negotiated link state.
    pub fn read<A: ConfigAccess>(
        io: &A,
        address: PciAddress,
        capability: Capability,
    ) -> Result<Self, ConfigError> {
        if capability.id != PCIE_CAPABILITY_ID {
            return Err(ConfigError::OutOfRange);
        }
        let offset = capability.offset as u16;
        if offset < 0x40 || offset > MAX_STANDARD_CAPABILITY_OFFSET || offset & 3 != 0 {
            return Err(ConfigError::OutOfRange);
        }

        let header = read_standard(io, address, offset)?;
        let cap_register = (header >> 16) as u16;
        let version = (cap_register & 0xf) as u8;
        let port_type = PciePortType::decode(((cap_register >> 4) & 0xf) as u8);
        let slot_implemented = cap_register & (1 << 8) != 0;
        let interrupt_message_number = ((cap_register >> 9) & 0x1f) as u8;

        let device_raw = read_standard(io, address, offset + 4)?;
        let link_raw = read_standard(io, address, offset + 0x0c)?;
        let link_status_raw = read_standard(io, address, offset + 0x10)?;
        let max_payload_code = (device_raw & 0x7) as u8;
        if max_payload_code > 5 {
            return Err(ConfigError::OutOfRange);
        }
        let max_payload_bytes = 128u16.checked_shl(max_payload_code as u32)
            .ok_or(ConfigError::OutOfRange)?;

        Ok(Self {
            offset: capability.offset,
            version,
            port_type,
            slot_implemented,
            interrupt_message_number,
            device: PcieDeviceCapabilities {
                max_payload_bytes,
                phantom_function_bits: ((device_raw >> 3) & 0x3) as u8,
                extended_tag_supported: device_raw & (1 << 5) != 0,
                l0s_acceptable_latency: ((device_raw >> 6) & 0x7) as u8,
                l1_acceptable_latency: ((device_raw >> 9) & 0x7) as u8,
                role_based_error_reporting: device_raw & (1 << 15) != 0,
                slot_power_limit_value: ((device_raw >> 18) & 0xff) as u8,
                slot_power_limit_scale: ((device_raw >> 26) & 0x3) as u8,
            },
            link: PcieLinkCapabilities {
                max_speed: PcieLinkSpeed::decode((link_raw & 0xf) as u8),
                max_width: ((link_raw >> 4) & 0x3f) as u8,
                aspm_support: ((link_raw >> 10) & 0x3) as u8,
                l0s_exit_latency: ((link_raw >> 12) & 0x7) as u8,
                l1_exit_latency: ((link_raw >> 15) & 0x7) as u8,
                clock_power_management: link_raw & (1 << 18) != 0,
                surprise_down_error_reporting: link_raw & (1 << 19) != 0,
                dll_link_active_reporting: link_raw & (1 << 20) != 0,
                link_bandwidth_notification: link_raw & (1 << 21) != 0,
            },
            link_status: PcieLinkStatus {
                current_speed: PcieLinkSpeed::decode(((link_status_raw >> 16) & 0xf) as u8),
                negotiated_width: ((link_status_raw >> 20) & 0x3f) as u8,
                link_training: link_status_raw & (1 << 27) != 0,
                slot_clock_configured: link_status_raw & (1 << 28) != 0,
                dll_link_active: link_status_raw & (1 << 29) != 0,
                bandwidth_management_status: link_status_raw & (1 << 30) != 0,
                autonomous_bandwidth_status: link_status_raw & (1 << 31) != 0,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockConfig([u32; 64]);
    impl ConfigAccess for MockConfig {
        fn read_u32(&self, _address: PciAddress, register: u16) -> Result<u32, ConfigError> {
            self.0.get((register / 4) as usize).copied().ok_or(ConfigError::OutOfRange)
        }
        fn write_u32(&self, _address: PciAddress, _register: u16, _value: u32) -> Result<(), ConfigError> {
            Err(ConfigError::OutOfRange)
        }
    }

    fn address() -> PciAddress {
        PciAddress { segment: 0, bus: 0, device: 0, function: 0 }
    }

    #[test]
    fn decodes_root_port_and_negotiated_gen3_x8_link() {
        let mut registers = [0u32; 64];
        registers[0x40 / 4] = 0x0742_0010; // v2, root port, slot, INT message 3.
        registers[0x44 / 4] = 0x0000_8022; // 512-byte MPS, extended tags, RBER.
        registers[0x4c / 4] = 0x003d_3d03; // Gen3 x16 max, ASPM, latencies and link reporting.
        registers[0x50 / 4] = 0x3083_0000; // Gen3 x8, DLL active and slot clock.
        let io = MockConfig(registers);
        let pcie = PcieCapability::read(&io, address(), Capability { id: PCIE_CAPABILITY_ID, offset: 0x40 }).unwrap();
        assert_eq!(pcie.version, 2);
        assert_eq!(pcie.port_type, PciePortType::RootPort);
        assert!(pcie.slot_implemented);
        assert_eq!(pcie.interrupt_message_number, 3);
        assert_eq!(pcie.device.max_payload_bytes, 512);
        assert!(pcie.device.extended_tag_supported);
        assert!(pcie.device.role_based_error_reporting);
        assert_eq!(pcie.link.max_speed, PcieLinkSpeed::Gen3_8Gt);
        assert_eq!(pcie.link.max_width, 16);
        assert_eq!(pcie.link_status.current_speed, PcieLinkSpeed::Gen3_8Gt);
        assert_eq!(pcie.link_status.negotiated_width, 8);
        assert!(pcie.link_status.dll_link_active);
        assert!(pcie.link_status.slot_clock_configured);
    }

    #[test]
    fn rejects_wrong_capability_and_offsets_past_standard_config_space() {
        let io = MockConfig([0; 64]);
        assert_eq!(PcieCapability::read(&io, address(), Capability { id: 0x11, offset: 0x40 }), Err(ConfigError::OutOfRange));
        assert_eq!(PcieCapability::read(&io, address(), Capability { id: PCIE_CAPABILITY_ID, offset: 0xf0 }), Err(ConfigError::OutOfRange));
    }
}
