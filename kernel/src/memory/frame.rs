//! Monotonic physical frame allocator for the earliest kernel bootstrap phase.
//!
//! Only UEFI conventional-memory descriptors are consumed. Other descriptor
//! types remain reserved, keeping loader image, page tables, bootstrap stack,
//! handoff buffers, ACPI, MMIO, and firmware-owned regions out of the free pool.

use super::boot_map::{
    BootMemoryError, BootMemoryMap, PhysicalRegion, UsableRegionIter, PAGE_SIZE,
};

pub const LOW_MEMORY_RESERVED_END: u64 = 0x10_0000;
pub const MAX_PHYSICAL_ADDRESS: u64 = 1u64 << 52;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameAllocatorError {
    InvalidPageCount,
    InvalidAlignment,
    AddressOverflow,
    MemoryMap(BootMemoryError),
}

impl From<BootMemoryError> for FrameAllocatorError {
    fn from(error: BootMemoryError) -> Self { Self::MemoryMap(error) }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameAllocation {
    pub start: u64,
    pub page_count: u64,
}

impl FrameAllocation {
    pub const fn byte_len(self) -> u64 { self.page_count * PAGE_SIZE }
    pub const fn end(self) -> u64 { self.start + self.page_count * PAGE_SIZE }
}

pub struct FrameAllocator<'map, 'buffer> {
    regions: UsableRegionIter<'map, 'buffer>,
    current: Option<PhysicalRegion>,
    cursor: u64,
    allocated_pages: u64,
}

impl<'map, 'buffer> FrameAllocator<'map, 'buffer> {
    pub fn new(memory: &'map BootMemoryMap<'map, 'buffer>) -> Self {
        Self {
            regions: memory.usable_regions(),
            current: None,
            cursor: 0,
            allocated_pages: 0,
        }
    }

    pub fn allocate_frame(&mut self) -> Result<Option<FrameAllocation>, FrameAllocatorError> {
        self.allocate_contiguous(1, 1)
    }

    /// Allocate contiguous frames. Alignment is expressed in 4 KiB pages and
    /// must be a non-zero power of two. This bootstrap allocator is monotonic:
    /// it never frees frames and is not intended for general runtime use.
    pub fn allocate_contiguous(
        &mut self,
        page_count: u64,
        alignment_pages: u64,
    ) -> Result<Option<FrameAllocation>, FrameAllocatorError> {
        if page_count == 0 { return Err(FrameAllocatorError::InvalidPageCount); }
        if alignment_pages == 0 || !alignment_pages.is_power_of_two() {
            return Err(FrameAllocatorError::InvalidAlignment);
        }

        let bytes = page_count.checked_mul(PAGE_SIZE)
            .ok_or(FrameAllocatorError::AddressOverflow)?;
        let alignment = alignment_pages.checked_mul(PAGE_SIZE)
            .ok_or(FrameAllocatorError::AddressOverflow)?;

        loop {
            if self.current.is_none() {
                let Some(region) = self.regions.next() else { return Ok(None); };
                let region = region.map_err(FrameAllocatorError::MemoryMap)?;
                let start = region.start.max(LOW_MEMORY_RESERVED_END);
                let Some(start) = align_up(start, PAGE_SIZE) else {
                    return Err(FrameAllocatorError::AddressOverflow);
                };
                self.current = Some(region);
                self.cursor = start;
            }

            let region = self.current.expect("current region set above");
            let region_end = region.end.min(MAX_PHYSICAL_ADDRESS);
            let candidate = match align_up(self.cursor, alignment) {
                Some(candidate) => candidate,
                None => return Err(FrameAllocatorError::AddressOverflow),
            };
            let Some(candidate_end) = candidate.checked_add(bytes) else {
                return Err(FrameAllocatorError::AddressOverflow);
            };

            if candidate_end <= region_end {
                let allocation = FrameAllocation { start: candidate, page_count };
                self.cursor = candidate_end;
                self.allocated_pages = self.allocated_pages.checked_add(page_count)
                    .ok_or(FrameAllocatorError::AddressOverflow)?;
                return Ok(Some(allocation));
            }

            // Never stitch separate EFI descriptors into one allocation.
            self.current = None;
            self.cursor = 0;
        }
    }

    pub const fn allocated_pages(&self) -> u64 { self.allocated_pages }
}

const fn align_up(value: u64, alignment: u64) -> Option<u64> {
    if alignment == 0 || !alignment.is_power_of_two() { return None; }
    match value.checked_add(alignment - 1) {
        Some(rounded) => Some(rounded & !(alignment - 1)),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignment_rounds_to_requested_power_of_two() {
        assert_eq!(align_up(0x10_0001, PAGE_SIZE), Some(0x10_1000));
        assert_eq!(align_up(0x20_0000, PAGE_SIZE), Some(0x20_0000));
        assert_eq!(align_up(0x20_0001, PAGE_SIZE * 8), Some(0x20_8000));
    }

    #[test]
    fn alignment_rejects_invalid_or_overflowing_values() {
        assert_eq!(align_up(0x1000, 0), None);
        assert_eq!(align_up(0x1000, 3), None);
        assert_eq!(align_up(u64::MAX - 7, PAGE_SIZE), None);
    }

    #[test]
    fn allocations_report_exclusive_end_and_size() {
        let allocation = FrameAllocation { start: 0x20_0000, page_count: 8 };
        assert_eq!(allocation.byte_len(), 0x8000);
        assert_eq!(allocation.end(), 0x20_8000);
    }
}
