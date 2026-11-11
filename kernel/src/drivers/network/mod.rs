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
pub mod e1000_irq;
pub mod e1000_mmio;
pub mod e1000_pci;
pub mod ethernet;

pub use device::{LinkState, NetworkDevice, NetworkInterfaceInfo};
pub use dma::{DmaBuffer, DmaBufferError, DMA_BUFFER_ALIGNMENT, E1000_RX_BUFFER_MIN_SIZE};
pub use e1000_device::{E1000Device, E1000DeviceError, E1000_MAX_FRAME_SIZE};
pub use e1000_irq::{E1000Interrupt, E1000InterruptError, E1000_DEFAULT_INTERRUPT_MASK};
pub use e1000_mmio::{E1000MmioError, VolatileE1000Registers, E1000_MMIO_MIN_LENGTH};
pub use ethernet::{EthernetFrame, EthernetFrameError, MacAddress};
