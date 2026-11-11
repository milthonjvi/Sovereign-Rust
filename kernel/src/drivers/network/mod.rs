//! Network interface driver contracts and Ethernet framing primitives.
//!
//! NIC hardware queues remain separate from protocol implementations
//! (ARP, IPv4/IPv6, ICMP, UDP, TCP, DNS). No concrete NIC is supported yet.

pub mod device;
pub mod ethernet;

pub use device::{LinkState, NetworkDevice, NetworkInterfaceInfo};
pub use ethernet::{EthernetFrame, EthernetFrameError, MacAddress};
