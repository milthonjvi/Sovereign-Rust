//! E1000 register-level initialization over a caller-provided MMIO window.
//!
//! The platform supplies the mapped MMIO accessor and DMA-backed descriptor
//! rings. This module never maps physical addresses, enables PCI bus mastering,
//! allocates DMA memory, or installs interrupt handlers.

use super::e1000::{
    RingConfig, RxDescriptor, TxDescriptor, RingCursor, E1000Error, TXD_STATUS_DD, CTRL_RST, DEFAULT_RX_RING_SIZE, DEFAULT_TX_RING_SIZE,
    REG_CTRL, REG_STATUS, REG_ICR, REG_IMC, REG_RAL0, REG_RAH0, REG_RCTL,
    REG_RDBAH, REG_RDBAL, REG_RDH, REG_RDLEN, REG_RDT,
    REG_TCTL, REG_TDBAH, REG_TDBAL, REG_TDH, REG_TDLEN, REG_TDT,
    RCTL_BAM, RCTL_EN, RCTL_LPE, RCTL_SECRC, TCTL_COLD_SHIFT, TCTL_CT_SHIFT, TCTL_EN, TCTL_PSP,
};
use super::e1000_descriptor::mac_address_from_registers;
use super::ethernet::MacAddress;

const RESET_POLL_LIMIT: usize = 1_000_000;
const RING_LENGTH_ALIGNMENT: u32 = 128;
const TCTL_COLLISION_THRESHOLD: u32 = 0x10;
const TCTL_COLLISION_DISTANCE: u32 = 0x40;
const STATUS_FD: u32 = 1 << 0;
const STATUS_LU: u32 = 1 << 1;
const STATUS_SPEED_MASK: u32 = 0b11 << 6;
const STATUS_SPEED_SHIFT: u32 = 6;

/// Minimal interface implemented by the platform's already-mapped MMIO window.
///
/// Implementations must perform volatile 32-bit accesses, enforce bounds, and
/// provide the architecture-required device-memory ordering.
pub trait E1000Registers {
    type Error;

    fn read32(&mut self, offset: u32) -> Result<u32, Self::Error>;
    fn write32(&mut self, offset: u32, value: u32) -> Result<(), Self::Error>;

    /// Order descriptor and buffer writes before notifying the device.
    fn dma_write_barrier(&mut self);
    /// Order device descriptor write-back before the CPU inspects it.
    fn dma_read_barrier(&mut self);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum E1000InitError<E> {
    Register(E),
    ResetTimeout,
    InvalidRingConfiguration,
    BusMasterDisabled,
    MissingMacAddress,
    InvalidDescriptorBuffers,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct E1000QueueConfig {
    pub rx: RingConfig,
    pub tx: RingConfig,
}

impl E1000QueueConfig {
    pub fn new(rx_base: u64, tx_base: u64, rx_count: usize, tx_count: usize)
        -> Result<Self, super::e1000::E1000Error>
    {
        let rx = RingConfig::new(rx_base, rx_count)?;
        let tx = RingConfig::new(tx_base, tx_count)?;
        if rx.byte_length % RING_LENGTH_ALIGNMENT != 0
            || tx.byte_length % RING_LENGTH_ALIGNMENT != 0
        {
            return Err(super::e1000::E1000Error::InvalidRingSize);
        }
        Ok(Self { rx, tx })
    }

    pub fn defaults(rx_base: u64, tx_base: u64) -> Result<Self, super::e1000::E1000Error> {
        Self::new(rx_base, tx_base, DEFAULT_RX_RING_SIZE, DEFAULT_TX_RING_SIZE)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct E1000InitInfo {
    pub mac: MacAddress,
    pub rx_descriptors: u16,
    pub tx_descriptors: u16,
}

/// Initialize one supported legacy E1000 using preallocated DMA rings.
///
/// Safety is delegated to the platform accessor and caller: the MMIO window
/// must refer to this device, RX descriptors must contain valid DMA buffer
/// addresses, RX buffers must be unique/non-overlapping and each provide at
/// least 2048 writable bytes,
/// all rings and buffers must be DMA-mapped and pinned, the descriptor slices
/// must alias the physical ring bases supplied in queues, and PCI bus mastering
/// must already be enabled. The platform must provide coherent DMA or implement
/// cache maintenance in its barrier methods.
pub fn initialize<R: E1000Registers>(
    registers: &mut R,
    queues: E1000QueueConfig,
    rx_descriptors: &mut [RxDescriptor],
    rx_buffer_dma: &[u64],
    tx_descriptors: &mut [TxDescriptor],
    bus_master_enabled: bool,
) -> Result<E1000InitInfo, E1000InitError<R::Error>> {
    if !bus_master_enabled {
        return Err(E1000InitError::BusMasterDisabled);
    }
    validate_queue(&queues).map_err(|_| E1000InitError::InvalidRingConfiguration)?;
    if rx_descriptors.len() != queues.rx.descriptor_count as usize
        || rx_buffer_dma.len() != rx_descriptors.len()
        || tx_descriptors.len() != queues.tx.descriptor_count as usize
        || rx_buffer_dma.iter().any(|address| *address == 0 || *address & 0xf != 0)
    {
        return Err(E1000InitError::InvalidDescriptorBuffers);
    }

    registers.write32(REG_IMC, u32::MAX).map_err(E1000InitError::Register)?;
    registers.write32(REG_RCTL, 0).map_err(E1000InitError::Register)?;
    registers.write32(REG_TCTL, 0).map_err(E1000InitError::Register)?;

    let ctrl = registers.read32(REG_CTRL).map_err(E1000InitError::Register)?;
    registers.write32(REG_CTRL, ctrl | CTRL_RST).map_err(E1000InitError::Register)?;

    let mut reset_complete = false;
    for _ in 0..RESET_POLL_LIMIT {
        let value = registers.read32(REG_CTRL).map_err(E1000InitError::Register)?;
        if value & CTRL_RST == 0 {
            reset_complete = true;
            break;
        }
        core::hint::spin_loop();
    }
    if !reset_complete {
        return Err(E1000InitError::ResetTimeout);
    }

    // Hardware queues are stopped and reset; now initialize the DMA-visible rings.
    for (descriptor, &buffer_dma) in rx_descriptors.iter_mut().zip(rx_buffer_dma.iter()) {
        *descriptor = RxDescriptor::for_buffer(buffer_dma);
    }
    for descriptor in tx_descriptors.iter_mut() {
        *descriptor = TxDescriptor { status: TXD_STATUS_DD, ..TxDescriptor::default() };
    }
    registers.dma_write_barrier();

    // Reset may restore interrupt masks and queue state; mask again before
    // reading the read-to-clear cause register or programming the queues.
    registers.write32(REG_IMC, u32::MAX).map_err(E1000InitError::Register)?;
    let _ = registers.read32(REG_ICR).map_err(E1000InitError::Register)?;
    registers.dma_write_barrier();

    let ral = registers.read32(REG_RAL0).map_err(E1000InitError::Register)?;
    let rah = registers.read32(REG_RAH0).map_err(E1000InitError::Register)?;
    let mac = mac_address_from_registers(ral, rah)
        .ok_or(E1000InitError::MissingMacAddress)?;

    program_ring(registers, REG_RDBAL, REG_RDBAH, REG_RDLEN, REG_RDH, REG_RDT, queues.rx)
        .map_err(E1000InitError::Register)?;
    program_ring(registers, REG_TDBAL, REG_TDBAH, REG_TDLEN, REG_TDH, REG_TDT, queues.tx)
        .map_err(E1000InitError::Register)?;

    let tctl = TCTL_EN | TCTL_PSP
        | (TCTL_COLLISION_THRESHOLD << TCTL_CT_SHIFT)
        | (TCTL_COLLISION_DISTANCE << TCTL_COLD_SHIFT);
    registers.write32(REG_TCTL, tctl).map_err(E1000InitError::Register)?;
    registers.dma_write_barrier();
    registers.write32(REG_RCTL, RCTL_EN | RCTL_LPE | RCTL_BAM | RCTL_SECRC)
        .map_err(E1000InitError::Register)?;

    Ok(E1000InitInfo {
        mac,
        rx_descriptors: queues.rx.descriptor_count,
        tx_descriptors: queues.tx.descriptor_count,
    })
}

/// Read the physical link state from the E1000 STATUS register.
pub fn read_link_state<R: E1000Registers>(
    registers: &mut R,
) -> Result<super::device::LinkState, R::Error> {
    let status = registers.read32(REG_STATUS)?;
    if status & STATUS_LU == 0 {
        return Ok(super::device::LinkState::Down);
    }
    let speed_mbps = match (status & STATUS_SPEED_MASK) >> STATUS_SPEED_SHIFT {
        0 => Some(10),
        1 => Some(100),
        2 => Some(1000),
        _ => None,
    };
    Ok(super::device::LinkState::Up {
        speed_mbps,
        full_duplex: status & STATUS_FD != 0,
    })
}

/// Runtime error returned by allocation-free RX/TX queue operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum E1000QueueError<E> {
    Register(E),
    InvalidRing,
    InvalidFrameLength,
    TxRingFull,
}

/// Single-producer/single-consumer software state for a legacy TX ring.
/// Keep one instance per hardware queue and serialize access to it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct E1000TxQueue {
    cursor: RingCursor,
    clean: RingCursor,
    pending: u16,
}

impl E1000TxQueue {
    pub fn new(descriptor_count: u16) -> Result<Self, E1000Error> {
        Ok(Self { cursor: RingCursor::new(descriptor_count)?, clean: RingCursor::new(descriptor_count)?, pending: 0 })
    }

    /// Submit one DMA-mapped frame and publish its descriptor to hardware.
    /// Frame bytes must already be present in the buffer and remain immutable
    /// until reap_one() returns the descriptor index.
    pub fn submit<R: E1000Registers>(
        &mut self,
        registers: &mut R,
        descriptors: &mut [TxDescriptor],
        buffer_dma: u64,
        length: usize,
    ) -> Result<u16, E1000QueueError<R::Error>> {
        let index = self.cursor.index();
        self.submit_batch(registers, descriptors, &[(buffer_dma, length)])?;
        Ok(index)
    }

    /// Submit a batch with one DMA barrier and one TDT write.
    /// This amortizes MMIO and ordering overhead under sustained traffic.
    pub fn submit_batch<R: E1000Registers>(
        &mut self,
        registers: &mut R,
        descriptors: &mut [TxDescriptor],
        frames: &[(u64, usize)],
    ) -> Result<usize, E1000QueueError<R::Error>> {
        if descriptors.len() != self.cursor_count() {
            return Err(E1000QueueError::InvalidRing);
        }
        if frames.is_empty() {
            return Ok(0);
        }
        if frames.iter().any(|(address, length)| {
            *address == 0 || *address & 0xf != 0 || *length == 0 || *length > u16::MAX as usize
        }) {
            return Err(E1000QueueError::InvalidFrameLength);
        }
        if frames.len() > descriptors.len() - 1 - self.pending as usize {
            return Err(E1000QueueError::TxRingFull);
        }

        let count = descriptors.len();
        let start = self.cursor.index() as usize;
        for offset in 0..frames.len() {
            let index = (start + offset) & (count - 1);
            // SAFETY: the caller provides a pinned DMA ring exclusively owned by this queue.
            let current = unsafe { core::ptr::read_volatile(descriptors.as_ptr().add(index)) };
            if current.status & TXD_STATUS_DD == 0 {
                return Err(E1000QueueError::TxRingFull);
            }
        }

        for (offset, &(buffer_dma, length)) in frames.iter().enumerate() {
            let index = (start + offset) & (count - 1);
            let descriptor = super::e1000_descriptor::TxDescriptor::for_frame(buffer_dma, length)
                .map_err(|_| E1000QueueError::InvalidFrameLength)?;
            // SAFETY: this descriptor belongs to the pinned DMA ring supplied by the caller.
            unsafe { core::ptr::write_volatile(descriptors.as_mut_ptr().add(index), descriptor); }
        }
        registers.dma_write_barrier();
        let next = (start + frames.len()) & (count - 1);
        if let Err(error) = registers.write32(REG_TDT, next as u32) {
            for offset in 0..frames.len() {
                let index = (start + offset) & (count - 1);
                // SAFETY: hardware was not notified, so these descriptors remain CPU-owned.
                unsafe {
                    let mut descriptor = core::ptr::read_volatile(descriptors.as_ptr().add(index));
                    descriptor.status = TXD_STATUS_DD;
                    core::ptr::write_volatile(descriptors.as_mut_ptr().add(index), descriptor);
                }
            }
            return Err(E1000QueueError::Register(error));
        }
        for _ in 0..frames.len() {
            self.cursor.advance();
        }
        self.pending += frames.len() as u16;
        Ok(frames.len())
    }

    /// Inspect the next producer slot without transferring ownership.
    ///
    /// The caller may write its matching DMA buffer only after this returns
    /// Some(index). The queue still validates descriptor availability again
    /// during submit(), so all accesses must remain externally serialized.
    pub fn next_available_slot<R: E1000Registers>(
        &self,
        registers: &mut R,
        descriptors: &[TxDescriptor],
    ) -> Result<Option<u16>, E1000QueueError<R::Error>> {
        if descriptors.len() != self.cursor_count() {
            return Err(E1000QueueError::InvalidRing);
        }
        if self.pending as usize >= descriptors.len() - 1 {
            return Ok(None);
        }
        registers.dma_read_barrier();
        let index = self.cursor.index() as usize;
        // SAFETY: this is a read-only ownership check on the pinned DMA ring.
        let descriptor = unsafe { core::ptr::read_volatile(descriptors.as_ptr().add(index)) };
        if descriptor.status & TXD_STATUS_DD != 0 {
            Ok(Some(index as u16))
        } else {
            Ok(None)
        }
    }

    /// Collect consecutive free producer slots with one DMA read barrier.
    ///
    /// The returned indices are in submission order and can be passed directly
    /// to submit_batch(). The caller must serialize queue access between this
    /// availability check and publication.
    pub fn available_slots<R: E1000Registers>(
        &self,
        registers: &mut R,
        descriptors: &[TxDescriptor],
        output: &mut [u16],
    ) -> Result<usize, E1000QueueError<R::Error>> {
        if descriptors.len() != self.cursor_count() {
            return Err(E1000QueueError::InvalidRing);
        }
        let free = descriptors.len() - 1 - self.pending as usize;
        let limit = core::cmp::min(output.len(), free);
        if limit == 0 {
            return Ok(0);
        }

        registers.dma_read_barrier();
        let count = descriptors.len();
        let start = self.cursor.index() as usize;
        let mut available = 0;
        while available < limit {
            let index = (start + available) & (count - 1);
            // SAFETY: read-only ownership check on the pinned DMA descriptor ring.
            let descriptor = unsafe {
                core::ptr::read_volatile(descriptors.as_ptr().add(index))
            };
            if descriptor.status & TXD_STATUS_DD == 0 {
                break;
            }
            output[available] = index as u16;
            available += 1;
        }
        Ok(available)
    }

    /// Return the next completed descriptor index in submission order.
    /// The caller can use this index to release the matching DMA buffer.
    pub fn reap_one<R: E1000Registers>(
        &mut self,
        registers: &mut R,
        descriptors: &[TxDescriptor],
    ) -> Result<Option<u16>, E1000QueueError<R::Error>> {
        if descriptors.len() != self.cursor_count() {
            return Err(E1000QueueError::InvalidRing);
        }
        if self.pending == 0 {
            return Ok(None);
        }
        let index = self.clean.index() as usize;
        registers.dma_read_barrier();
        // SAFETY: volatile read observes device write-back in the pinned DMA ring.
        let descriptor = unsafe { core::ptr::read_volatile(descriptors.as_ptr().add(index)) };
        if descriptor.status & TXD_STATUS_DD == 0 {
            return Ok(None);
        }
        self.clean.advance();
        self.pending -= 1;
        Ok(Some(index as u16))
    }

    /// Reclaim all currently completed descriptors with one DMA read barrier.
    /// Completion is consumed strictly in submission order.
    pub fn reap<R: E1000Registers>(
        &mut self,
        registers: &mut R,
        descriptors: &[TxDescriptor],
    ) -> Result<usize, E1000QueueError<R::Error>> {
        if descriptors.len() != self.cursor_count() {
            return Err(E1000QueueError::InvalidRing);
        }
        if self.pending == 0 {
            return Ok(0);
        }
        registers.dma_read_barrier();
        let mut completed = 0usize;
        while completed < self.pending as usize {
            let index = self.clean.index() as usize;
            // SAFETY: volatile read observes device write-back in the pinned DMA ring.
            let descriptor = unsafe { core::ptr::read_volatile(descriptors.as_ptr().add(index)) };
            if descriptor.status & TXD_STATUS_DD == 0 {
                break;
            }
            self.clean.advance();
            completed += 1;
        }
        self.pending -= completed as u16;
        Ok(completed)
    }

    const fn cursor_count(&self) -> usize {
        self.cursor.mask() as usize + 1
    }
}

/// Consumer cursor for the device's legacy RX descriptor ring.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct E1000RxQueue {
    cursor: RingCursor,
    recycle: RingCursor,
    outstanding: u16,
}

impl E1000RxQueue {
    pub fn new(descriptor_count: u16) -> Result<Self, E1000Error> {
        Ok(Self { cursor: RingCursor::new(descriptor_count)?, recycle: RingCursor::new(descriptor_count)?, outstanding: 0 })
    }

    /// Consume one completed descriptor without returning its buffer to hardware.
    /// The caller owns the packet buffer until recycle() succeeds.
    pub fn poll<R: E1000Registers>(
        &mut self,
        registers: &mut R,
        descriptors: &[RxDescriptor],
    ) -> Result<Option<(u16, super::e1000_descriptor::RxCompletion)>, E1000QueueError<R::Error>> {
        let mut result = [(0u16, super::e1000_descriptor::RxCompletion {
            length: 0, end_of_packet: false, errors: 0,
        }); 1];
        if self.poll_batch(registers, descriptors, &mut result)? == 0 {
            Ok(None)
        } else {
            Ok(Some(result[0]))
        }
    }

    /// Drain a contiguous batch of completed RX descriptors with one DMA
    /// read barrier. Output is caller-owned; no heap allocation is performed.
    pub fn poll_batch<R: E1000Registers>(
        &mut self,
        registers: &mut R,
        descriptors: &[RxDescriptor],
        output: &mut [(u16, super::e1000_descriptor::RxCompletion)],
    ) -> Result<usize, E1000QueueError<R::Error>> {
        if descriptors.len() != self.cursor_count() {
            return Err(E1000QueueError::InvalidRing);
        }
        let available = descriptors.len() - self.outstanding as usize;
        let limit = core::cmp::min(output.len(), available);
        if limit == 0 {
            return Ok(0);
        }
        registers.dma_read_barrier();
        let mut count = 0;
        while count < limit {
            let index = self.cursor.index() as usize;
            // SAFETY: volatile read observes device write-back in the pinned DMA ring.
            let descriptor = unsafe { core::ptr::read_volatile(descriptors.as_ptr().add(index)) };
            let Some(completion) = descriptor.completion() else {
                break;
            };
            output[count] = (index as u16, completion);
            self.cursor.advance();
            self.outstanding += 1;
            count += 1;
        }
        Ok(count)
    }

    /// Recycle a contiguous batch of processed RX descriptors with one RDT write.
    ///
    /// `backup` must have room for every completion. The descriptors are saved
    /// before rearming so a failed MMIO tail update can restore CPU-visible
    /// completion metadata and leave queue ownership unchanged.
    pub fn recycle_batch<R: E1000Registers>(
        &mut self,
        registers: &mut R,
        descriptors: &mut [RxDescriptor],
        completed: &[(u16, super::e1000_descriptor::RxCompletion)],
        backup: &mut [RxDescriptor],
    ) -> Result<usize, E1000QueueError<R::Error>> {
        if descriptors.len() != self.cursor_count()
            || completed.len() > backup.len()
            || completed.len() > self.outstanding as usize
        {
            return Err(E1000QueueError::InvalidRing);
        }
        if completed.is_empty() {
            return Ok(0);
        }

        let count = descriptors.len();
        for (offset, (index, _)) in completed.iter().enumerate() {
            let expected = (self.recycle.index() as usize + offset) & (count - 1);
            if *index as usize != expected {
                return Err(E1000QueueError::InvalidRing);
            }
        }

        // Preflight every descriptor before changing any of them. This prevents
        // callers from recycling a slot that hardware has not completed yet.
        for (offset, (index, _)) in completed.iter().enumerate() {
            let slot = *index as usize;
            // SAFETY: read-only inspection of a pinned DMA descriptor owned by CPU.
            let original = unsafe { core::ptr::read_volatile(descriptors.as_ptr().add(slot)) };
            if original.status & super::e1000::RXD_STATUS_DD == 0 {
                return Err(E1000QueueError::InvalidRing);
            }
            backup[offset] = original;
        }
        for (offset, (index, _)) in completed.iter().enumerate() {
            let slot = *index as usize;
            let rearmed = RxDescriptor::for_buffer(backup[offset].buffer_address);
            // SAFETY: rearm only descriptors whose packets have already been consumed.
            unsafe { core::ptr::write_volatile(descriptors.as_mut_ptr().add(slot), rearmed); }
        }
        registers.dma_write_barrier();
        let tail = completed[completed.len() - 1].0 as u32;
        if let Err(error) = registers.write32(REG_RDT, tail) {
            for (offset, (index, _)) in completed.iter().enumerate() {
                // SAFETY: failed tail publication leaves all descriptors CPU-owned.
                unsafe {
                    core::ptr::write_volatile(
                        descriptors.as_mut_ptr().add(*index as usize), backup[offset],
                    );
                }
            }
            return Err(E1000QueueError::Register(error));
        }

        for _ in completed {
            self.recycle.advance();
        }
        self.outstanding -= completed.len() as u16;
        Ok(completed.len())
    }

    /// Return a processed RX buffer to hardware. Recycling must follow descriptor
    /// order so RDT never advances past an unreleased buffer.
    pub fn recycle<R: E1000Registers>(
        &mut self,
        registers: &mut R,
        descriptors: &mut [RxDescriptor],
        index: u16,
    ) -> Result<(), E1000QueueError<R::Error>> {
        if descriptors.len() != self.cursor_count() || self.outstanding == 0
            || index != self.recycle.index()
        {
            return Err(E1000QueueError::InvalidRing);
        }
        let slot = index as usize;
        // SAFETY: this slot is pinned DMA memory and is CPU-owned until recycled.
        let completed_descriptor = unsafe { core::ptr::read_volatile(descriptors.as_ptr().add(slot)) };
        if completed_descriptor.status & super::e1000::RXD_STATUS_DD == 0 {
            return Err(E1000QueueError::InvalidRing);
        }
        let rearmed = RxDescriptor::for_buffer(completed_descriptor.buffer_address);
        // SAFETY: write the reset descriptor before advancing the hardware tail.
        unsafe { core::ptr::write_volatile(descriptors.as_mut_ptr().add(slot), rearmed); }
        registers.dma_write_barrier();
        if let Err(error) = registers.write32(REG_RDT, index as u32) {
            // SAFETY: restore the completed descriptor because hardware did not accept the tail update.
            unsafe { core::ptr::write_volatile(descriptors.as_mut_ptr().add(slot), completed_descriptor); }
            return Err(E1000QueueError::Register(error));
        }
        self.recycle.advance();
        self.outstanding -= 1;
        Ok(())
    }

    const fn cursor_count(&self) -> usize {
        self.cursor.mask() as usize + 1
    }
}

fn validate_queue(queues: &E1000QueueConfig) -> Result<(), ()> {
    for ring in [queues.rx, queues.tx] {
        if ring.descriptor_count < 8
            || !ring.descriptor_count.is_power_of_two()
            || ring.byte_length as usize
                != ring.descriptor_count as usize * super::e1000::DESCRIPTOR_SIZE
            || ring.byte_length % RING_LENGTH_ALIGNMENT != 0
            || ring.physical_base % super::e1000::DESCRIPTOR_SIZE as u64 != 0
        {
            return Err(());
        }
    }
    Ok(())
}

fn program_ring<R: E1000Registers>(
    registers: &mut R,
    low_reg: u32,
    high_reg: u32,
    length_reg: u32,
    head_reg: u32,
    tail_reg: u32,
    ring: RingConfig,
) -> Result<(), R::Error> {
    let (low, high) = ring.base_registers();
    registers.write32(low_reg, low)?;
    registers.write32(high_reg, high)?;
    registers.write32(length_reg, ring.byte_length)?;
    registers.write32(head_reg, 0)?;
    let initial_tail = if tail_reg == REG_RDT {
        ring.descriptor_count as u32 - 1
    } else {
        0
    };
    registers.write32(tail_reg, initial_tail)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    #[derive(Default)]
    struct MockRegisters {
        writes: Vec<(u32, u32)>,
    }

    impl E1000Registers for MockRegisters {
        type Error = ();

        fn read32(&mut self, offset: u32) -> Result<u32, Self::Error> {
            if offset == REG_CTRL {
                return Ok(0);
            }
            if offset == REG_RAL0 { return Ok(0x4433_2211); }
            if offset == REG_STATUS { return Ok(STATUS_LU | STATUS_FD | (2 << STATUS_SPEED_SHIFT)); }
            if offset == REG_RAH0 { return Ok(0x8000_6655); }
            Ok(0)
        }

        fn write32(&mut self, offset: u32, value: u32) -> Result<(), Self::Error> {
            self.writes.push((offset, value));
            Ok(())
        }
        fn dma_write_barrier(&mut self) { core::sync::atomic::fence(core::sync::atomic::Ordering::Release); }
        fn dma_read_barrier(&mut self) { core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire); }
    }

    #[test]
    fn link_state_reports_speed_and_duplex() {
        let mut regs = MockRegisters::default();
        assert_eq!(read_link_state(&mut regs), Ok(super::super::device::LinkState::Up {
            speed_mbps: Some(1000), full_duplex: true,
        }));
    }

    #[test]
    fn queue_config_enforces_hardware_ring_length_alignment() {
        assert!(E1000QueueConfig::defaults(0x1000, 0x2000).is_ok());
        assert!(E1000QueueConfig::new(0x1000, 0x2000, 12, 32).is_err());
    }

    #[test]
    fn initialization_programs_rings_and_enables_rx_tx() {
        let mut regs = MockRegisters::default();
        let queues = E1000QueueConfig::defaults(0x1_0000_1000, 0x1_0000_2000).unwrap();
        let mut rx = [RxDescriptor::default(); 32];
        let rx_buffers = core::array::from_fn(|index| 0x4000u64 + index as u64 * 2048);
        let mut tx = [TxDescriptor::default(); 32];
        let info = initialize(&mut regs, queues, &mut rx, &rx_buffers, &mut tx, true).unwrap();
        assert_eq!(info.mac, MacAddress([0x11, 0x22, 0x33, 0x44, 0x55, 0x66]));
        assert_eq!(rx[0].buffer_address, 0x4000);
        assert_eq!(rx[0].status, 0);
        assert_eq!(tx[0].status, TXD_STATUS_DD);
        assert!(regs.writes.contains(&(REG_RDBAH, 1)));
        assert!(regs.writes.contains(&(REG_TDBAH, 1)));
        assert!(regs.writes.contains(&(REG_RDT, 31)));
        assert!(regs.writes.contains(&(REG_TDT, 0)));
        assert!(regs.writes.contains(&(REG_RCTL, RCTL_EN | RCTL_LPE | RCTL_BAM | RCTL_SECRC)));
        assert!(regs.writes.iter().any(|(reg, value)| *reg == REG_TCTL && value & TCTL_EN != 0));
    }

    #[test]
    fn tx_queue_tracks_in_flight_descriptors_and_reclaims_in_order() {
        let mut regs = MockRegisters::default();
        let mut descriptors = [TxDescriptor { status: TXD_STATUS_DD, ..TxDescriptor::default() }; 8];
        let mut queue = E1000TxQueue::new(8).unwrap();
        assert_eq!(queue.submit(&mut regs, &mut descriptors, 0x8000, 128), Ok(0));
        assert_eq!(queue.reap(&mut regs, &descriptors), Ok(0));
        descriptors[0].status = TXD_STATUS_DD;
        assert_eq!(queue.reap_one(&mut regs, &descriptors), Ok(Some(0)));
        assert_eq!(queue.reap(&mut regs, &descriptors), Ok(0));
        assert_eq!(queue.submit(&mut regs, &mut descriptors, 0x9000, 256), Ok(1));
    }

    #[test]
    fn tx_next_available_slot_tracks_descriptor_ownership() {
        let mut regs = MockRegisters::default();
        let mut descriptors = [TxDescriptor { status: TXD_STATUS_DD, ..TxDescriptor::default() }; 8];
        let mut queue = E1000TxQueue::new(8).unwrap();
        assert_eq!(queue.next_available_slot(&mut regs, &descriptors), Ok(Some(0)));
        assert_eq!(queue.submit(&mut regs, &mut descriptors, 0x8000, 64), Ok(0));
        assert_eq!(queue.next_available_slot(&mut regs, &descriptors), Ok(Some(1)));
        descriptors[1].status = 0;
        assert_eq!(queue.next_available_slot(&mut regs, &descriptors), Ok(None));
        descriptors[1].status = TXD_STATUS_DD;
        descriptors[0].status = TXD_STATUS_DD;
        assert_eq!(queue.reap_one(&mut regs, &descriptors), Ok(Some(0)));
        assert_eq!(queue.next_available_slot(&mut regs, &descriptors), Ok(Some(1)));
    }

    #[test]
    fn tx_available_slots_returns_contiguous_batch_in_ring_order() {
        let mut regs = MockRegisters::default();
        let mut descriptors = [TxDescriptor { status: TXD_STATUS_DD, ..TxDescriptor::default() }; 8];
        let mut queue = E1000TxQueue::new(8).unwrap();
        let mut slots = [u16::MAX; 4];
        assert_eq!(queue.available_slots(&mut regs, &descriptors, &mut slots), Ok(4));
        assert_eq!(slots, [0, 1, 2, 3]);

        assert_eq!(queue.submit_batch(&mut regs, &mut descriptors, &[(0x8000, 64), (0x9000, 64)]), Ok(2));
        assert_eq!(queue.available_slots(&mut regs, &descriptors, &mut slots), Ok(2));
        assert_eq!(&slots[..2], &[2, 3]);
    }

    #[test]
    fn tx_batch_publishes_multiple_descriptors_with_one_tail_update() {
        let mut regs = MockRegisters::default();
        let mut descriptors = [TxDescriptor { status: TXD_STATUS_DD, ..TxDescriptor::default() }; 8];
        let mut queue = E1000TxQueue::new(8).unwrap();
        assert_eq!(queue.submit_batch(&mut regs, &mut descriptors, &[(0x8000, 128), (0x9000, 256)]), Ok(2));
        assert_eq!(regs.writes.iter().filter(|(register, _)| *register == REG_TDT).count(), 1);
        assert!(regs.writes.contains(&(REG_TDT, 2)));
        descriptors[0].status = TXD_STATUS_DD;
        descriptors[1].status = TXD_STATUS_DD;
        assert_eq!(queue.reap(&mut regs, &descriptors), Ok(2));
    }

    #[test]
    fn rx_queue_rearms_only_a_completed_descriptor() {
        let mut regs = MockRegisters::default();
        let mut descriptors = [RxDescriptor::for_buffer(0xa000); 8];
        let mut queue = E1000RxQueue::new(8).unwrap();
        assert_eq!(queue.poll(&mut regs, &mut descriptors), Ok(None));
        descriptors[0].length = 96;
        descriptors[0].status = super::super::e1000::RXD_STATUS_DD | super::super::e1000::RXD_STATUS_EOP;
        let (index, completion) = queue.poll(&mut regs, &descriptors).unwrap().unwrap();
        assert_eq!(index, 0);
        assert_eq!(completion.length, 96);
        assert_eq!(descriptors[0].buffer_address, 0xa000);
        assert_eq!(descriptors[0].status & super::super::e1000::RXD_STATUS_DD, super::super::e1000::RXD_STATUS_DD);
        assert!(queue.recycle(&mut regs, &mut descriptors, index).is_ok());
        assert_eq!(descriptors[0].status, 0);
        assert!(regs.writes.contains(&(REG_RDT, 0)));
    }

    #[test]
    fn rx_batch_recycles_contiguous_descriptors_with_one_tail_update() {
        let mut regs = MockRegisters::default();
        let mut descriptors = [RxDescriptor::for_buffer(0xa000); 8];
        let mut queue = E1000RxQueue::new(8).unwrap();
        let mut completions = [(0u16, super::super::e1000_descriptor::RxCompletion {
            length: 0, end_of_packet: false, errors: 0,
        }); 2];
        descriptors[0].length = 64;
        descriptors[0].status = super::super::e1000::RXD_STATUS_DD | super::super::e1000::RXD_STATUS_EOP;
        descriptors[1].length = 96;
        descriptors[1].status = super::super::e1000::RXD_STATUS_DD | super::super::e1000::RXD_STATUS_EOP;
        assert_eq!(queue.poll_batch(&mut regs, &descriptors, &mut completions), Ok(2));
        let mut backup = [RxDescriptor::default(); 2];
        assert_eq!(queue.recycle_batch(&mut regs, &mut descriptors, &completions, &mut backup), Ok(2));
        assert_eq!(descriptors[0].status, 0);
        assert_eq!(descriptors[1].status, 0);
        assert_eq!(regs.writes.iter().filter(|(register, _)| *register == REG_RDT).count(), 1);
        assert!(regs.writes.contains(&(REG_RDT, 1)));
        assert_eq!(queue.poll(&mut regs, &descriptors), Ok(None));
    }

    #[test]
    fn rx_batch_rejects_uncompleted_descriptor_without_partial_rearm() {
        let mut regs = MockRegisters::default();
        let mut descriptors = [RxDescriptor::for_buffer(0xa000); 8];
        let mut queue = E1000RxQueue::new(8).unwrap();
        let mut completions = [(0u16, super::super::e1000_descriptor::RxCompletion {
            length: 64, end_of_packet: true, errors: 0,
        }); 2];
        descriptors[0].status = super::super::e1000::RXD_STATUS_DD;
        descriptors[1].status = 0;
        assert_eq!(queue.poll_batch(&mut regs, &descriptors, &mut completions), Ok(1));
        // Simulate corrupted ownership metadata after polling. The queue must
        // reject the recycle without advancing RDT or changing software state.
        descriptors[0].status = 0;
        let mut backup = [RxDescriptor::default(); 1];
        assert_eq!(queue.recycle_batch(&mut regs, &mut descriptors, &completions[..1], &mut backup),
            Err(E1000QueueError::InvalidRing));
        assert_eq!(descriptors[0].status, 0);
        assert_eq!(regs.writes.iter().filter(|(register, _)| *register == REG_RDT).count(), 0);
    }

    #[test]
    fn refuses_to_initialize_without_bus_mastering() {
        let mut regs = MockRegisters::default();
        let queues = E1000QueueConfig::defaults(0x1000, 0x2000).unwrap();
        let mut rx = [RxDescriptor::default(); 32];
        let rx_buffers = [0x4000u64; 32];
        let mut tx = [TxDescriptor::default(); 32];
        assert_eq!(initialize(&mut regs, queues, &mut rx, &rx_buffers, &mut tx, false), Err(E1000InitError::BusMasterDisabled));
        assert!(regs.writes.is_empty());
    }
}
