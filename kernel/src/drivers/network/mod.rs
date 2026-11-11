//! Network interface driver contracts and Ethernet framing primitives.
//!
//! NIC hardware queues remain separate from protocol implementations
//! (ARP, IPv4/IPv6, ICMP, UDP, TCP, DNS). This module includes an allocation-free
//! Intel E1000 datapath, volatile MMIO access, DMA ownership contracts, and
//! deferred APIC interrupt notification; platform mapping and DMA allocation
//! remain supplied by kernel services.

pub mod device;
pub mod dma;
pub mod e1000;
pub mod e1000_descriptor;
pub mod e1000_device;
mod e1000_device_rx;
mod e1000_device_tx;
pub mod e1000_filter;
pub mod e1000_hw;
pub mod e1000_irq;
pub mod e1000_mmio;
pub mod e1000_pci;
pub mod ethernet;

pub use device::{LinkState, NetworkDevice, NetworkInterfaceInfo};
pub use dma::{DmaBuffer, DmaBufferError, DMA_BUFFER_ALIGNMENT, E1000_RX_BUFFER_MIN_SIZE};
pub use e1000_device::{E1000Device, E1000DeviceError, E1000_MAX_FRAME_SIZE};
pub use e1000_filter::{E1000MulticastError, E1000MulticastFilter};
pub use e1000_hw::{
    reset as reset_e1000, E1000InitError, E1000InitInfo, E1000QueueConfig,
    E1000Registers, E1000ResetError,
};
pub use e1000_pci::{
    E1000Model, E1000PciDevice, E1000PciError, PCI_COMMAND_BUS_MASTER,
    PCI_COMMAND_MEMORY_SPACE,
};
pub use e1000_irq::{E1000Interrupt, E1000InterruptError, E1000_DEFAULT_INTERRUPT_MASK};
pub use e1000_mmio::{E1000MmioError, VolatileE1000Registers, E1000_MMIO_MIN_LENGTH};
pub use ethernet::{EthernetFrame, EthernetFrameError, MacAddress};
