//! Allocation-free validation and lookup for systems with multiple I/O APICs.
//!
//! Firmware supplies each controller's GSI base in MADT; the implemented
//! redirection count is read from the controller version register. Keep this
//! topology layer independent of MMIO so it can be exhaustively unit-tested
//! before any interrupt source is routed.

pub const MAX_REDIRECTION_ENTRIES: u32 = 120;

/// A validated controller interval. The GSI interval is half-open.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IoApicGsiRange {
    pub apic_id: u8,
    pub gsi_base: u32,
    pub redirection_count: u32,
}

impl IoApicGsiRange {
    pub const fn end_exclusive(self) -> Option<u32> {
        self.gsi_base.checked_add(self.redirection_count)
    }

    pub const fn contains(self, gsi: u32) -> bool {
        match self.end_exclusive() {
            Some(end) => gsi >= self.gsi_base && gsi < end,
            None => false,
        }
    }
}

/// Index and pin number for a GSI in the caller-provided controller array.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GsiRoute {
    pub controller_index: usize,
    pub pin: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TopologyError {
    EmptyTopology,
    InvalidRedirectionCount,
    GsiRangeOverflow,
    DuplicateControllerId,
    OverlappingGsiRanges,
    GsiNotCovered,
}

/// Validate controller identity and GSI interval invariants.
///
/// Call after reading each controller's actual redirection count. Duplicate
/// IDs and overlapping GSI intervals are rejected instead of selecting an
/// arbitrary controller. No allocation is performed.
pub fn validate_io_apic_ranges(ranges: &[IoApicGsiRange]) -> Result<(), TopologyError> {
    if ranges.is_empty() {
        return Err(TopologyError::EmptyTopology);
    }

    for (index, range) in ranges.iter().copied().enumerate() {
        if range.redirection_count == 0
            || range.redirection_count > MAX_REDIRECTION_ENTRIES
        {
            return Err(TopologyError::InvalidRedirectionCount);
        }
        let end = range.end_exclusive().ok_or(TopologyError::GsiRangeOverflow)?;

        for previous in &ranges[..index] {
            if previous.apic_id == range.apic_id {
                return Err(TopologyError::DuplicateControllerId);
            }
            let previous_end = previous.end_exclusive()
                .ok_or(TopologyError::GsiRangeOverflow)?;
            if range.gsi_base < previous_end && previous.gsi_base < end {
                return Err(TopologyError::OverlappingGsiRanges);
            }
        }
    }
    Ok(())
}

/// Resolve a GSI to exactly one controller and its local redirection pin.
///
/// The ranges should first pass validate_io_apic_ranges. Malformed input is
/// still checked here to avoid arithmetic wrap or ambiguous lookup.
pub fn resolve_gsi(
    gsi: u32,
    ranges: &[IoApicGsiRange],
) -> Result<GsiRoute, TopologyError> {
    let mut found = None;
    for (controller_index, range) in ranges.iter().copied().enumerate() {
        if range.redirection_count == 0
            || range.redirection_count > MAX_REDIRECTION_ENTRIES
        {
            return Err(TopologyError::InvalidRedirectionCount);
        }
        if range.end_exclusive().is_none() {
            return Err(TopologyError::GsiRangeOverflow);
        }
        if range.contains(gsi) {
            if found.is_some() {
                return Err(TopologyError::OverlappingGsiRanges);
            }
            found = Some(GsiRoute {
                controller_index,
                pin: gsi - range.gsi_base,
            });
        }
    }
    found.ok_or(TopologyError::GsiNotCovered)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOPOLOGY: [IoApicGsiRange; 3] = [
        IoApicGsiRange { apic_id: 2, gsi_base: 0, redirection_count: 24 },
        IoApicGsiRange { apic_id: 5, gsi_base: 32, redirection_count: 16 },
        IoApicGsiRange { apic_id: 7, gsi_base: 64, redirection_count: 8 },
    ];

    #[test]
    fn validates_disjoint_ranges_and_resolves_boundaries() {
        assert_eq!(validate_io_apic_ranges(&TOPOLOGY), Ok(()));
        assert_eq!(resolve_gsi(0, &TOPOLOGY),
            Ok(GsiRoute { controller_index: 0, pin: 0 }));
        assert_eq!(resolve_gsi(23, &TOPOLOGY),
            Ok(GsiRoute { controller_index: 0, pin: 23 }));
        assert_eq!(resolve_gsi(32, &TOPOLOGY),
            Ok(GsiRoute { controller_index: 1, pin: 0 }));
        assert_eq!(resolve_gsi(47, &TOPOLOGY),
            Ok(GsiRoute { controller_index: 1, pin: 15 }));
        assert_eq!(resolve_gsi(64, &TOPOLOGY),
            Ok(GsiRoute { controller_index: 2, pin: 0 }));
        assert_eq!(resolve_gsi(72, &TOPOLOGY), Err(TopologyError::GsiNotCovered));
        assert_eq!(resolve_gsi(24, &TOPOLOGY), Err(TopologyError::GsiNotCovered));
    }

    #[test]
    fn rejects_empty_and_invalid_ranges() {
        assert_eq!(validate_io_apic_ranges(&[]), Err(TopologyError::EmptyTopology));
        let zero = [IoApicGsiRange { apic_id: 1, gsi_base: 0, redirection_count: 0 }];
        assert_eq!(validate_io_apic_ranges(&zero),
            Err(TopologyError::InvalidRedirectionCount));
        let oversized = [IoApicGsiRange {
            apic_id: 1, gsi_base: 0, redirection_count: MAX_REDIRECTION_ENTRIES + 1,
        }];
        assert_eq!(validate_io_apic_ranges(&oversized),
            Err(TopologyError::InvalidRedirectionCount));
    }

    #[test]
    fn rejects_overlapping_ranges_and_duplicate_ids() {
        let overlap = [
            IoApicGsiRange { apic_id: 1, gsi_base: 0, redirection_count: 24 },
            IoApicGsiRange { apic_id: 2, gsi_base: 23, redirection_count: 8 },
        ];
        assert_eq!(validate_io_apic_ranges(&overlap),
            Err(TopologyError::OverlappingGsiRanges));

        let duplicate_id = [
            IoApicGsiRange { apic_id: 4, gsi_base: 0, redirection_count: 8 },
            IoApicGsiRange { apic_id: 4, gsi_base: 16, redirection_count: 8 },
        ];
        assert_eq!(validate_io_apic_ranges(&duplicate_id),
            Err(TopologyError::DuplicateControllerId));
    }

    #[test]
    fn accepts_adjacent_ranges_and_detects_overflow() {
        let adjacent = [
            IoApicGsiRange { apic_id: 1, gsi_base: 0, redirection_count: 8 },
            IoApicGsiRange { apic_id: 2, gsi_base: 8, redirection_count: 8 },
        ];
        assert_eq!(validate_io_apic_ranges(&adjacent), Ok(()));

        let overflow = [IoApicGsiRange {
            apic_id: 1, gsi_base: u32::MAX - 3, redirection_count: 8,
        }];
        assert_eq!(validate_io_apic_ranges(&overflow),
            Err(TopologyError::GsiRangeOverflow));
    }
}
