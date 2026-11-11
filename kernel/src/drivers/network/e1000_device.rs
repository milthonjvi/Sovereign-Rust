//! Allocation-free E1000 adapter implementing the common NetworkDevice contract.
//!
//! Construction is intentionally downstream of platform setup: PCI bus mastering,
//! BAR mapping, DMA allocation/pinning, ring placement, and hardware initialization
//! must already be complete. This adapter only owns the packet datapath.

use crate::drivers::api::{BusAddress, DeviceClass, DeviceInfo, Driver, DriverError};
use super::device::{
    LinkState, NetworkDevice, NetworkInterfaceInfo, ETHERNET_DEFAULT_MTU,
};
use super::dma::DmaBuffer;
use super::e1000::{
    supports_device, RxDescriptor, TxDescriptor, PCI_VENDOR_INTEL, REG_RAL0, REG_RAH0, REG_MTA_BASE,
};
use super::e1000_descriptor::mac_address_from_registers;
use super::e1000_filter::E1000MulticastFilter;
use super::e1000_hw::{
    read_link_state, reset, E1000QueueError, E1000Registers, E1000ResetError,
    E1000QueueConfig, E1000RxQueue, E1000TxQueue,
};
use super::e1000_irq::{E1000Interrupt, E1000_ICR_LSC, E1000_ICR_TXDW};

/// Maximum Ethernet frame accepted by this non-jumbo adapter (header included,
/// FCS excluded because RCTL.SECRC strips it). Includes one 802.1Q VLAN tag.
pub const E1000_MAX_FRAME_SIZE: usize = ETHERNET_DEFAULT_MTU as usize + 18;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum E1000DeviceError {
    InvalidRing,
    InvalidBufferPool,
    OverlappingDmaPools,
    InvalidMacAddress,
    HardwareFault,
}

pub struct E1000Device<'a, 'dma, R: E1000Registers> {
    pub(super) registers: &'a mut R,
    pub(super) rx_descriptors: &'a mut [RxDescriptor],
    pub(super) tx_descriptors: &'a mut [TxDescriptor],
    pub(super) rx_buffers: &'a mut [DmaBuffer<'dma>],
    pub(super) tx_buffers: &'a mut [DmaBuffer<'dma>],
    pub(super) rx_queue: E1000RxQueue,
    pub(super) tx_queue: E1000TxQueue,
    pub(super) interface: NetworkInterfaceInfo,
    pub(super) multicast_filter: E1000MulticastFilter,
    pub(super) ready: bool,
    pub(super) stopped: bool,
    pub(super) faulted: bool,
    pub(super) dropping_rx_frame: bool,
}

impl<'a, 'dma, R: E1000Registers> E1000Device<'a, 'dma, R> {
    /// Bind preinitialized hardware queues to their CPU-visible DMA buffers.
    ///
    /// The descriptor slices must alias the exact pinned DMA ring memory that
    /// was programmed into RDBA/TDBA. Every DmaBuffer must refer to the same
    /// pinned mapping and cache-coherency domain as its device DMA address.
    ///
    /// # Safety
    /// The descriptor slices must be the live, pinned DMA rings programmed into
    /// this NIC. Every DmaBuffer must reference a valid device-visible mapping,
    /// with RX buffers writable by the device and TX buffers retained until DD.
    /// The MMIO accessor must target the same device and no other owner may
    /// mutate these rings or buffers for the lifetime of this adapter. The
    /// caller must successfully shut down the adapter before releasing any DMA
    /// mapping or dropping it; if shutdown fails, mappings must remain pinned
    /// until the platform independently proves that the NIC is quiescent. If
    /// this constructor rejects a device that was already started by
    /// e1000_hw::initialize(), the caller must invoke e1000_hw::reset() before
    /// releasing those mappings.
    pub unsafe fn new(
        registers: &'a mut R,
        queues: E1000QueueConfig,
        rx_descriptors: &'a mut [RxDescriptor],
        tx_descriptors: &'a mut [TxDescriptor],
        rx_buffers: &'a mut [DmaBuffer<'dma>],
        tx_buffers: &'a mut [DmaBuffer<'dma>],
    ) -> Result<Self, E1000DeviceError> {
        if queues.validate().is_err()
            || queues.rx.descriptor_count as usize != rx_descriptors.len()
            || queues.tx.descriptor_count as usize != tx_descriptors.len()
            || rx_descriptors.len() < 8
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
        for ring in [queues.rx, queues.tx] {
            DmaBuffer::validate_disjoint_from_range(
                rx_buffers, ring.dma_base, ring.byte_length as usize,
            ).map_err(|_| E1000DeviceError::OverlappingDmaPools)?;
            DmaBuffer::validate_disjoint_from_range(
                tx_buffers, ring.dma_base, ring.byte_length as usize,
            ).map_err(|_| E1000DeviceError::OverlappingDmaPools)?;
        }
        for (descriptor, buffer) in rx_descriptors.iter().zip(rx_buffers.iter()) {
            if descriptor.buffer_address != buffer.dma_address() {
                return Err(E1000DeviceError::InvalidBufferPool);
            }
        }
        for (index, buffer) in tx_buffers.iter().enumerate() {
            if buffer.is_empty() || buffer.len() < E1000_MAX_FRAME_SIZE {
                return Err(E1000DeviceError::InvalidBufferPool);
            }
            for other in tx_buffers.iter().skip(index + 1) {
                if ranges_overlap(buffer, other)? {
                    return Err(E1000DeviceError::OverlappingDmaPools);
                }
            }
        }
        validate_cross_pool_disjoint(rx_buffers, tx_buffers)?;
        let rx_queue = E1000RxQueue::new(rx_descriptors.len() as u16)
            .map_err(|_| E1000DeviceError::InvalidRing)?;
        let tx_queue = E1000TxQueue::new(tx_descriptors.len() as u16)
            .map_err(|_| E1000DeviceError::InvalidRing)?;
        let ral = registers.read32(REG_RAL0).map_err(|_| E1000DeviceError::HardwareFault)?;
        let rah = registers.read32(REG_RAH0).map_err(|_| E1000DeviceError::HardwareFault)?;
        let mac = mac_address_from_registers(ral, rah)
            .filter(|address| address.is_unicast())
            .ok_or(E1000DeviceError::InvalidMacAddress)?;
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
            multicast_filter: E1000MulticastFilter::empty(),
            ready: false,
            stopped: false,
            faulted: false,
            dropping_rx_frame: false,
        })
    }

    /// Refresh link status from hardware. This is bounded to one MMIO read.
    pub fn refresh_link_state(&mut self) -> Result<LinkState, DriverError> {
        if self.stopped || self.faulted {
            return Err(DriverError::NotReady);
        }
        let link = match read_link_state(self.registers) {
            Ok(link) => link,
            Err(_) => {
                self.ready = false;
                self.faulted = true;
                return Err(DriverError::HardwareFault);
            }
        };
        self.interface.link = link;
        Ok(link)
    }

    /// Replace the hardware multicast filter with the requested group addresses.
    ///
    /// The MTA image is updated without enabling promiscuous/all-multicast mode.
    /// Packets arriving during the register writes may observe an intermediate
    /// filter; this method is intended for control-plane configuration.
    pub fn set_multicast_filter(
        &mut self,
        addresses: &[super::ethernet::MacAddress],
    ) -> Result<(), DriverError> {
        if !self.ready {
            return Err(DriverError::NotReady);
        }
        let next = E1000MulticastFilter::from_addresses(addresses)
            .map_err(|_| DriverError::InvalidConfiguration)?;
        let current_registers = *self.multicast_filter.registers();
        let next_registers = *next.registers();
        let mut changed = false;
        for index in 0..next_registers.len() {
            if current_registers[index] == next_registers[index] {
                continue;
            }
            let offset = REG_MTA_BASE + index as u32 * 4;
            if self.registers.write32(offset, next_registers[index]).is_err() {
                self.ready = false;
                self.faulted = true;
                return Err(DriverError::HardwareFault);
            }
            changed = true;
        }
        if changed && self.registers.read32(super::e1000::REG_STATUS).is_err() {
            self.ready = false;
            self.faulted = true;
            return Err(DriverError::HardwareFault);
        }
        self.multicast_filter = next;
        Ok(())
    }

    /// Enable the interrupt causes configured in an installed E1000 APIC bridge.
    /// The platform must have completed MSI routing before this call.
    pub fn enable_interrupts(&mut self, interrupt: &mut E1000Interrupt) -> Result<(), DriverError> {
        if !self.ready {
            return Err(DriverError::NotReady);
        }
        interrupt.enable(self.registers).map_err(map_interrupt_error)
    }

    /// Mask all device interrupt causes. The vector can then be uninstalled
    /// after the platform confirms that no interrupt callback is in flight.
    pub fn disable_interrupts(&mut self, interrupt: &mut E1000Interrupt) -> Result<(), DriverError> {
        interrupt.disable(self.registers).map_err(map_interrupt_error)
    }

    /// Consume one coalesced APIC notification and read/clear the E1000 ICR
    /// from deferred context. Queue processing remains in receive/transmit paths.
    pub fn take_interrupt_causes(
        &mut self,
        interrupt: &E1000Interrupt,
    ) -> Result<Option<u32>, DriverError> {
        if !self.ready {
            return Err(DriverError::NotReady);
        }
        interrupt.take_causes(self.registers).map_err(map_interrupt_error)
    }

    /// Service one coalesced interrupt in deferred context. Link changes are
    /// reflected immediately and completed TX descriptors are reclaimed; RX
    /// packets remain available to receive()/receive_batch() for caller delivery.
    pub fn service_interrupt(
        &mut self,
        interrupt: &E1000Interrupt,
    ) -> Result<Option<u32>, DriverError> {
        let Some(causes) = self.take_interrupt_causes(interrupt)? else {
            return Ok(None);
        };
        if causes & E1000_ICR_LSC != 0 {
            self.refresh_link_state()?;
        }
        if causes & E1000_ICR_TXDW != 0 {
            let mut no_slots = [];
            if let Err(error) = self.tx_queue.reclaim_and_available_slots(
                self.registers,
                self.tx_descriptors,
                &mut no_slots,
            ) {
                if queue_error_is_fatal(&error) {
                    self.ready = false;
                    self.faulted = true;
                }
                return Err(map_queue_error(error));
            }
        }
        Ok(Some(causes))
    }

}

impl<R: E1000Registers> Driver for E1000Device<'_, '_, R> {
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
        if self.faulted {
            return Err(DriverError::HardwareFault);
        }
        if self.stopped {
            return Err(DriverError::NotReady);
        }
        if !self.supports(device) {
            return Err(DriverError::UnsupportedDevice);
        }
        self.refresh_link_state()?;
        self.ready = true;
        Ok(())
    }

    /// Stop the MAC and its DMA engines with the global reset sequence.
    ///
    /// A successful return is the point at which the caller may release the
    /// DMA mappings. On error, hardware quiescence is unknown and mappings must
    /// remain pinned until a platform-level reset or equivalent proof completes.
    /// The separate E1000Interrupt bridge must also be disabled and uninstalled
    /// by its owner; resetting the MAC does not update that handle's state.
    fn shutdown(&mut self) -> Result<(), DriverError> {
        if self.stopped {
            return Ok(());
        }
        self.ready = false;
        match reset(self.registers) {
            Ok(()) => {
                self.stopped = true;
                self.faulted = false;
                self.interface.link = LinkState::Down;
                Ok(())
            }
            Err(E1000ResetError::Register(_)) | Err(E1000ResetError::Timeout) => {
                self.faulted = true;
                self.interface.link = LinkState::Unknown;
                Err(DriverError::HardwareFault)
            }
        }
    }
}

impl<R: E1000Registers> NetworkDevice for E1000Device<'_, '_, R> {
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
            let polled = match self.rx_queue.poll(self.registers, self.rx_descriptors) {
                Ok(polled) => polled,
                Err(error) => {
                    if queue_error_is_fatal(&error) {
                        self.ready = false;
                        self.faulted = true;
                    }
                    return Err(map_queue_error(error));
                }
            };
            let Some((index, completion)) = polled else {
                return Ok(None);
            };

            let slot = index as usize;
            let was_fragmented = self.dropping_rx_frame;
            if !completion.end_of_packet {
                self.dropping_rx_frame = true;
            } else {
                self.dropping_rx_frame = false;
            }
            let valid = !was_fragmented
                && completion.end_of_packet
                && completion.errors == 0
                && completion.length != 0
                && completion.length as usize <= self.rx_buffers[slot].len()
                && completion.length as usize <= E1000_MAX_FRAME_SIZE;

            if !valid {
                self.recycle_rx(index)?;
                continue;
            }

            let length = completion.length as usize;
            if length > output.len() {
                self.recycle_rx(index)?;
                return Err(DriverError::InvalidConfiguration);
            }

            output[..length].copy_from_slice(&self.rx_buffers[slot].as_slice()[..length]);
            self.recycle_rx(index)?;
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
        self.transmit_with(|buffer| {
            buffer[..frame.len()].copy_from_slice(frame);
            frame.len()
        })
    }

    fn interface_info(&self) -> NetworkInterfaceInfo {
        self.interface
    }
}

fn map_interrupt_error<E>(error: super::e1000_irq::E1000InterruptError<E>) -> DriverError {
    match error {
        super::e1000_irq::E1000InterruptError::Register(_) => DriverError::HardwareFault,
        _ => DriverError::InvalidConfiguration,
    }
}

pub(super) fn queue_error_is_fatal<E>(error: &E1000QueueError<E>) -> bool {
    matches!(error, E1000QueueError::Register(_) | E1000QueueError::InvalidRing)
}

pub(super) fn map_queue_error<E>(error: E1000QueueError<E>) -> DriverError {
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
        for tx_buffer in tx {
            if ranges_overlap(rx_buffer, tx_buffer)? {
                return Err(E1000DeviceError::OverlappingDmaPools);
            }
        }
    }
    Ok(())
}

fn ranges_overlap(
    left: &DmaBuffer<'_>,
    right: &DmaBuffer<'_>,
) -> Result<bool, E1000DeviceError> {
    let left_start = left.dma_address();
    let left_end = left_start.checked_add(left.len() as u64)
        .ok_or(E1000DeviceError::InvalidBufferPool)?;
    let right_start = right.dma_address();
    let right_end = right_start.checked_add(right.len() as u64)
        .ok_or(E1000DeviceError::InvalidBufferPool)?;
    Ok(left_start < right_end && right_start < left_end)
}


#[cfg(test)]
#[path = "e1000_device_tests.rs"]
mod tests;
