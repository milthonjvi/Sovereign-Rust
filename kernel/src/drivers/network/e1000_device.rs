//! Allocation-free E1000 adapter implementing the common NetworkDevice contract.
//!
//! Construction is intentionally downstream of platform setup: PCI bus mastering,
//! BAR mapping, DMA allocation/pinning, ring placement, and hardware initialization
//! must already be complete. This adapter only owns the packet datapath.

use crate::drivers::api::{BusAddress, DeviceClass, DeviceInfo, Driver, DriverError};
use super::device::{
    LinkState, NetworkDevice, NetworkInterfaceInfo, ETHERNET_DEFAULT_MTU,
};
use super::dma::{DmaBuffer, DmaBufferError, E1000_RX_BUFFER_MIN_SIZE};
use super::e1000::{
    supports_device, RxDescriptor, TxDescriptor, PCI_VENDOR_INTEL,
    RXD_STATUS_EOP,
};
use super::e1000_hw::{
    read_link_state, E1000QueueError, E1000Registers, E1000RxQueue, E1000TxQueue,
    REG_IMC, REG_RCTL, REG_TCTL,
};
use super::ethernet::MacAddress;

/// Maximum Ethernet frame accepted by this non-jumbo adapter (header included,
/// FCS excluded because RCTL.SECRC strips it).
pub const E1000_MAX_FRAME_SIZE: usize = ETHERNET_DEFAULT_MTU as usize + 14;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum E1000DeviceError {
    InvalidRing,
    InvalidBufferPool,
    OverlappingDmaPools,
    InvalidMacAddress,
}

pub struct E1000Device<'a, R: E1000Registers> {
    registers: &'a mut R,
    rx_descriptors: &'a mut [RxDescriptor],
    tx_descriptors: &'a mut [TxDescriptor],
    rx_buffers: &'a mut [DmaBuffer<'a>],
    tx_buffers: &'a mut [DmaBuffer<'a>],
    rx_queue: E1000RxQueue,
    tx_queue: E1000TxQueue,
    interface: NetworkInterfaceInfo,
    ready: bool,
}

impl<'a, R: E1000Registers> E1000Device<'a, R> {
    /// Bind preinitialized hardware queues to their CPU-visible DMA buffers.
    ///
    /// The descriptor slices must alias the exact pinned DMA ring memory that
    /// was programmed into RDBA/TDBA. Every DmaBuffer must refer to the same
    /// pinned mapping and cache-coherency domain as its device DMA address.
    pub fn new(
        registers: &'a mut R,
        rx_descriptors: &'a mut [RxDescriptor],
        tx_descriptors: &'a mut [TxDescriptor],
        rx_buffers: &'a mut [DmaBuffer<'a>],
        tx_buffers: &'a mut [DmaBuffer<'a>],
        mac: MacAddress,
    ) -> Result<Self, E1000DeviceError> {
        if rx_descriptors.len() < 8
            || tx_descriptors.len() < 8
            || !rx_descriptors.len().is_power_of_two()
            || !tx_descriptors.len().is_power_of_two()
            || rx_descriptors.len() != rx_buffers.len()
            || tx_descriptors.len() != tx_buffers.len()
        {
            return Err(E1000DeviceError::InvalidRing);
        }
        DmaBuffer::validate_rx_pool(rx_buffers)
            .map_err(|_| E1000DeviceError::InvalidBufferPool)?;
        for buffer in tx_buffers.iter() {
            if buffer.is_empty() || buffer.len() < E1000_MAX_FRAME_SIZE {
                return Err(E1000DeviceError::InvalidBufferPool);
            }
        }
        validate_cross_pool_disjoint(rx_buffers, tx_buffers)?;
        let rx_queue = E1000RxQueue::new(rx_descriptors.len() as u16)
            .map_err(|_| E1000DeviceError::InvalidRing)?;
        let tx_queue = E1000TxQueue::new(tx_descriptors.len() as u16)
            .map_err(|_| E1000DeviceError::InvalidRing)?;
        let interface = NetworkInterfaceInfo::new(mac, ETHERNET_DEFAULT_MTU)
            .map_err(|_| E1000DeviceError::InvalidMacAddress)?;
        Ok(Self {
            registers,
            rx_descriptors,
            tx_descriptors,
            rx_buffers,
            tx_buffers,
            rx_queue,
            tx_queue,
            interface,
            ready: false,
        })
    }

    /// Refresh link status from hardware. This is bounded to one MMIO read.
    pub fn refresh_link_state(&mut self) -> Result<LinkState, DriverError> {
        let link = read_link_state(self.registers).map_err(|_| DriverError::HardwareFault)?;
        self.interface.link = link;
        Ok(link)
    }

    fn reclaim_tx(&mut self) -> Result<(), DriverError> {
        self.tx_queue
            .reap(self.registers, self.tx_descriptors)
            .map(|_| ())
            .map_err(map_queue_error)
    }
}

impl<R: E1000Registers> Driver for E1000Device<'_, R> {
    fn name(&self) -> &'static str {
        "Intel E1000"
    }

    fn supports(&self, device: &DeviceInfo) -> bool {
        device.class == DeviceClass::Network
            && device.vendor_id == Some(PCI_VENDOR_INTEL)
            && device.device_id.map(|id| supports_device(PCI_VENDOR_INTEL, id)).unwrap_or(false)
            && matches!(device.address, BusAddress::Pci { .. })
    }

    /// The hardware must have been initialized with e1000_hw::initialize()
    /// before this binding step. This method validates identity and activates
    /// the software datapath; it does not alter PCI configuration or reset NIC.
    fn initialize(&mut self, device: &DeviceInfo) -> Result<(), DriverError> {
        if !self.supports(device) {
            return Err(DriverError::UnsupportedDevice);
        }
        self.refresh_link_state()?;
        self.ready = true;
        Ok(())
    }

    fn shutdown(&mut self) -> Result<(), DriverError> {
        self.registers.write32(REG_IMC, u32::MAX)
            .map_err(|_| DriverError::HardwareFault)?;
        self.registers.write32(REG_RCTL, 0)
            .map_err(|_| DriverError::HardwareFault)?;
        self.registers.write32(REG_TCTL, 0)
            .map_err(|_| DriverError::HardwareFault)?;
        self.ready = false;
        self.interface.link = LinkState::Down;
        Ok(())
    }
}

impl<R: E1000Registers> NetworkDevice for E1000Device<'_, R> {
    /// Poll descriptors, discard errored/fragmented frames, copy one complete
    /// frame, then recycle its RX descriptor. No allocation or waiting occurs.
    fn receive(&mut self, output: &mut [u8]) -> Result<Option<usize>, DriverError> {
        if !self.ready {
            return Err(DriverError::NotReady);
        }
        if output.is_empty() {
            return Err(DriverError::InvalidConfiguration);
        }

        // A frame may span multiple legacy RX descriptors. The current
        // non-jumbo configuration uses 2 KiB buffers, but malformed or
        // unexpectedly fragmented input is drained and dropped safely.
        for _ in 0..self.rx_descriptors.len() {
            let Some((index, completion)) = self.rx_queue
                .poll(self.registers, self.rx_descriptors)
                .map_err(map_queue_error)?
            else {
                return Ok(None);
            };

            let slot = index as usize;
            let valid = completion.end_of_packet
                && completion.errors == 0
                && completion.length as usize <= self.rx_buffers[slot].len()
                && completion.length as usize <= E1000_MAX_FRAME_SIZE;

            if !valid {
                self.rx_queue.recycle(self.registers, self.rx_descriptors, index)
                    .map_err(map_queue_error)?;
                continue;
            }

            let length = completion.length as usize;
            if length > output.len() {
                self.rx_queue.recycle(self.registers, self.rx_descriptors, index)
                    .map_err(map_queue_error)?;
                return Err(DriverError::InvalidConfiguration);
            }

            output[..length].copy_from_slice(&self.rx_buffers[slot].as_slice()[..length]);
            self.rx_queue.recycle(self.registers, self.rx_descriptors, index)
                .map_err(map_queue_error)?;
            return Ok(Some(length));
        }
        Ok(None)
    }

    /// Copy a frame into the next reclaimed DMA slot and publish it to hardware.
    /// The TX buffer remains owned by the queue until its descriptor reports DD.
    fn transmit(&mut self, frame: &[u8]) -> Result<(), DriverError> {
        if !self.ready {
            return Err(DriverError::NotReady);
        }
        if frame.is_empty() || frame.len() > E1000_MAX_FRAME_SIZE {
            return Err(DriverError::InvalidConfiguration);
        }

        self.reclaim_tx()?;
        let index = self.tx_queue.next_available_slot(self.registers, self.tx_descriptors)
            .map_err(map_queue_error)?
            .ok_or(DriverError::ResourceUnavailable)? as usize;
        let buffer = &mut self.tx_buffers[index];
        if frame.len() > buffer.len() {
            return Err(DriverError::InvalidConfiguration);
        }
        buffer.as_mut_slice()[..frame.len()].copy_from_slice(frame);

        self.tx_queue.submit(
            self.registers,
            self.tx_descriptors,
            buffer.dma_address(),
            frame.len(),
        ).map_err(map_queue_error)?;
        Ok(())
    }

    fn interface_info(&self) -> NetworkInterfaceInfo {
        self.interface
    }
}

fn map_queue_error<E>(error: E1000QueueError<E>) -> DriverError {
    match error {
        E1000QueueError::TxRingFull => DriverError::ResourceUnavailable,
        E1000QueueError::InvalidFrameLength => DriverError::InvalidConfiguration,
        E1000QueueError::Register(_) | E1000QueueError::InvalidRing =>
            DriverError::HardwareFault,
    }
}

fn validate_cross_pool_disjoint(
    rx: &[DmaBuffer<'_>],
    tx: &[DmaBuffer<'_>],
) -> Result<(), E1000DeviceError> {
    for rx_buffer in rx {
        let rx_start = rx_buffer.dma_address();
        let rx_end = rx_start.checked_add(rx_buffer.len() as u64)
            .ok_or(E1000DeviceError::InvalidBufferPool)?;
        for tx_buffer in tx {
            let tx_start = tx_buffer.dma_address();
            let tx_end = tx_start.checked_add(tx_buffer.len() as u64)
                .ok_or(E1000DeviceError::InvalidBufferPool)?;
            if rx_start < tx_end && tx_start < rx_end {
                return Err(E1000DeviceError::OverlappingDmaPools);
            }
        }
    }
    Ok(())
}
