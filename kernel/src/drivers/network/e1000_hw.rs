//! E1000 register-level initialization over a caller-provided MMIO window.
//!
//! The platform supplies the mapped MMIO accessor and DMA-backed descriptor
//! rings. This module never maps physical addresses, enables PCI bus mastering,
//! allocates DMA memory, or installs interrupt handlers.

use super::e1000::{
    RingConfig, RxDescriptor, TxDescriptor, RingCursor, E1000Error, RXD_STATUS_DD, TXD_STATUS_DD, CTRL_RST, DEFAULT_RX_RING_SIZE, DEFAULT_TX_RING_SIZE,
    REG_CTRL, REG_STATUS, REG_ICR, REG_IMC, REG_RAL0, REG_RAH0, REG_RCTL,
    REG_MTA_BASE, E1000_MTA_REGISTER_COUNT,
    REG_RDBAH, REG_RDBAL, REG_RDH, REG_RDLEN, REG_RDT,
    REG_TCTL, REG_TDBAH, REG_TDBAL, REG_TDH, REG_TDLEN, REG_TDT,
    RCTL_BAM, RCTL_EN, RCTL_SECRC, TCTL_COLD_SHIFT, TCTL_CT_SHIFT, TCTL_EN, TCTL_PSP,
};
use super::e1000_descriptor::{mac_address_from_registers, RAH0_AV};
use super::ethernet::MacAddress;
use super::dma::DmaBuffer;

#[path = "e1000_hw_rx.rs"]
mod rx_queue;
#[path = "e1000_hw_tx.rs"]
mod tx_queue;
pub(crate) use rx_queue::E1000RxQueue;

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
///
/// # Safety
/// Implementors must ensure accesses target a live E1000 MMIO window, never
/// access outside the mapped region, and provide barriers that correctly order
/// CPU access to coherent DMA memory with device ownership transitions.
/// A failed `write32` must be reported before any register write is issued;
/// returning an error after a posted/possibly-visible write would make descriptor
/// rollback unsafe because the device might already own the published buffers.
pub unsafe trait E1000Registers {
    type Error;

    fn read32(&mut self, offset: u32) -> Result<u32, Self::Error>;
    fn write32(&mut self, offset: u32, value: u32) -> Result<(), Self::Error>;

    /// Order CPU writes to packet buffers and descriptors before notifying the device.
    fn dma_write_barrier(&mut self);
    /// After the CPU observes a device-written DD ownership bit, make all
    /// preceding device writes to descriptor metadata and packet buffers visible
    /// before the CPU reads that metadata or reuses the completed buffer.
    fn dma_read_barrier(&mut self);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum E1000InitError<E> {
    Register(E),
    ResetTimeout,
    InvalidRingConfiguration,
    BusMasterDisabled,
    MissingMacAddress,
    InvalidMacAddress,
    InvalidDescriptorBuffers,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum E1000ResetError<E> {
    Register(E),
    Timeout,
}

/// Mask interrupts and reset the MAC/DMA engines, waiting for the self-clearing
/// reset bit before the caller releases or reprograms DMA-visible memory.
pub fn reset<R: E1000Registers>(registers: &mut R) -> Result<(), E1000ResetError<R::Error>> {
    registers.write32(REG_IMC, u32::MAX).map_err(E1000ResetError::Register)?;
    let control = registers.read32(REG_CTRL).map_err(E1000ResetError::Register)?;
    registers.write32(REG_CTRL, control | CTRL_RST).map_err(E1000ResetError::Register)?;

    for _ in 0..RESET_POLL_LIMIT {
        let value = registers.read32(REG_CTRL).map_err(E1000ResetError::Register)?;
        if value & CTRL_RST == 0 {
            return Ok(());
        }
        core::hint::spin_loop();
    }
    Err(E1000ResetError::Timeout)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct E1000QueueConfig {
    pub rx: RingConfig,
    pub tx: RingConfig,
}

impl E1000QueueConfig {
    pub fn new(rx_dma_base: u64, tx_dma_base: u64, rx_count: usize, tx_count: usize)
        -> Result<Self, super::e1000::E1000Error>
    {
        let config = Self {
            rx: RingConfig::new(rx_dma_base, rx_count)?,
            tx: RingConfig::new(tx_dma_base, tx_count)?,
        };
        config.validate()?;
        Ok(config)
    }

    /// Revalidate public ring fields and ensure the RX/TX DMA ranges are disjoint.
    pub fn validate(self) -> Result<(), super::e1000::E1000Error> {
        let rx = RingConfig::new(self.rx.dma_base, self.rx.descriptor_count as usize)?;
        let tx = RingConfig::new(self.tx.dma_base, self.tx.descriptor_count as usize)?;
        if rx != self.rx || tx != self.tx
            || rx.byte_length % RING_LENGTH_ALIGNMENT != 0
            || tx.byte_length % RING_LENGTH_ALIGNMENT != 0
        {
            return Err(super::e1000::E1000Error::InvalidRingSize);
        }
        let rx_end = rx.dma_base + rx.byte_length as u64;
        let tx_end = tx.dma_base + tx.byte_length as u64;
        if rx.dma_base < tx_end && tx.dma_base < rx_end {
            return Err(super::e1000::E1000Error::OverlappingDmaRanges);
        }
        Ok(())
    }

    pub fn defaults(rx_dma_base: u64, tx_dma_base: u64) -> Result<Self, super::e1000::E1000Error> {
        Self::new(rx_dma_base, tx_dma_base, DEFAULT_RX_RING_SIZE, DEFAULT_TX_RING_SIZE)
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
/// must alias the device-visible DMA ring bases supplied in queues, and PCI bus mastering
/// must already be enabled. The platform must provide coherent DMA or implement
/// cache maintenance in its barrier methods. After successful initialization,
/// the caller must keep every ring and buffer mapping pinned until `reset()`
/// returns successfully, including when a later adapter-binding step fails.
///
/// # Safety
/// The MMIO accessor must target this NIC. Both descriptor slices must alias
/// the pinned DMA ring mappings at the device-visible DMA bases in `queues`; every RX
/// DmaBuffer must refer to a pinned, writable device-visible mapping for its
/// entire lifetime. No other CPU or device may mutate these rings during setup.
pub unsafe fn initialize<R: E1000Registers>(
    registers: &mut R,
    queues: E1000QueueConfig,
    rx_descriptors: &mut [RxDescriptor],
    rx_buffers: &[DmaBuffer<'_>],
    tx_descriptors: &mut [TxDescriptor],
    bus_master_enabled: bool,
) -> Result<E1000InitInfo, E1000InitError<R::Error>> {
    if !bus_master_enabled {
        return Err(E1000InitError::BusMasterDisabled);
    }
    validate_queue(&queues).map_err(|_| E1000InitError::InvalidRingConfiguration)?;
    if rx_descriptors.len() != queues.rx.descriptor_count as usize
        || rx_buffers.len() != rx_descriptors.len()
        || tx_descriptors.len() != queues.tx.descriptor_count as usize
        || DmaBuffer::validate_rx_pool(rx_buffers).is_err()
        || DmaBuffer::validate_disjoint_from_range(
            rx_buffers, queues.rx.dma_base, queues.rx.byte_length as usize,
        ).is_err()
        || DmaBuffer::validate_disjoint_from_range(
            rx_buffers, queues.tx.dma_base, queues.tx.byte_length as usize,
        ).is_err()
    {
        return Err(E1000InitError::InvalidDescriptorBuffers);
    }

    // Use the global MAC reset to stop DMA engines. In particular, avoid
    // clearing RCTL.EN while a packet is being received on affected 82574L
    // revisions; Intel's specification update prescribes CTRL.RST instead.
    reset(registers).map_err(|error| match error {
        E1000ResetError::Register(error) => E1000InitError::Register(error),
        E1000ResetError::Timeout => E1000InitError::ResetTimeout,
    })?;

    // Start with an explicit empty multicast table. The adapter later installs
    // only the multicast groups requested by the network stack.
    for index in 0..E1000_MTA_REGISTER_COUNT {
        registers.write32(REG_MTA_BASE + index as u32 * 4, 0)
            .map_err(E1000InitError::Register)?;
    }

    // Hardware queues are stopped and reset; now initialize the DMA-visible rings.
    for (descriptor, buffer) in rx_descriptors.iter_mut().zip(rx_buffers.iter()) {
        *descriptor = RxDescriptor::for_buffer(buffer.dma_address());
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

    // EEPROM auto-load may finish shortly after CTRL.RST clears. Wait for
    // RAH0.AV rather than assuming the receive address is immediately valid.
    let mut mac = None;
    for _ in 0..RESET_POLL_LIMIT {
        let rah = registers.read32(REG_RAH0).map_err(E1000InitError::Register)?;
        if rah & RAH0_AV != 0 {
            let ral = registers.read32(REG_RAL0).map_err(E1000InitError::Register)?;
            mac = mac_address_from_registers(ral, rah);
            break;
        }
        core::hint::spin_loop();
    }
    let mac = mac.ok_or(E1000InitError::MissingMacAddress)?;
    if !mac.is_unicast() {
        return Err(E1000InitError::InvalidMacAddress);
    }

    program_ring(registers, REG_RDBAL, REG_RDBAH, REG_RDLEN, REG_RDH, REG_RDT, queues.rx)
        .map_err(E1000InitError::Register)?;
    program_ring(registers, REG_TDBAL, REG_TDBAH, REG_TDLEN, REG_TDH, REG_TDT, queues.tx)
        .map_err(E1000InitError::Register)?;

    let tctl = TCTL_EN | TCTL_PSP
        | (TCTL_COLLISION_THRESHOLD << TCTL_CT_SHIFT)
        | (TCTL_COLLISION_DISTANCE << TCTL_COLD_SHIFT);
    registers.write32(REG_TCTL, tctl).map_err(E1000InitError::Register)?;
    registers.dma_write_barrier();
    // Keep LPE disabled for the standard 1500-byte MTU. VLAN-tagged standard
    // frames remain supported; jumbo frames are intentionally not enabled.
    registers.write32(REG_RCTL, RCTL_EN | RCTL_BAM | RCTL_SECRC)
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
pub(crate) enum E1000QueueError<E> {
    Register(E),
    InvalidRing,
    InvalidFrameLength,
    TxRingFull,
}

/// Single-producer/single-consumer software state for a legacy TX ring.
/// Keep one instance per hardware queue and serialize access to it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct E1000TxQueue {
    cursor: RingCursor,
    clean: RingCursor,
    pending: u16,
}

fn validate_queue(queues: &E1000QueueConfig) -> Result<(), ()> {
    queues.validate().map_err(|_| ())
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
#[path = "e1000_hw_tests.rs"]
mod tests;
