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
    supports_device, RxDescriptor, TxDescriptor, PCI_VENDOR_INTEL,
};
use super::e1000_hw::{
    read_link_state, E1000QueueError, E1000Registers, E1000RxQueue, E1000TxQueue,
    REG_IMC, REG_RCTL, REG_TCTL,
};
use super::ethernet::MacAddress;

/// Maximum Ethernet frame accepted by this non-jumbo adapter (header included,
/// FCS excluded because RCTL.SECRC strips it). Includes two VLAN tags.
pub const E1000_MAX_FRAME_SIZE: usize = ETHERNET_DEFAULT_MTU as usize + 22;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum E1000DeviceError {
    InvalidRing,
    InvalidBufferPool,
    OverlappingDmaPools,
    InvalidMacAddress,
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
    ready: bool,
    dropping_rx_frame: bool,
}

impl<'a, 'dma, R: E1000Registers> E1000Device<'a, 'dma, R> {
    /// Bind preinitialized hardware queues to their CPU-visible DMA buffers.
    ///
    /// The descriptor slices must alias the exact pinned DMA ring memory that
    /// was programmed into RDBA/TDBA. Every DmaBuffer must refer to the same
    /// pinned mapping and cache-coherency domain as its device DMA address.
    pub fn new(
        registers: &'a mut R,
        rx_descriptors: &'a mut [RxDescriptor],
        tx_descriptors: &'a mut [TxDescriptor],
        rx_buffers: &'a mut [DmaBuffer<'dma>],
        tx_buffers: &'a mut [DmaBuffer<'dma>],
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
            dropping_rx_frame: false,
        })
    }

    /// Refresh link status from hardware. This is bounded to one MMIO read.
    pub fn refresh_link_state(&mut self) -> Result<LinkState, DriverError> {
        let link = read_link_state(self.registers).map_err(|_| DriverError::HardwareFault)?;
        self.interface.link = link;
        Ok(link)
    }

    /// Copy and publish several frames as one hardware batch.
    ///
    /// The slots and submissions arrays are caller-owned scratch storage and
    /// must each have at least frames.len() entries. No frame bytes are copied
    /// unless the complete batch has free TX slots. Queue access must be
    /// externally serialized, as for transmit().
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
        if frames.len() > slots.len() || frames.len() > submissions.len()
            || frames.iter().any(|frame| {
                frame.is_empty() || frame.len() > E1000_MAX_FRAME_SIZE
            })
        {
            return Err(DriverError::InvalidConfiguration);
        }
        if frames.iter().any(|frame| frame.len() > self.tx_buffers[0].len()) {
            return Err(DriverError::InvalidConfiguration);
        }

        self.reclaim_tx()?;
        let available = self.tx_queue.available_slots(
            self.registers,
            self.tx_descriptors,
            &mut slots[..frames.len()],
        ).map_err(map_queue_error)?;
        if available != frames.len() {
            return Err(DriverError::ResourceUnavailable);
        }

        for (frame_index, frame) in frames.iter().enumerate() {
            let slot = slots[frame_index] as usize;
            let buffer = &mut self.tx_buffers[slot];
            if frame.len() > buffer.len() {
                return Err(DriverError::InvalidConfiguration);
            }
            buffer.as_mut_slice()[..frame.len()].copy_from_slice(frame);
            submissions[frame_index] = (buffer.dma_address(), frame.len());
        }

        self.tx_queue.submit_batch(
            self.registers,
            self.tx_descriptors,
            &submissions[..frames.len()],
        ).map_err(map_queue_error)
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

        let received = self.rx_queue.poll_batch(
            self.registers,
            self.rx_descriptors,
            &mut completions[..limit],
        ).map_err(map_queue_error)?;
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
                Err(map_queue_error(error))
            }
        }
    }

    fn reclaim_tx(&mut self) -> Result<(), DriverError> {
        self.tx_queue
            .reap(self.registers, self.tx_descriptors)
            .map(|_| ())
            .map_err(map_queue_error)
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
            let Some((index, completion)) = self.rx_queue
                .poll(self.registers, self.rx_descriptors)
                .map_err(map_queue_error)?
            else {
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
    }

    impl E1000Registers for MockRegisters {
        type Error = ();

        fn read32(&mut self, offset: u32) -> Result<u32, Self::Error> {
            if offset == REG_STATUS {
                return Ok((1 << 1) | (2 << 6) | 1);
            }
            Ok(0)
        }

        fn write32(&mut self, offset: u32, value: u32) -> Result<(), Self::Error> {
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

    fn supported_device() -> DeviceInfo {
        DeviceInfo {
            class: DeviceClass::Network,
            vendor_id: Some(PCI_VENDOR_INTEL),
            device_id: Some(super::super::e1000::DEVICE_82540EM),
            address: BusAddress::Pci { segment: 0, bus: 0, device: 3, function: 0 },
        }
    }

    #[test]
    fn batch_transmit_copies_and_publishes_with_one_tail_write() {
        let mut registers = MockRegisters::default();
        let mut rx_descriptors = [RxDescriptor::for_buffer(0x10000); 8];
        let mut tx_descriptors = [TxDescriptor { status: TXD_STATUS_DD, ..TxDescriptor::default() }; 8];
        let mut rx_storage = [[0u8; 2048]; 8];
        let mut tx_storage = [[0u8; 2048]; 8];
        let mut rx_buffers: [_; 8] = core::array::from_fn(|i|
            DmaBuffer::new(0x10000 + i as u64 * 2048, &mut rx_storage[i]).unwrap());
        let mut tx_buffers: [_; 8] = core::array::from_fn(|i|
            DmaBuffer::new(0x20000 + i as u64 * 2048, &mut tx_storage[i]).unwrap());
        for (descriptor, buffer) in rx_descriptors.iter_mut().zip(rx_buffers.iter()) {
            descriptor.buffer_address = buffer.dma_address();
        }
        let mut device = E1000Device::new(
            &mut registers, &mut rx_descriptors, &mut tx_descriptors,
            &mut rx_buffers, &mut tx_buffers, MacAddress([2, 0, 0, 0, 0, 1]),
        ).unwrap();
        device.initialize(&supported_device()).unwrap();

        let first = [0x11u8; 64];
        let second = [0x22u8; 96];
        let frames: [&[u8]; 2] = [&first, &second];
        let mut slots = [u16::MAX; 2];
        let mut submissions = [(0u64, 0usize); 2];
        assert_eq!(device.transmit_batch(&frames, &mut slots, &mut submissions), Ok(2));
        assert_eq!(slots, [0, 1]);
        assert_eq!(device.tx_buffers[0].as_slice()[..64], first);
        assert_eq!(device.tx_buffers[1].as_slice()[..96], second);
        assert_eq!(device.registers.writes.iter().filter(|(reg, _)| *reg == REG_TDT).count(), 1);
        assert!(device.registers.writes.contains(&(REG_TDT, 2)));
    }

    #[test]
    fn batch_receive_copies_frames_and_recycles_with_one_tail_write() {
        let mut registers = MockRegisters::default();
        let mut rx_descriptors = [RxDescriptor::for_buffer(0x10000); 8];
        let tx_descriptors = [TxDescriptor { status: TXD_STATUS_DD, ..TxDescriptor::default() }; 8];
        let mut rx_storage = [[0u8; 2048]; 8];
        let mut tx_storage = [[0u8; 2048]; 8];
        rx_storage[0][..64].fill(0x31);
        rx_storage[1][..96].fill(0x42);
        for (index, descriptor) in rx_descriptors.iter_mut().enumerate().take(2) {
            descriptor.buffer_address = 0x10000 + index as u64 * 2048;
            descriptor.length = if index == 0 { 64 } else { 96 };
            descriptor.status = RXD_STATUS_DD | RXD_STATUS_EOP;
        }
        let mut rx_buffers = core::array::from_fn(|i|
            DmaBuffer::new(0x10000 + i as u64 * 2048, &mut rx_storage[i]).unwrap());
        let mut tx_buffers = core::array::from_fn(|i|
            DmaBuffer::new(0x20000 + i as u64 * 2048, &mut tx_storage[i]).unwrap());
        let mut device = E1000Device::new(
            &mut registers, &mut rx_descriptors, &mut tx_descriptors,
            &mut rx_buffers, &mut tx_buffers, MacAddress([2, 0, 0, 0, 0, 1]),
        ).unwrap();
        device.initialize(&supported_device()).unwrap();

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
        assert_eq!(device.registers.writes.iter().filter(|(reg, _)| *reg == REG_RDT).count(), 1);
        assert!(device.registers.writes.contains(&(REG_RDT, 1)));
    }
}
