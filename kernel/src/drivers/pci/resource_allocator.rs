//! Fixed-capacity PCI BAR resource allocation from platform-provided apertures.
//!
//! The platform supplies already-decoded host-bridge/bridge windows. This
//! allocator does not parse ACPI, map MMIO, or program BAR configuration space.

use super::config::{BarKind, PciAddress};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceSpace {
    Io,
    Memory,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AllocationError {
    InvalidWindow,
    OverlappingWindows,
    InvalidRequest,
    OutOfSpace,
    AddressOverflow,
}

/// One available host-bridge or bridge aperture. `end` is exclusive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceWindow {
    pub space: ResourceSpace,
    pub prefetchable: bool,
    pub base: u64,
    pub end: u64,
    cursor: u64,
}

impl ResourceWindow {
    pub const fn new(
        space: ResourceSpace,
        prefetchable: bool,
        base: u64,
        size: u64,
    ) -> Result<Self, AllocationError> {
        if size == 0 || (matches!(space, ResourceSpace::Io) && prefetchable) {
            return Err(AllocationError::InvalidWindow);
        }
        let end = match base.checked_add(size) {
            Some(value) => value,
            None => return Err(AllocationError::AddressOverflow),
        };
        Ok(Self { space, prefetchable, base, end, cursor: base })
    }

    pub const fn remaining(self) -> u64 {
        self.end - self.cursor
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceRequest {
    pub address: PciAddress,
    pub bar_index: u8,
    pub kind: BarKind,
    pub prefetchable: bool,
    pub size: u64,
}

impl ResourceRequest {
    pub const fn new(
        address: PciAddress,
        bar_index: u8,
        kind: BarKind,
        prefetchable: bool,
        size: u64,
    ) -> Result<Self, AllocationError> {
        if size == 0 || !size.is_power_of_two() || (matches!(kind, BarKind::Io) && prefetchable) {
            return Err(AllocationError::InvalidRequest);
        }
        Ok(Self { address, bar_index, kind, prefetchable, size })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceAllocation {
    pub address: PciAddress,
    pub bar_index: u8,
    pub kind: BarKind,
    pub prefetchable: bool,
    pub base: u64,
    pub size: u64,
    pub end: u64,
}

/// Allocation policy over caller-owned windows; no heap allocation is used.
pub struct ResourcePool<'a> {
    windows: &'a mut [ResourceWindow],
}

impl<'a> ResourcePool<'a> {
    pub fn new(windows: &'a mut [ResourceWindow]) -> Result<Self, AllocationError> {
        for index in 0..windows.len() {
            if windows[index].base >= windows[index].end
                || windows[index].cursor < windows[index].base
                || windows[index].cursor > windows[index].end
                || (windows[index].space == ResourceSpace::Io && windows[index].prefetchable)
            {
                return Err(AllocationError::InvalidWindow);
            }
            for previous in 0..index {
                let a = windows[index];
                let b = windows[previous];
                if a.space == b.space && a.base < b.end && b.base < a.end {
                    return Err(AllocationError::OverlappingWindows);
                }
            }
        }
        Ok(Self { windows })
    }

    /// Allocate the best-fitting suitable aperture, preserving larger windows
    /// where possible. Alignment is always the BAR size, as required by PCI.
    pub fn allocate(&mut self, request: ResourceRequest) -> Result<ResourceAllocation, AllocationError> {
        if request.size == 0
            || !request.size.is_power_of_two()
            || (request.kind == BarKind::Io && request.prefetchable)
        {
            return Err(AllocationError::InvalidRequest);
        }
        let space = if request.kind == BarKind::Io { ResourceSpace::Io } else { ResourceSpace::Memory };
        let width_limit = match request.kind {
            BarKind::MemoryBelow1M => 1u64 << 20,
            BarKind::Io | BarKind::Memory32 => 1u64 << 32,
            BarKind::Memory64 => u64::MAX,
        };

        let mut best: Option<(usize, u64, u64, u64)> = None;
        for (index, window) in self.windows.iter().enumerate() {
            if window.space != space || window.prefetchable != request.prefetchable {
                continue;
            }
            let aligned = match window.cursor.checked_add(request.size - 1) {
                Some(value) => value & !(request.size - 1),
                None => continue,
            };
            let end = match aligned.checked_add(request.size) {
                Some(value) => value,
                None => continue,
            };
            if aligned < window.base || end > window.end || end > width_limit {
                continue;
            }
            let waste = window.end - end;
            if best.map_or(true, |(_, _, _, best_waste)| waste < best_waste) {
                best = Some((index, aligned, end, waste));
            }
        }

        let (index, base, end, _) = best.ok_or(AllocationError::OutOfSpace)?;
        self.windows[index].cursor = end;
        Ok(ResourceAllocation {
            address: request.address,
            bar_index: request.bar_index,
            kind: request.kind,
            prefetchable: request.prefetchable,
            base,
            size: request.size,
            end,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocates_aligned_resources_and_prefers_best_fit() {
        let mut windows = [
            ResourceWindow::new(ResourceSpace::Memory, false, 0x8000_0000, 0x1000_0000).unwrap(),
            ResourceWindow::new(ResourceSpace::Memory, false, 0x9000_0000, 0x4000).unwrap(),
        ];
        let mut pool = ResourcePool::new(&mut windows).unwrap();
        let address = PciAddress { segment: 0, bus: 0, device: 1, function: 0 };
        let request = ResourceRequest::new(address, 0, BarKind::Memory32, false, 0x2000).unwrap();
        let allocation = pool.allocate(request).unwrap();
        assert_eq!(allocation.base, 0x9000_0000);
        assert_eq!(allocation.end, 0x9000_2000);
    }

    #[test]
    fn keeps_io_and_memory_apertures_separate() {
        let mut windows = [
            ResourceWindow::new(ResourceSpace::Io, false, 0x1000, 0x1000).unwrap(),
            ResourceWindow::new(ResourceSpace::Memory, false, 0x1000, 0x1000).unwrap(),
        ];
        let mut pool = ResourcePool::new(&mut windows).unwrap();
        let address = PciAddress { segment: 0, bus: 0, device: 1, function: 0 };
        let io = pool.allocate(ResourceRequest::new(address, 0, BarKind::Io, false, 0x100).unwrap()).unwrap();
        let memory = pool.allocate(ResourceRequest::new(address, 1, BarKind::Memory32, false, 0x100).unwrap()).unwrap();
        assert_eq!(io.base, 0x1000);
        assert_eq!(memory.base, 0x1000);
    }

    #[test]
    fn enforces_prefetchability_and_address_width() {
        let mut windows = [
            ResourceWindow::new(ResourceSpace::Memory, true, 0x80000, 0x80000).unwrap(),
            ResourceWindow::new(ResourceSpace::Memory, false, 0x1_0000_0000, 0x1_0000).unwrap(),
        ];
        let mut pool = ResourcePool::new(&mut windows).unwrap();
        let address = PciAddress { segment: 0, bus: 0, device: 1, function: 0 };
        let below_1m = ResourceRequest::new(address, 0, BarKind::MemoryBelow1M, true, 0x1000).unwrap();
        assert_eq!(pool.allocate(below_1m).unwrap().base, 0x80000);
        let memory32 = ResourceRequest::new(address, 1, BarKind::Memory32, false, 0x1000).unwrap();
        assert_eq!(pool.allocate(memory32), Err(AllocationError::OutOfSpace));
    }

    #[test]
    fn rejects_overlapping_windows() {
        let mut windows = [
            ResourceWindow::new(ResourceSpace::Memory, false, 0x1000, 0x4000).unwrap(),
            ResourceWindow::new(ResourceSpace::Memory, true, 0x3000, 0x4000).unwrap(),
        ];
        assert!(matches!(ResourcePool::new(&mut windows), Err(AllocationError::OverlappingWindows)));
    }
}
