//! Safe normalization of the UEFI memory map for early physical-memory management.
//!
//! Only EFI_CONVENTIONAL_MEMORY (type 7) is exposed as immediately allocatable.
//! Loader, boot-services, ACPI, MMIO, NVS, and reserved ranges remain unavailable
//! until their individual lifetime and ownership rules are implemented.

use crate::boot::sovereign::{EfiMemoryMap, SovereignBootError};

pub const PAGE_SIZE: u64 = 4096;
pub const EFI_CONVENTIONAL_MEMORY: u32 = 7;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootMemoryError {
    EmptyMap,
    Descriptor(SovereignBootError),
    ZeroPageCount,
    MisalignedPhysicalStart,
    RangeOverflow,
    OverlappingOrUnorderedDescriptors,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PhysicalRegion {
    /// Inclusive physical start address.
    pub start: u64,
    /// Exclusive physical end address.
    pub end: u64,
    pub page_count: u64,
}

impl PhysicalRegion {
    pub const fn byte_len(self) -> u64 {
        self.end - self.start
    }
}

pub struct BootMemoryMap<'map, 'buffer> {
    descriptors: &'map EfiMemoryMap<'buffer>,
}

impl<'map, 'buffer> BootMemoryMap<'map, 'buffer> {
    pub fn new(
        descriptors: &'map EfiMemoryMap<'buffer>,
    ) -> Result<Self, BootMemoryError> {
        if descriptors.is_empty() {
            return Err(BootMemoryError::EmptyMap);
        }
        Ok(Self { descriptors })
    }

    /// Iterate only conventional RAM ranges that are safe to consider for
    /// immediate frame allocation. All other EFI types are intentionally held.
    pub fn usable_regions(&self) -> UsableRegionIter<'map, 'buffer> {
        UsableRegionIter {
            descriptors: self.descriptors,
            next_index: 0,
            failed: false,
            previous_end: None,
        }
    }

    /// Summarize allocatable RAM without allocating or retaining temporary data.
    pub fn usable_summary(&self) -> Result<MemoryMapSummary, BootMemoryError> {
        let mut summary = MemoryMapSummary::default();
        for region in self.usable_regions() {
            let region = region?;
            summary.region_count = summary.region_count.checked_add(1)
                .ok_or(BootMemoryError::RangeOverflow)?;
            summary.page_count = summary.page_count.checked_add(region.page_count)
                .ok_or(BootMemoryError::RangeOverflow)?;
            summary.byte_count = summary.byte_count.checked_add(region.byte_len())
                .ok_or(BootMemoryError::RangeOverflow)?;
        }
        Ok(summary)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MemoryMapSummary {
    pub region_count: u64,
    pub page_count: u64,
    pub byte_count: u64,
}

pub struct UsableRegionIter<'map, 'buffer> {
    descriptors: &'map EfiMemoryMap<'buffer>,
    next_index: usize,
    failed: bool,
    previous_end: Option<u64>,
}

impl Iterator for UsableRegionIter<'_, '_> {
    type Item = Result<PhysicalRegion, BootMemoryError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }

        while self.next_index < self.descriptors.len() {
            let index = self.next_index;
            self.next_index += 1;

            let descriptor = match self.descriptors.descriptor(index) {
                Ok(descriptor) => descriptor,
                Err(error) => {
                    self.failed = true;
                    return Some(Err(BootMemoryError::Descriptor(error)));
                }
            };

            // Validate every descriptor before trusting its type. A malformed
            // non-usable descriptor must not silently hide corrupt map data.
            let region = match physical_region(
                descriptor.physical_start,
                descriptor.number_of_pages,
            ) {
                Ok(region) => region,
                Err(error) => {
                    self.failed = true;
                    return Some(Err(error));
                }
            };

            // UEFI returns descriptors in ascending physical-address order.
            // Reject overlaps and reordered entries rather than risking double
            // allocation of the same physical pages.
            if self.previous_end.is_some_and(|end| region.start < end) {
                self.failed = true;
                return Some(Err(BootMemoryError::OverlappingOrUnorderedDescriptors));
            }
            self.previous_end = Some(region.end);

            if descriptor.memory_type == EFI_CONVENTIONAL_MEMORY {
                return Some(Ok(region));
            }
        }

        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.descriptors.len().saturating_sub(self.next_index);
        (0, Some(remaining))
    }
}

fn physical_region(start: u64, pages: u64) -> Result<PhysicalRegion, BootMemoryError> {
    if pages == 0 {
        return Err(BootMemoryError::ZeroPageCount);
    }
    if start & (PAGE_SIZE - 1) != 0 {
        return Err(BootMemoryError::MisalignedPhysicalStart);
    }
    let byte_len = pages.checked_mul(PAGE_SIZE)
        .ok_or(BootMemoryError::RangeOverflow)?;
    let end = start.checked_add(byte_len)
        .ok_or(BootMemoryError::RangeOverflow)?;
    Ok(PhysicalRegion {
        start,
        end,
        page_count: pages,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_region_uses_exclusive_end() {
        let region = physical_region(0x20_0000, 16).unwrap();
        assert_eq!(region.start, 0x20_0000);
        assert_eq!(region.end, 0x21_0000);
        assert_eq!(region.byte_len(), 0x1_0000);
    }

    #[test]
    fn rejects_empty_and_unaligned_ranges() {
        assert_eq!(physical_region(0x1000, 0), Err(BootMemoryError::ZeroPageCount));
        assert_eq!(
            physical_region(0x1001, 1),
            Err(BootMemoryError::MisalignedPhysicalStart),
        );
    }

    #[test]
    fn rejects_overflowing_ranges() {
        assert_eq!(
            physical_region(u64::MAX & !(PAGE_SIZE - 1), 1),
            Err(BootMemoryError::RangeOverflow),
        );
        assert_eq!(
            physical_region(0, u64::MAX),
            Err(BootMemoryError::RangeOverflow),
        );
    }
}
