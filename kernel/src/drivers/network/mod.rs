//! Network interface driver contracts and Ethernet framing primitives.
//!
//! NIC hardware queues remain separate from protocol implementations
//! (ARP, IPv4/IPv6, ICMP, UDP, TCP, DNS). Concrete hardware support is tracked
//! separately from register and descriptor groundwork.

pub mod device;
pub mod e1000;
pub mod ethernet;

pub use device::{LinkState, NetworkDevice, NetworkInterfaceInfo};
pub use ethernet::{EthernetFrame, EthernetFrameError, MacAddress};
