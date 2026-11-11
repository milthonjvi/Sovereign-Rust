//! PIT IRQ-backed coarse clocksource for legacy interrupt fallback.
//!
//! This source advances from the 8254 channel-0 periodic IRQ handler. It is
//! intentionally coarse and is suitable only when a higher-resolution
//! invariant TSC or HPET clocksource is unavailable.

use crate::{
    drivers::timer::pit::{periodic_frequency_hz, periodic_irq_active, periodic_ticks},
};
use super::clock::ClockSource;

/// Coarse monotonic clocksource driven by the legacy PIT IRQ0 tick.
pub struct PitClockSource {
    frequency_hz: u64,
}

impl PitClockSource {
    /// Construct only while the PIT periodic IRQ handler is registered.
    pub fn new() -> Option<Self> {
        if !periodic_irq_active() {
            return None;
        }
        let frequency_hz = periodic_frequency_hz() as u64;
        if frequency_hz == 0 {
            return None;
        }
        Some(Self { frequency_hz })
    }

    /// Return the total number of PIT ticks since IRQ initialization.
    #[inline]
    pub fn ticks() -> u64 {
        periodic_ticks()
    }
}

impl ClockSource for PitClockSource {
    #[inline]
    fn read_ticks(&self) -> u64 {
        periodic_ticks()
    }

    #[inline]
    fn frequency_hz(&self) -> u64 {
        self.frequency_hz
    }

    #[inline]
    fn mask(&self) -> u64 {
        u64::MAX
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clocksource_is_not_available_without_pit_irq() {
        // This test does not initialize hardware; a fresh test process starts
        // with the PIT IRQ inactive.
        assert!(PitClockSource::new().is_none());
    }
}
