//! E1000 register-level initialization over a caller-provided MMIO window.
//!
//! The platform supplies the mapped MMIO accessor and DMA-backed descriptor
//! rings. This module never maps physical addresses, enables PCI bus mastering,
//! allocates DMA memory, or installs interrupt handlers.

use super::e1000::{
    RingConfig, RxDescriptor, TxDescriptor, RingCursor, E1000Error, RXD_STATUS_DD, TXD_STATUS_DD, CTRL_RST, DEFAULT_RX_RING_SIZE, DEFAULT_TX_RING_SIZE,
    REG_CTRL, REG_ICR, REG_IMC, REG_RAL0, REG_RAH0, REG_RCTL,
    REG_RDBAH, REG_RDBAL, REG_RDH, REG_RDLEN, REG_RDT,
    REG_TCTL, REG_TDBAH, REG_TDBAL, REG_TDH, REG_TDLEN, REG_TDT,
    RCTL_BAM, RCTL_EN, RCTL_SECRC, TCTL_COLD_SHIFT, TCTL_CT_SHIFT, TCTL_EN, TCTL_PSP,
};
use super::e1000_descriptor::mac_address_from_registers;
use super::ethernet::MacAddress;

const RESET_POLL_LIMIT: usize = 1_000_000;
const RING_LENGTH_ALIGNMENT: u32 = 128;
const TCTL_COLLISION_THRESHOLD: u32 = 0x10;
const TCTL_COLLISION_DISTANCE: u32 = 0x40;

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
    InvalidFrameLength,
    TxRingFull,
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
/// addresses, TX descriptors must start with DD set, all rings and buffers
/// must be DMA-mapped and pinned, and PCI bus mastering must already be enabled.
/// No DMA memory is dereferenced here.
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
        || rx_buffer_dma.iter().any(|address| *address == 0)
    {
        return Err(E1000InitError::InvalidDescriptorBuffers);
    }

    for (descriptor, &buffer_dma) in rx_descriptors.iter_mut().zip(rx_buffer_dma.iter()) {
        *descriptor = RxDescriptor::for_buffer(buffer_dma);
    }
    for descriptor in tx_descriptors.iter_mut() {
        *descriptor = TxDescriptor { status: TXD_STATUS_DD, ..TxDescriptor::default() };
    }

    registers.dma_write_barrier();
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
    registers.write32(REG_RCTL, RCTL_EN | RCTL_BAM | RCTL_SECRC)
        .map_err(E1000InitError::Register)?;

    Ok(E1000InitInfo {
        mac,
        rx_descriptors: queues.rx.descriptor_count,
        tx_descriptors: queues.tx.descriptor_count,
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
}

impl E1000TxQueue {
    pub fn new(descriptor_count: u16) -> Result<Self, E1000Error> {
        Ok(Self { cursor: RingCursor::new(descriptor_count)? })
    }

    /// Submit a DMA-mapped frame. The caller must keep its buffer alive until
    /// the corresponding descriptor's DD bit is observed by reap().
    pub fn submit<R: E1000Registers>(
        &mut self,
        registers: &mut R,
        descriptors: &mut [TxDescriptor],
        buffer_dma: u64,
        length: usize,
    ) -> Result<u16, E1000QueueError<R::Error>> {
        if descriptors.len() != self.cursor_count() {
            return Err(E1000QueueError::InvalidRing);
        }
        if length == 0 || length > u16::MAX as usize || buffer_dma == 0 {
            return Err(E1000QueueError::InvalidFrameLength);
        }
        let index = self.cursor.index() as usize;
        if descriptors[index].status & TXD_STATUS_DD == 0 {
            return Err(E1000QueueError::TxRingFull);
        }
        let descriptor = super::e1000_descriptor::TxDescriptor::for_frame(buffer_dma, length)
            .map_err(|_| E1000QueueError::InvalidFrameLength)?;
        descriptors[index] = descriptor;
        registers.dma_write_barrier();
        let next = self.cursor.advance();
        registers.write32(REG_TDT, next as u32).map_err(E1000QueueError::Register)?;
        Ok(index as u16)
    }

    /// Count and release completed descriptors in order. The caller may free
    /// the associated DMA buffers only after this reports their completion.
    pub fn reap(&mut self, registers: &mut R, descriptors: &[TxDescriptor])
        -> Result<usize, E1000QueueError<R::Error>>
    where
        R: E1000Registers,
    {
        if descriptors.len() != self.cursor_count() {
            return Err(E1000QueueError::InvalidRing);
        }
        let mut completed = 0;
        for descriptor in descriptors {
            registers.dma_read_barrier();
            if descriptor.status & TXD_STATUS_DD != 0 {
                completed += 1;
            }
        }
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
}

impl E1000RxQueue {
    pub fn new(descriptor_count: u16) -> Result<Self, E1000Error> {
        Ok(Self { cursor: RingCursor::new(descriptor_count)? })
    }

    /// Consume one completed descriptor and return it to hardware. The packet
    /// remains in its DMA buffer; the caller must inspect/copy it before reuse.
    pub fn poll<R: E1000Registers>(
        &mut self,
        registers: &mut R,
        descriptors: &mut [RxDescriptor],
    ) -> Result<Option<(u16, super::e1000_descriptor::RxCompletion)>, E1000QueueError<R::Error>> {
        if descriptors.len() != self.cursor_count() {
            return Err(E1000QueueError::InvalidRing);
        }
        let index = self.cursor.index() as usize;
        registers.dma_read_barrier();
        let Some(completion) = descriptors[index].completion() else {
            return Ok(None);
        };

        let buffer_dma = descriptors[index].buffer_address;
        descriptors[index] = RxDescriptor::for_buffer(buffer_dma);
        registers.dma_write_barrier();
        registers.write32(REG_RDT, index as u32).map_err(E1000QueueError::Register)?;
        self.cursor.advance();
        Ok(Some((index as u16, completion)))
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
    fn queue_config_enforces_hardware_ring_length_alignment() {
        assert!(E1000QueueConfig::defaults(0x1000, 0x2000).is_ok());
        assert!(E1000QueueConfig::new(0x1000, 0x2000, 12, 32).is_err());
    }

    #[test]
    fn initialization_programs_rings_and_enables_rx_tx() {
        let mut regs = MockRegisters::default();
        let queues = E1000QueueConfig::defaults(0x1_0000_1000, 0x1_0000_2000).unwrap();
        let mut rx = [RxDescriptor::default(); 32];
        let rx_buffers = [0x4000u64; 32];
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
        assert!(regs.writes.contains(&(REG_RCTL, RCTL_EN | RCTL_BAM | RCTL_SECRC)));
        assert!(regs.writes.iter().any(|(reg, value)| *reg == REG_TCTL && value & TCTL_EN != 0));
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
