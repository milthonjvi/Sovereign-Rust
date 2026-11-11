//! Intel 8254x/8257x E1000 register and descriptor definitions.
//!
//! This module deliberately contains no raw MMIO or DMA operations yet: the
//! kernel does not currently expose safe BAR mapping or DMA allocation APIs.
//! It provides validated, hardware-layout primitives for that integration.

use core::mem::{align_of, size_of};

pub const PCI_VENDOR_INTEL: u16 = 0x8086;
pub const DEVICE_82540EM: u16 = 0x100e; // QEMU's common e1000 device
pub const DEVICE_82545EM: u16 = 0x100f;
pub const DEVICE_82574L: u16 = 0x10d3;

pub const REG_CTRL: u32 = 0x0000;
pub const REG_STATUS: u32 = 0x0008;
pub const REG_EECD: u32 = 0x0010;
pub const REG_EERD: u32 = 0x0014;
pub const REG_ICR: u32 = 0x00c0;
pub const REG_IMS: u32 = 0x00d0;
pub const REG_IMC: u32 = 0x00d8;
pub const REG_RCTL: u32 = 0x0100;
pub const REG_TCTL: u32 = 0x0400;
pub const REG_RDBAL: u32 = 0x2800;
pub const REG_RDBAH: u32 = 0x2804;
pub const REG_RDLEN: u32 = 0x2808;
pub const REG_RDH: u32 = 0x2810;
pub const REG_RDT: u32 = 0x2818;
pub const REG_TDBAL: u32 = 0x3800;
pub const REG_TDBAH: u32 = 0x3804;
pub const REG_TDLEN: u32 = 0x3808;
pub const REG_TDH: u32 = 0x3810;
pub const REG_TDT: u32 = 0x3818;
pub const REG_RAL0: u32 = 0x5400;
pub const REG_RAH0: u32 = 0x5404;
pub const REG_MTA_BASE: u32 = 0x5200;
pub const E1000_MTA_REGISTER_COUNT: usize = 128;

pub const CTRL_RST: u32 = 1 << 26;
pub const CTRL_ASDE: u32 = 1 << 5;
pub const RCTL_EN: u32 = 1 << 1;
pub const RCTL_LPE: u32 = 1 << 5;
pub const RCTL_BAM: u32 = 1 << 15;
pub const RCTL_SECRC: u32 = 1 << 26;
pub const TCTL_EN: u32 = 1 << 1;
pub const TCTL_PSP: u32 = 1 << 3;
pub const TCTL_CT_SHIFT: u32 = 4;
pub const TCTL_COLD_SHIFT: u32 = 12;

pub const RXD_STATUS_DD: u8 = 1 << 0;
pub const RXD_STATUS_EOP: u8 = 1 << 1;
pub const TXD_CMD_EOP: u8 = 1 << 0;
pub const TXD_CMD_IFCS: u8 = 1 << 1;
pub const TXD_CMD_RS: u8 = 1 << 3;
pub const TXD_STATUS_DD: u8 = 1 << 0;
pub const DESCRIPTOR_SIZE: usize = 16;
pub const DEFAULT_RX_RING_SIZE: usize = 32;
pub const DEFAULT_TX_RING_SIZE: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum E1000Error { UnsupportedDevice, InvalidRingSize, NullDmaAddress, MisalignedDmaAddress, DmaAddressTooWide, OverlappingDmaRanges, InvalidDescriptorLength }

pub const fn supports_device(vendor_id: u16, device_id: u16) -> bool {
    vendor_id == PCI_VENDOR_INTEL
        && (device_id == DEVICE_82540EM || device_id == DEVICE_82545EM || device_id == DEVICE_82574L)
}

/// Legacy receive descriptor layout, exactly 16 bytes as specified by Intel.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RxDescriptor {
    pub buffer_address: u64,
    pub length: u16,
    pub checksum: u16,
    pub status: u8,
    pub errors: u8,
    pub special: u16,
}

/// Legacy transmit descriptor layout, exactly 16 bytes as specified by Intel.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct TxDescriptor {
    pub buffer_address: u64,
    pub length: u16,
    pub checksum_offset: u8,
    pub command: u8,
    pub status: u8,
    pub checksum_start: u8,
    pub special: u16,
}

const _: [(); DESCRIPTOR_SIZE] = [(); size_of::<RxDescriptor>()];
const _: [(); DESCRIPTOR_SIZE] = [(); size_of::<TxDescriptor>()];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RingConfig { pub dma_base: u64, pub byte_length: u32, pub descriptor_count: u16 }

impl RingConfig {
    /// Validate a legacy descriptor ring before programming RDBA/TDBA registers.
    /// The DMA allocator must guarantee that the returned device-visible range remains
    /// pinned and device-accessible for the entire lifetime of the ring.
    pub fn new(dma_base: u64, descriptor_count: usize) -> Result<Self, E1000Error> {
        if descriptor_count < 8 || descriptor_count > u16::MAX as usize || !descriptor_count.is_power_of_two() {
            return Err(E1000Error::InvalidRingSize);
        }
        if dma_base == 0 {
            return Err(E1000Error::NullDmaAddress);
        }
        if dma_base % DESCRIPTOR_SIZE as u64 != 0 {
            return Err(E1000Error::MisalignedDmaAddress);
        }
        let byte_length = descriptor_count.checked_mul(DESCRIPTOR_SIZE)
            .and_then(|n| u32::try_from(n).ok()).ok_or(E1000Error::InvalidRingSize)?;
        dma_base.checked_add(byte_length as u64)
            .ok_or(E1000Error::DmaAddressTooWide)?;
        if byte_length % DESCRIPTOR_SIZE as u32 != 0 {
            return Err(E1000Error::InvalidRingSize);
        }
        Ok(Self { dma_base, byte_length, descriptor_count: descriptor_count as u16 })
    }

    /// E1000 legacy ring registers accept a 64-bit base split across two registers.
    pub const fn base_registers(self) -> (u32, u32) {
        (self.dma_base as u32, (self.dma_base >> 32) as u32)
    }
}

/// A descriptor-ring cursor. The ring size must be a power of two.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RingCursor { index: u16, mask: u16 }

impl RingCursor {
    pub fn new(descriptor_count: u16) -> Result<Self, E1000Error> {
        if descriptor_count < 8 || !descriptor_count.is_power_of_two() {
            return Err(E1000Error::InvalidRingSize);
        }
        Ok(Self { index: 0, mask: descriptor_count - 1 })
    }
    pub const fn index(self) -> u16 { self.index }
    pub const fn mask(self) -> u16 { self.mask }
    pub fn advance(&mut self) -> u16 {
        self.index = self.index.wrapping_add(1) & self.mask;
        self.index
    }
}

/// Compile-time layout checks also guard against accidental ABI changes.
pub const fn descriptor_alignment() -> usize { align_of::<RxDescriptor>() }
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_qemu_e1000() {
        assert!(supports_device(PCI_VENDOR_INTEL, DEVICE_82540EM));
        assert!(!supports_device(0x1234, DEVICE_82540EM));
        assert!(!supports_device(PCI_VENDOR_INTEL, 0xffff));
    }

    #[test]
    fn descriptor_abi_is_sixteen_bytes() {
        assert_eq!(size_of::<RxDescriptor>(), 16);
        assert_eq!(size_of::<TxDescriptor>(), 16);
    }

    #[test]
    fn ring_config_checks_alignment_and_size() {
        let ring = RingConfig::new(0x1000, 32).unwrap();
        assert_eq!(ring.byte_length, 512);
        assert_eq!(ring.base_registers(), (0x1000, 0));
        assert_eq!(RingConfig::new(0x1001, 32), Err(E1000Error::MisalignedDmaAddress));
        assert_eq!(RingConfig::new(0, 32), Err(E1000Error::NullDmaAddress));
        assert_eq!(RingConfig::new(0x1000, 12), Err(E1000Error::InvalidRingSize));
        assert_eq!(RingConfig::new(u64::MAX - 0xf, 8), Err(E1000Error::DmaAddressTooWide));
    }

    #[test]
    fn ring_cursor_wraps_at_power_of_two_boundary() {
        let mut cursor = RingCursor::new(8).unwrap();
        for expected in 1..8 { assert_eq!(cursor.advance(), expected); }
        assert_eq!(cursor.advance(), 0);
    }
}
