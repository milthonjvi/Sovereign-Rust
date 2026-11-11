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
    registers: &'a mut R,
    rx_descriptors: &'a mut [RxDescriptor],
    tx_descriptors: &'a mut [TxDescriptor],
    rx_buffers: &'a mut [DmaBuffer<'dma>],
    tx_buffers: &'a mut [DmaBuffer<'dma>],
    rx_queue: E1000RxQueue,
    tx_queue: E1000TxQueue,
    interface: NetworkInterfaceInfo,
    multicast_filter: E1000MulticastFilter,
    ready: bool,
    stopped: bool,
    faulted: bool,
    dropping_rx_frame: bool,
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
        let current_registers = self.multicast_filter.registers();
        let next_registers = next.registers();
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

    /// Fill an available DMA TX buffer directly, avoiding an intermediate frame copy.
    ///
    /// The closure receives the selected buffer and returns the number of bytes
    /// written. It must not retain the buffer borrow or access the NIC reentrantly.
    pub fn transmit_with<F>(&mut self, fill: F) -> Result<(), DriverError>
    where
        F: FnOnce(&mut [u8]) -> usize,
    {
        if !self.ready {
            return Err(DriverError::NotReady);
        }

        let mut available = [u16::MAX; 1];
        let count = match self.tx_queue.reclaim_and_available_slots(
            self.registers,
            self.tx_descriptors,
            &mut available,
        ) {
            Ok(count) => count,
            Err(error) => {
                if queue_error_is_fatal(&error) {
                    self.ready = false;
                    self.faulted = true;
                }
                return Err(map_queue_error(error));
            }
        };
        if count == 0 {
            return Err(DriverError::ResourceUnavailable);
        }

        let index = available[0] as usize;
        let buffer = &mut self.tx_buffers[index];
        let length = fill(buffer.as_mut_slice());
        if length == 0 || length > E1000_MAX_FRAME_SIZE || length > buffer.len() {
            return Err(DriverError::InvalidConfiguration);
        }
        if let Err(error) = self.tx_queue.submit(
            self.registers,
            self.tx_descriptors,
            buffer.dma_address(),
            length,
        ) {
            if queue_error_is_fatal(&error) {
                self.ready = false;
                self.faulted = true;
            }
            return Err(map_queue_error(error));
        }
        Ok(())
    }

    /// Process completed RX frames directly from their DMA buffers without copying.
    ///
    /// The callback runs while the current buffer is CPU-owned. It must be bounded
    /// and nonblocking; its slice cannot outlive the call. The descriptor is recycled
    /// immediately after the callback returns. At most one complete RX ring is
    /// examined per invocation.
    pub fn receive_with<F>(&mut self, mut consume: F) -> Result<usize, DriverError>
    where
        F: FnMut(&[u8]),
    {
        if !self.ready {
            return Err(DriverError::NotReady);
        }

        let mut delivered = 0usize;
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
                break;
            };

            let slot = index as usize;
            let was_fragmented = self.dropping_rx_frame;
            if !completion.end_of_packet {
                self.dropping_rx_frame = true;
            } else {
                self.dropping_rx_frame = false;
            }

            let length = completion.length as usize;
            let valid = !was_fragmented
                && completion.end_of_packet
                && completion.errors == 0
                && length != 0
                && length <= E1000_MAX_FRAME_SIZE
                && length <= self.rx_buffers[slot].len();
            if valid {
                consume(&self.rx_buffers[slot].as_slice()[..length]);
                delivered += 1;
            }
            self.recycle_rx(index)?;
        }
        Ok(delivered)
    }

    /// Copy and publish several frames as one hardware batch.
    ///
    /// The slots and submissions arrays are caller-owned scratch storage and
    /// must each have at least frames.len() entries. No frame bytes are copied
    /// unless the complete batch has free TX slots. Queue access must be
    /// externally serialized, as for transmit().
    /// Build and publish a TX batch directly in its DMA buffers.
    ///
    /// The builder is called only after the complete batch has available ring
    /// slots. If any builder rejects a frame, no descriptor in the batch is
    /// published. The callback must be bounded and must not re-enter this device.
    pub fn transmit_batch_with<F>(
        &mut self,
        count: usize,
        slots: &mut [u16],
        submissions: &mut [(u64, usize)],
        mut fill: F,
    ) -> Result<usize, DriverError>
    where
        F: FnMut(usize, &mut [u8]) -> Result<usize, DriverError>,
    {
        if !self.ready {
            return Err(DriverError::NotReady);
        }
        if count == 0 {
            return Ok(0);
        }
        if count > self.tx_descriptors.len() - 1 {
            return Err(DriverError::ResourceUnavailable);
        }
        if count > slots.len() || count > submissions.len() {
            return Err(DriverError::InvalidConfiguration);
        }

        let available = match self.tx_queue.reclaim_and_available_slots(
            self.registers,
            self.tx_descriptors,
            &mut slots[..count],
        ) {
            Ok(available) => available,
            Err(error) => {
                if queue_error_is_fatal(&error) {
                    self.ready = false;
                    self.faulted = true;
                }
                return Err(map_queue_error(error));
            }
        };
        if available != count {
            return Err(DriverError::ResourceUnavailable);
        }

        for slot in slots[..count].iter().copied() {
            let index = slot as usize;
            if index >= self.tx_buffers.len() {
                self.ready = false;
                self.faulted = true;
                return Err(DriverError::HardwareFault);
            }
            if self.tx_buffers[index].len() < E1000_MAX_FRAME_SIZE {
                return Err(DriverError::InvalidConfiguration);
            }
        }

        for index in 0..count {
            let slot = slots[index] as usize;
            let buffer = &mut self.tx_buffers[slot];
            let length = fill(index, buffer.as_mut_slice())?;
            if length == 0 || length > E1000_MAX_FRAME_SIZE || length > buffer.len() {
                return Err(DriverError::InvalidConfiguration);
            }
            submissions[index] = (buffer.dma_address(), length);
        }

        match self.tx_queue.submit_batch(
            self.registers,
            self.tx_descriptors,
            &submissions[..count],
        ) {
            Ok(submitted) => Ok(submitted),
            Err(error) => {
                if queue_error_is_fatal(&error) {
                    self.ready = false;
                    self.faulted = true;
                }
                Err(map_queue_error(error))
            }
        }
    }

    /// Copy and publish several frames as one hardware batch.
    ///
    /// The slots and submissions arguments are caller-owned scratch arrays.
    /// The complete batch is preflighted before any frame is copied into DMA.
    pub fn transmit_batch(
        &mut self,
        frames: &[&[u8]],
        slots: &mut [u16],
        submissions: &mut [(u64, usize)],
    ) -> Result<usize, DriverError> {
        if !self.ready {
            return Err(DriverError::NotReady);
        }
        if frames.is_empty() {
            return Ok(0);
        }
        if frames.len() > self.tx_descriptors.len() - 1 {
            return Err(DriverError::ResourceUnavailable);
        }
        if frames.iter().any(|frame| {
            frame.is_empty() || frame.len() > E1000_MAX_FRAME_SIZE
        }) {
            return Err(DriverError::InvalidConfiguration);
        }

        self.transmit_batch_with(
            frames.len(),
            slots,
            submissions,
            |index, buffer| {
                let frame = frames[index];
                buffer[..frame.len()].copy_from_slice(frame);
                Ok(frame.len())
            },
        )
    }

    /// Receive several completed Ethernet frames with one descriptor scan
    /// barrier and one RX-tail MMIO update. Invalid, fragmented, and undersized
    /// frames are dropped; valid frames are packed into the leading output slots.
    /// All scratch storage is supplied by the caller and no allocation occurs.
    pub fn receive_batch(
        &mut self,
        outputs: &mut [&mut [u8]],
        completions: &mut [(u16, super::e1000_descriptor::RxCompletion)],
        recycle_backup: &mut [RxDescriptor],
    ) -> Result<usize, DriverError> {
        if !self.ready {
            return Err(DriverError::NotReady);
        }
        let limit = core::cmp::min(outputs.len(), completions.len());
        if limit == 0 {
            return Ok(0);
        }
        if recycle_backup.len() < limit {
            return Err(DriverError::InvalidConfiguration);
        }

        let received = match self.rx_queue.poll_batch(
            self.registers,
            self.rx_descriptors,
            &mut completions[..limit],
        ) {
            Ok(received) => received,
            Err(error) => {
                if queue_error_is_fatal(&error) {
                    self.ready = false;
                    self.faulted = true;
                }
                return Err(map_queue_error(error));
            }
        };
        if received == 0 {
            return Ok(0);
        }

        let mut delivered = 0usize;
        for item in &completions[..received] {
            let (index, completion) = *item;
            let slot = index as usize;
            let was_fragmented = self.dropping_rx_frame;
            self.dropping_rx_frame = !completion.end_of_packet;
            let length = completion.length as usize;
            let valid = !was_fragmented
                && completion.end_of_packet
                && completion.errors == 0
                && length != 0
                && length <= self.rx_buffers[slot].len()
                && length <= E1000_MAX_FRAME_SIZE;
            if !valid || length > outputs[delivered].len() {
                continue;
            }

            outputs[delivered][..length]
                .copy_from_slice(&self.rx_buffers[slot].as_slice()[..length]);
            delivered += 1;
        }

        if let Err(error) = self.rx_queue.recycle_batch(
            self.registers,
            self.rx_descriptors,
            &completions[..received],
            &mut recycle_backup[..received],
        ) {
            // Fail closed: the descriptors remain CPU-owned after a failed RDT
            // write, and the caller must reinitialize the device before polling.
            self.ready = false;
            self.faulted = true;
            return Err(map_queue_error(error));
        }
        Ok(delivered)
    }

    fn recycle_rx(&mut self, index: u16) -> Result<(), DriverError> {
        match self.rx_queue.recycle(self.registers, self.rx_descriptors, index) {
            Ok(()) => Ok(()),
            Err(error) => {
                // A failed RDT write leaves software ownership unchanged. Stop
                // the adapter so a later poll cannot skip the unrecycled slot.
                self.ready = false;
                self.faulted = true;
                Err(map_queue_error(error))
            }
        }
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

fn queue_error_is_fatal<E>(error: &E1000QueueError<E>) -> bool {
    matches!(error, E1000QueueError::Register(_) | E1000QueueError::InvalidRing)
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
mod tests {
    use super::*;
    use crate::drivers::api::{DeviceClass, DeviceInfo};
    use super::super::e1000::{RXD_STATUS_DD, RXD_STATUS_EOP, TXD_STATUS_DD, REG_TDT, REG_RDT, REG_STATUS};
    use super::super::e1000_hw::E1000Registers;
    use std::vec::Vec;

    #[derive(Default)]
    struct MockRegisters {
        writes: Vec<(u32, u32)>,
        fail_register: Option<u32>,
    }

    unsafe impl E1000Registers for MockRegisters {
        type Error = ();

        fn read32(&mut self, offset: u32) -> Result<u32, Self::Error> {
            if offset == REG_STATUS {
                return Ok((1 << 1) | (2 << 6) | 1);
            }
            if offset == REG_RAL0 { return Ok(0x4433_2202); }
            if offset == REG_RAH0 { return Ok(0x8000_6655); }
            Ok(0)
        }

        fn write32(&mut self, offset: u32, value: u32) -> Result<(), Self::Error> {
            if self.fail_register == Some(offset) {
                return Err(());
            }
            self.writes.push((offset, value));
            Ok(())
        }

        fn dma_write_barrier(&mut self) {
            core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
        }

        fn dma_read_barrier(&mut self) {
            core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire);
        }
    }

    fn mock_dma_buffer<'a>(address: u64, bytes: &'a mut [u8]) -> DmaBuffer<'a> {
        DmaBuffer::new_for_test(address, bytes).unwrap()
    }

    fn test_queues() -> E1000QueueConfig {
        E1000QueueConfig::new(0x30000, 0x31000, 8, 8).unwrap()
    }

    fn supported_device() -> DeviceInfo {
        DeviceInfo {
            class: DeviceClass::Network,
            vendor_id: Some(PCI_VENDOR_INTEL),
            device_id: Some(super::super::e1000::DEVICE_82540EM),
            address: BusAddress::Pci { segment: 0, bus: 0, device: 3, function: 0 },
        }
    }

    #[test]
    fn shutdown_resets_dma_engines_and_prevents_software_reactivation() {
        let mut registers = MockRegisters::default();
        let mut rx_descriptors = [RxDescriptor::for_buffer(0x10000); 8];
        let mut tx_descriptors = [TxDescriptor { status: TXD_STATUS_DD, ..TxDescriptor::default() }; 8];
        let mut rx_storage = [[0u8; 2048]; 8];
        let mut tx_storage = [[0u8; 2048]; 8];
        let mut rx_buffers: [DmaBuffer<'_>; 8] = rx_storage.iter_mut().enumerate()
            .map(|(i, bytes)| mock_dma_buffer(0x10000 + i as u64 * 2048, bytes))
            .collect::<Vec<_>>().try_into().ok().unwrap();
        let mut tx_buffers: [DmaBuffer<'_>; 8] = tx_storage.iter_mut().enumerate()
            .map(|(i, bytes)| mock_dma_buffer(0x20000 + i as u64 * 2048, bytes))
            .collect::<Vec<_>>().try_into().ok().unwrap();
        for (descriptor, buffer) in rx_descriptors.iter_mut().zip(rx_buffers.iter()) {
            descriptor.buffer_address = buffer.dma_address();
        }
        // SAFETY: the mock backend performs no DMA; these arrays model pinned mappings.
        let mut device = unsafe { E1000Device::new(
            &mut registers, test_queues(), &mut rx_descriptors, &mut tx_descriptors,
            &mut rx_buffers, &mut tx_buffers,
        ) }.unwrap();
        device.initialize(&supported_device()).unwrap();

        device.registers.fail_register = Some(super::super::e1000::REG_CTRL);
        assert_eq!(device.shutdown(), Err(DriverError::HardwareFault));
        assert!(device.faulted);
        assert!(!device.ready);
        assert_eq!(device.interface.link, LinkState::Unknown);

        device.registers.fail_register = None;
        assert_eq!(device.shutdown(), Ok(()));
        assert!(device.stopped);
        assert!(!device.faulted);
        assert!(!device.ready);
        assert_eq!(device.interface.link, LinkState::Down);
        assert!(device.registers.writes.contains(&(super::super::e1000::REG_CTRL,
            super::super::e1000::CTRL_RST)));
        assert!(!device.registers.writes.contains(&(super::super::e1000::REG_RCTL, 0)));
        assert!(!device.registers.writes.contains(&(super::super::e1000::REG_TCTL, 0)));
        assert_eq!(device.initialize(&supported_device()), Err(DriverError::NotReady));
    }

    #[test]
    fn batch_transmit_copies_and_publishes_with_one_tail_write() {
        let mut registers = MockRegisters::default();
        let mut rx_descriptors = [RxDescriptor::for_buffer(0x10000); 8];
        let mut tx_descriptors = [TxDescriptor { status: TXD_STATUS_DD, ..TxDescriptor::default() }; 8];
        let mut rx_storage = [[0u8; 2048]; 8];
        let mut tx_storage = [[0u8; 2048]; 8];
        let mut rx_buffers: [DmaBuffer<'_>; 8] = rx_storage.iter_mut().enumerate()
            .map(|(i, bytes)| mock_dma_buffer(0x10000 + i as u64 * 2048, bytes))
            .collect::<Vec<_>>().try_into().ok().unwrap();
        let mut tx_buffers: [DmaBuffer<'_>; 8] = tx_storage.iter_mut().enumerate()
            .map(|(i, bytes)| mock_dma_buffer(0x20000 + i as u64 * 2048, bytes))
            .collect::<Vec<_>>().try_into().ok().unwrap();
        for (descriptor, buffer) in rx_descriptors.iter_mut().zip(rx_buffers.iter()) {
            descriptor.buffer_address = buffer.dma_address();
        }
        // SAFETY: the mock register backend does not perform DMA; backing arrays model
        // the pinned rings and device-visible buffers required by the constructor.
        let mut device = unsafe { E1000Device::new(
            &mut registers, test_queues(), &mut rx_descriptors, &mut tx_descriptors,
            &mut rx_buffers, &mut tx_buffers,
        ) }.unwrap();
        device.initialize(&supported_device()).unwrap();

        let first = [0x11u8; 64];
        let second = [0x22u8; 96];
        let oversized: [&[u8]; 8] = [&first; 8];
        let mut oversized_slots = [u16::MAX; 8];
        let mut oversized_submissions = [(0u64, 0usize); 8];
        assert_eq!(
            device.transmit_batch(&oversized, &mut oversized_slots, &mut oversized_submissions),
            Err(DriverError::ResourceUnavailable),
        );
        assert!(!device.registers.writes.iter().any(|(reg, _)| *reg == REG_TDT));

        let frames: [&[u8]; 2] = [&first, &second];
        let mut slots = [u16::MAX; 2];
        let mut submissions = [(0u64, 0usize); 2];
        assert_eq!(device.transmit_batch(&frames, &mut slots, &mut submissions), Ok(2));
        assert_eq!(slots, [0, 1]);
        assert_eq!(device.tx_buffers[0].as_slice()[..64], first);
        assert_eq!(device.tx_buffers[1].as_slice()[..96], second);
        assert_eq!(device.registers.writes.iter().filter(|(reg, _)| *reg == REG_TDT).count(), 1);
        assert!(device.registers.writes.contains(&(REG_TDT, 2)));

        assert_eq!(
            device.transmit_batch_with(2, &mut slots, &mut submissions, |index, buffer| {
                let length = 32 + index;
                buffer[..length].fill(0x44 + index as u8);
                Ok(length)
            }),
            Ok(2),
        );
        assert_eq!(slots, [2, 3]);
        assert_eq!(&device.tx_buffers[2].as_slice()[..32], &[0x44; 32]);
        assert_eq!(&device.tx_buffers[3].as_slice()[..33], &[0x45; 33]);
        assert!(device.registers.writes.contains(&(REG_TDT, 4)));

        let tdt_writes_before_rejected_batch = device.registers.writes
            .iter().filter(|(reg, _)| *reg == REG_TDT).count();
        assert_eq!(
            device.transmit_batch_with(2, &mut slots, &mut submissions, |index, buffer| {
                buffer[..8].fill(0xaa);
                if index == 0 {
                    Ok(8)
                } else {
                    Err(DriverError::InvalidConfiguration)
                }
            }),
            Err(DriverError::InvalidConfiguration),
        );
        assert_eq!(
            device.registers.writes.iter().filter(|(reg, _)| *reg == REG_TDT).count(),
            tdt_writes_before_rejected_batch,
        );
        assert_eq!(device.tx_descriptors[4].status, TXD_STATUS_DD);
        assert_eq!(device.tx_descriptors[5].status, TXD_STATUS_DD);

        assert_eq!(
            device.transmit_with(|buffer| {
                buffer[..16].fill(0x55);
                16
            }),
            Ok(()),
        );
        assert_eq!(&device.tx_buffers[4].as_slice()[..16], &[0x55; 16]);
        assert!(device.registers.writes.contains(&(REG_TDT, 5)));

        device.registers.fail_register = Some(REG_TDT);
        assert_eq!(device.transmit(&[0x33; 64]), Err(DriverError::HardwareFault));
        assert_eq!(device.tx_descriptors[5].status, TXD_STATUS_DD);
        assert!(!device.ready);
        assert!(device.faulted);
        assert_eq!(device.shutdown(), Ok(()));
    }

    #[test]
    fn receive_drains_fragmented_frame_before_delivering_next_packet() {
        let mut registers = MockRegisters::default();
        let mut rx_descriptors = [RxDescriptor::for_buffer(0x10000); 8];
        let mut tx_descriptors = [TxDescriptor { status: TXD_STATUS_DD, ..TxDescriptor::default() }; 8];
        let mut rx_storage = [[0u8; 2048]; 8];
        let mut tx_storage = [[0u8; 2048]; 8];
        rx_storage[2][..64].fill(0x53);
        for (index, descriptor) in rx_descriptors.iter_mut().enumerate() {
            descriptor.buffer_address = 0x10000 + index as u64 * 2048;
            if index < 3 {
                descriptor.length = if index == 0 { 2048 } else { 64 };
                descriptor.status = RXD_STATUS_DD
                    | if index == 0 { 0 } else { RXD_STATUS_EOP };
            }
        }
        let mut rx_buffers: [DmaBuffer<'_>; 8] = rx_storage.iter_mut().enumerate()
            .map(|(i, bytes)| mock_dma_buffer(0x10000 + i as u64 * 2048, bytes))
            .collect::<Vec<_>>().try_into().ok().unwrap();
        let mut tx_buffers: [DmaBuffer<'_>; 8] = tx_storage.iter_mut().enumerate()
            .map(|(i, bytes)| mock_dma_buffer(0x20000 + i as u64 * 2048, bytes))
            .collect::<Vec<_>>().try_into().ok().unwrap();
        // SAFETY: the mock backend performs no DMA; arrays model pinned mappings.
        let mut device = unsafe { E1000Device::new(
            &mut registers, test_queues(), &mut rx_descriptors, &mut tx_descriptors,
            &mut rx_buffers, &mut tx_buffers,
        ) }.unwrap();
        device.initialize(&supported_device()).unwrap();

        let mut output = [0u8; 128];
        assert_eq!(device.receive(&mut output), Ok(Some(64)));
        assert_eq!(&output[..64], &[0x53; 64]);
        assert_eq!(
            device.registers.writes.iter().filter(|(reg, _)| *reg == REG_RDT).count(),
            3,
        );
        assert!(!device.dropping_rx_frame);
    }

    #[test]
    fn batch_receive_copies_frames_and_recycles_with_one_tail_write() {
        let mut registers = MockRegisters::default();
        let mut rx_descriptors = [RxDescriptor::for_buffer(0x10000); 8];
        let mut tx_descriptors = [TxDescriptor { status: TXD_STATUS_DD, ..TxDescriptor::default() }; 8];
        let mut rx_storage = [[0u8; 2048]; 8];
        let mut tx_storage = [[0u8; 2048]; 8];
        rx_storage[0][..64].fill(0x31);
        rx_storage[1][..96].fill(0x42);
        rx_storage[2][..64].fill(0x53);
        for (index, descriptor) in rx_descriptors.iter_mut().enumerate() {
            descriptor.buffer_address = 0x10000 + index as u64 * 2048;
            if index < 3 {
                descriptor.length = match index { 0 => 64, 1 => 96, _ => 64 };
                descriptor.status = RXD_STATUS_DD | RXD_STATUS_EOP;
            }
        }
        let mut rx_buffers: [DmaBuffer<'_>; 8] = rx_storage.iter_mut().enumerate()
            .map(|(i, bytes)| mock_dma_buffer(0x10000 + i as u64 * 2048, bytes))
            .collect::<Vec<_>>().try_into().ok().unwrap();
        let mut tx_buffers: [DmaBuffer<'_>; 8] = tx_storage.iter_mut().enumerate()
            .map(|(i, bytes)| mock_dma_buffer(0x20000 + i as u64 * 2048, bytes))
            .collect::<Vec<_>>().try_into().ok().unwrap();
        // SAFETY: the mock register backend does not perform DMA; backing arrays model
        // the pinned rings and device-visible buffers required by the constructor.
        let mut device = unsafe { E1000Device::new(
            &mut registers, test_queues(), &mut rx_descriptors, &mut tx_descriptors,
            &mut rx_buffers, &mut tx_buffers,
        ) }.unwrap();
        device.initialize(&supported_device()).unwrap();

        let group = super::super::ethernet::MacAddress([0x33, 0x33, 0, 0, 0, 1]);
        let hash = E1000MulticastFilter::hash(group) as usize;
        assert_eq!(device.set_multicast_filter(&[group]), Ok(()));
        assert!(device.multicast_filter.accepts(group));
        assert_eq!(
            device.registers.writes.last(),
            Some(&(REG_MTA_BASE + (hash >> 5) as u32 * 4, 1u32 << (hash & 0x1f))),
        );
        let writes_before_invalid = device.registers.writes.len();
        assert_eq!(
            device.set_multicast_filter(&[super::super::ethernet::MacAddress([0x02, 0, 0, 0, 0, 1])]),
            Err(DriverError::InvalidConfiguration),
        );
        assert_eq!(device.registers.writes.len(), writes_before_invalid);
        assert_eq!(device.set_multicast_filter(&[]), Ok(()));
        assert!(device.multicast_filter.is_empty());

        let mut first = [0u8; 128];
        let mut second = [0u8; 128];
        let mut outputs: [&mut [u8]; 2] = [&mut first, &mut second];
        let mut completions = [(0u16, super::super::e1000_descriptor::RxCompletion {
            length: 0, end_of_packet: false, errors: 0,
        }); 2];
        let mut backup = [RxDescriptor::default(); 2];
        assert_eq!(device.receive_batch(&mut outputs, &mut completions, &mut backup), Ok(2));
        assert_eq!(&first[..64], &[0x31; 64]);
        assert_eq!(&second[..96], &[0x42; 96]);

        let mut observed = [0u8; 64];
        let mut observed_length = 0usize;
        assert_eq!(
            device.receive_with(|frame| {
                observed_length = frame.len();
                observed[..frame.len()].copy_from_slice(frame);
            }),
            Ok(1),
        );
        assert_eq!(observed_length, 64);
        assert_eq!(observed, [0x53; 64]);
        assert_eq!(device.registers.writes.iter().filter(|(reg, _)| *reg == REG_RDT).count(), 1);
        assert!(device.registers.writes.contains(&(REG_RDT, 1)));
    }
}
