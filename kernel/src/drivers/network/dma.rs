//! Caller-owned DMA buffer descriptors shared by network drivers.
//!
//! This module does not allocate, map, pin, or perform cache maintenance. The
//! platform must supply a CPU-visible slice whose backing pages are pinned and
//! mapped for DMA at the supplied device-visible address for the whole borrow.

use core::ops::{Deref, DerefMut};

pub const E1000_RX_BUFFER_MIN_SIZE: usize = 2048;
pub const DMA_BUFFER_ALIGNMENT: u64 = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DmaBufferError {
    Empty,
    MisalignedAddress,
    AddressRangeOverflow,
    OverlappingBuffers,
    RxBufferTooSmall,
}

/// A bounded CPU view paired with its device-visible DMA address.
///
/// The lifetime keeps the backing slice exclusively borrowed while this
/// descriptor exists. It does not prove that the physical mapping is valid;
/// that guarantee belongs to the platform DMA allocator.
pub struct DmaBuffer<'a> {
    dma_address: u64,
    bytes: &'a mut [u8],
}

impl<'a> DmaBuffer<'a> {
    pub fn new(dma_address: u64, bytes: &'a mut [u8]) -> Result<Self, DmaBufferError> {
        if bytes.is_empty() {
            return Err(DmaBufferError::Empty);
        }
        if dma_address == 0 || dma_address % DMA_BUFFER_ALIGNMENT != 0 {
            return Err(DmaBufferError::MisalignedAddress);
        }
        dma_address.checked_add(bytes.len() as u64)
            .ok_or(DmaBufferError::AddressRangeOverflow)?;
        Ok(Self { dma_address, bytes })
    }

    pub const fn dma_address(&self) -> u64 {
        self.dma_address
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn as_slice(&self) -> &[u8] {
        self.bytes
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        self.bytes
    }

    /// Validate a receive pool before its addresses are installed in RX
    /// descriptors. Every buffer must meet the legacy E1000 2 KiB minimum and
    /// device-visible ranges must be disjoint.
    pub fn validate_rx_pool(pool: &[Self]) -> Result<(), DmaBufferError> {
        for buffer in pool {
            if buffer.len() < E1000_RX_BUFFER_MIN_SIZE {
                return Err(DmaBufferError::RxBufferTooSmall);
            }
            buffer.dma_address.checked_add(buffer.len() as u64)
                .ok_or(DmaBufferError::AddressRangeOverflow)?;
        }
        for left in 0..pool.len() {
            let left_start = pool[left].dma_address;
            let left_end = left_start + pool[left].len() as u64;
            for right in (left + 1)..pool.len() {
                let right_start = pool[right].dma_address;
                let right_end = right_start + pool[right].len() as u64;
                if left_start < right_end && right_start < left_end {
                    return Err(DmaBufferError::OverlappingBuffers);
                }
            }
        }
        Ok(())
    }
}

impl Deref for DmaBuffer<'_> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl DerefMut for DmaBuffer<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.as_mut_slice()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_cpu_slice_with_dma_address() {
        let mut bytes = [0u8; 2048];
        let mut buffer = DmaBuffer::new(0x4000, &mut bytes).unwrap();
        buffer[0] = 0xaa;
        assert_eq!(buffer.dma_address(), 0x4000);
        assert_eq!(buffer.as_slice()[0], 0xaa);
    }

    #[test]
    fn rejects_empty_and_misaligned_buffers() {
        let mut empty = [];
        assert_eq!(DmaBuffer::new(0x4000, &mut empty).err(), Some(DmaBufferError::Empty));
        let mut bytes = [0u8; 32];
        assert_eq!(DmaBuffer::new(0x4001, &mut bytes).err(), Some(DmaBufferError::MisalignedAddress));
    }

    #[test]
    fn validates_rx_capacity_and_physical_non_overlap() {
        let mut a = [0u8; E1000_RX_BUFFER_MIN_SIZE];
        let mut b = [0u8; E1000_RX_BUFFER_MIN_SIZE];
        let pool = [
            DmaBuffer::new(0x4000, &mut a).unwrap(),
            DmaBuffer::new(0x4800, &mut b).unwrap(),
        ];
        assert_eq!(DmaBuffer::validate_rx_pool(&pool), Ok(()));
    }

    #[test]
    fn rejects_overlapping_dma_ranges() {
        let mut a = [0u8; E1000_RX_BUFFER_MIN_SIZE];
        let mut b = [0u8; E1000_RX_BUFFER_MIN_SIZE];
        let pool = [
            DmaBuffer::new(0x4000, &mut a).unwrap(),
            DmaBuffer::new(0x4800, &mut b).unwrap(),
        ];
        assert_eq!(DmaBuffer::validate_rx_pool(&pool), Err(DmaBufferError::OverlappingBuffers));
    }

    #[test]
    fn rejects_rx_buffers_smaller_than_2kib() {
        let mut bytes = [0u8; 1024];
        let pool = [DmaBuffer::new(0x4000, &mut bytes).unwrap()];
        assert_eq!(DmaBuffer::validate_rx_pool(&pool), Err(DmaBufferError::RxBufferTooSmall));
    }
}
