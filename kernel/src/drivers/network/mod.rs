//! Network interface driver contracts and Ethernet framing primitives.
//!
//! NIC hardware queues remain separate from protocol implementations
//! (ARP, IPv4/IPv6, ICMP, UDP, TCP, DNS). Concrete hardware support is tracked
//! separately from register and descriptor groundwork.

pub mod device;
pub mod dma;
pub mod e1000;
pub mod e1000_descriptor;
pub mod e1000_device;
pub mod e1000_hw;
pub mod e1000_pci;
pub mod ethernet;

pub use device::{LinkState, NetworkDevice, NetworkInterfaceInfo};
pub use dma::{DmaBuffer, DmaBufferError, DMA_BUFFER_ALIGNMENT, E1000_RX_BUFFER_MIN_SIZE};
pub use e1000_device::{E1000Device, E1000DeviceError, E1000_MAX_FRAME_SIZE};
pub use ethernet::{EthernetFrame, EthernetFrameError, MacAddress};
