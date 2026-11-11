//! HPET-backed free-running clocksource adapter.
//!
//! This is the policy bridge between the HPET device driver and the generic
//! monotonic timekeeping layer. The HPET must already be mapped and enabled.

use crate::drivers::timer::Hpet;
use super::clock::ClockSource;

/// Femto­seconds in one second.
const FEMTOSECONDS_PER_SECOND: u64 = 1_000_000_000_000_000;

/// Convert an HPET period (femtoseconds per tick) to its integer frequency.
pub const fn hpet_frequency_hz(period_fs: u32) -> Option<u64> {
    if period_fs == 0 {
        return None;
    }
    let frequency = FEMTOSECONDS_PER_SECOND / period_fs as u64;
    if frequency == 0 { None } else { Some(frequency) }
}

/// Read-only clocksource view over an initialized HPET main counter.
///
/// The borrow prevents the HPET handle from being mutably reconfigured while
/// this adapter exists. Concurrent reads of the free-running MMIO counter are
/// safe on x86_64: they do not mutate device state and each access is volatile.
///
/// # Synchronization
/// The adapter is Sync so the monotonic clock can be read on multiple CPUs.
/// This does not make HPET configuration operations concurrently accessible;
/// those require exclusive access through the original Hpet handle.
pub struct HpetClockSource<'a> {
    hpet: &'a Hpet,
    frequency_hz: u64,
    mask: u64,
}

// SAFETY: The adapter only performs volatile reads of the HPET main counter.
// Its shared borrow prevents Rust-side mutable access to the Hpet handle;
// hardware counter updates are autonomous and read-only from this adapter.
unsafe impl Sync for HpetClockSource<'_> {}

impl<'a> HpetClockSource<'a> {
    /// Create a clocksource only after the HPET main counter is enabled.
    pub fn new(hpet: &'a Hpet) -> Option<Self> {
        if !hpet.is_enabled() {
            return None;
        }

        let capabilities = hpet.capabilities();
        let frequency_hz = hpet_frequency_hz(capabilities.period_fs)?;
        let mask = if capabilities.counter_64_bit { u64::MAX } else { u32::MAX as u64 };

        Some(Self { hpet, frequency_hz, mask })
    }

    pub const fn period_fs(&self) -> u32 {
        self.hpet.capabilities().period_fs
    }
}

impl ClockSource for HpetClockSource<'_> {
    #[inline]
    fn read_ticks(&self) -> u64 {
        self.hpet.read_counter() & self.mask
    }

    #[inline]
    fn frequency_hz(&self) -> u64 {
        self.frequency_hz
    }

    #[inline]
    fn mask(&self) -> u64 {
        self.mask
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_hpet_period_to_integer_frequency() {
        assert_eq!(hpet_frequency_hz(100_000_000), Some(10_000_000));
        assert_eq!(hpet_frequency_hz(10_000_000), Some(100_000_000));
        assert_eq!(hpet_frequency_hz(0), None);
    }
}
