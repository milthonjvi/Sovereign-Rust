//! Common network-interface contracts shared by NIC drivers.

use crate::drivers::api::{Driver, DriverError};
use super::ethernet::MacAddress;

/// Ethernet's standard minimum MTU and conventional maximum without jumbo frames.
pub const ETHERNET_MIN_MTU: u16 = 46;
pub const ETHERNET_DEFAULT_MTU: u16 = 1500;
pub const ETHERNET_MAX_STANDARD_MTU: u16 = 1500;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinkState { Down, Up { speed_mbps: Option<u32>, full_duplex: bool }, Unknown }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkInterfaceInfo { pub mac: MacAddress, pub mtu: u16, pub link: LinkState }

impl NetworkInterfaceInfo {
    pub const fn new(mac: MacAddress, mtu: u16) -> Result<Self, DriverError> {
        if mtu < ETHERNET_MIN_MTU { return Err(DriverError::InvalidConfiguration); }
        Ok(Self { mac, mtu, link: LinkState::Unknown })
    }
}

/// Hardware-facing NIC contract. Caller-owned buffers avoid imposing an allocator.
/// Implementations must be bounded and non-blocking when called from IRQ context.
pub trait NetworkDevice: Driver {
    /// Copy one received Ethernet frame into buffer; None means the queue is empty.
    fn receive(&mut self, buffer: &mut [u8]) -> Result<Option<usize>, DriverError>;
    /// Submit one complete Ethernet frame for transmission.
    fn transmit(&mut self, frame: &[u8]) -> Result<(), DriverError>;
    fn interface_info(&self) -> NetworkInterfaceInfo;
}