//! Validated PCI BAR resource ranges.
//!
//! This module does not probe BAR sizes or program hardware. The caller must
//! supply a size obtained from a trusted platform/driver resource-discovery
//! phase. All ranges use an exclusive end address.

use super::config::{Bar, BarKind};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceError {
    InvalidSize,
    MisalignedBase,
    AddressOverflow,
    AddressWidthExceeded,
}

/// A validated PCI BAR aperture. The end address is exclusive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BarResource {
    pub index: u8,
    pub kind: BarKind,
    pub prefetchable: bool,
    pub base: u64,
    pub size: u64,
    pub end: u64,
}

impl BarResource {
    /// Validate a decoded BAR against a known aperture size.
    /// This is pure and performs no configuration-space or MMIO accesses.
    /// The size must come from a trusted sizing mechanism; it cannot be
    /// inferred from the currently programmed BAR address alone.
    pub const fn new(bar: Bar, size: u64) -> Result<Self, ResourceError> {
        if size == 0 || !size.is_power_of_two() {
            return Err(ResourceError::InvalidSize);
        }
        if bar.base & (size - 1) != 0 {
            return Err(ResourceError::MisalignedBase);
        }
        let end = match bar.base.checked_add(size) {
            Some(value) => value,
            None => return Err(ResourceError::AddressOverflow),
        };
        let width_limit = match bar.kind {
            BarKind::MemoryBelow1M => 1u64 << 20,
            BarKind::Memory32 | BarKind::Io => 1u64 << 32,
            BarKind::Memory64 => u64::MAX,
        };
        if end > width_limit {
            return Err(ResourceError::AddressWidthExceeded);
        }
        Ok(Self { index: bar.index, kind: bar.kind, prefetchable: bar.prefetchable, base: bar.base, size, end })
    }

    /// Returns true when an address belongs to this BAR aperture.
    pub const fn contains(self, address: u64) -> bool {
        address >= self.base && address < self.end
    }

    /// Compare ranges; I/O and memory BARs belong to distinct address spaces.
    pub const fn overlaps(self, other: Self) -> bool {
        self.kind_is_io() == other.kind_is_io() && self.base < other.end && other.base < self.end
    }

    const fn kind_is_io(self) -> bool {
        matches!(self.kind, BarKind::Io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory32(base: u64) -> Bar {
        Bar { index: 0, kind: BarKind::Memory32, prefetchable: true, base, raw_low: base as u32, raw_high: None }
    }

    #[test]
    fn validates_range_and_exclusive_end() {
        let resource = BarResource::new(memory32(0x8000_0000), 0x1000).unwrap();
        assert_eq!(resource.end, 0x8000_1000);
        assert!(resource.contains(0x8000_0fff));
        assert!(!resource.contains(0x8000_1000));
    }

    #[test]
    fn rejects_zero_and_non_power_of_two_sizes() {
        assert_eq!(BarResource::new(memory32(0x1000), 0), Err(ResourceError::InvalidSize));
        assert_eq!(BarResource::new(memory32(0x1000), 0x1800), Err(ResourceError::InvalidSize));
    }

    #[test]
    fn rejects_misaligned_base_and_32_bit_overflow() {
        assert_eq!(BarResource::new(memory32(0x1800), 0x1000), Err(ResourceError::MisalignedBase));
        assert_eq!(BarResource::new(memory32(0xffff_f000), 0x2000), Err(ResourceError::AddressWidthExceeded));
    }

    #[test]
    fn validates_below_one_megabyte_limit() {
        let bar = Bar { index: 0, kind: BarKind::MemoryBelow1M, prefetchable: false, base: 0x80000, raw_low: 0x80002, raw_high: None };
        assert!(BarResource::new(bar, 0x10000).is_ok());
        assert_eq!(BarResource::new(bar, 0x20000), Err(ResourceError::AddressWidthExceeded));
    }

    #[test]
    fn overlap_respects_address_spaces() {
        let a = BarResource::new(memory32(0x1000), 0x1000).unwrap();
        let b = BarResource::new(memory32(0x1800), 0x1000).unwrap();
        let io_bar = Bar { index: 1, kind: BarKind::Io, prefetchable: false, base: 0x1800, raw_low: 0x1801, raw_high: None };
        let io = BarResource::new(io_bar, 0x1000).unwrap();
        assert!(a.overlaps(b));
        assert!(!a.overlaps(io));
    }

    #[test]
    fn accepts_64_bit_aperture_above_four_gib() {
        let bar = Bar { index: 0, kind: BarKind::Memory64, prefetchable: false, base: 0x1_0000_0000, raw_low: 0x4, raw_high: Some(1) };
        let resource = BarResource::new(bar, 0x2000).unwrap();
        assert_eq!(resource.end, 0x1_0000_2000);
    }
}
