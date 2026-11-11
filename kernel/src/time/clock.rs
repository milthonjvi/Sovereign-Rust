//! Allocation-free monotonic clocksource abstraction.
//!
//! Hardware drivers expose raw counters. This layer converts them to a
//! nanosecond timeline and clamps concurrent observations so time never moves
//! backwards. A source must not wrap more than once between observations.

use core::sync::atomic::{AtomicU64, Ordering};

/// A readable free-running hardware counter.
/// mask contains the valid counter bits (u64::MAX for 64-bit, u32::MAX for 32-bit).
pub trait ClockSource: Sync {
    fn read_ticks(&self) -> u64;
    fn frequency_hz(&self) -> u64;
    fn mask(&self) -> u64;
}

/// Convert counter ticks to nanoseconds, truncating sub-nanosecond fractions.
pub fn ticks_to_ns(ticks: u64, frequency_hz: u64) -> Option<u64> {
    if frequency_hz == 0 { return None; }
    let ns = (ticks as u128).checked_mul(1_000_000_000)? / frequency_hz as u128;
    if ns > u64::MAX as u128 { None } else { Some(ns as u64) }
}

/// Convert nanoseconds to ticks, rounding up so a delay is never early.
pub fn ns_to_ticks_ceil(ns: u64, frequency_hz: u64) -> Option<u64> {
    if frequency_hz == 0 { return None; }
    let numerator = (ns as u128).checked_mul(frequency_hz as u128)?;
    let ticks = numerator.checked_add(999_999_999)? / 1_000_000_000;
    if ticks > u64::MAX as u128 { None } else { Some(ticks as u64) }
}

/// Lock-free monotonic nanosecond clock backed by a free-running source.
pub struct MonotonicClock<S: ClockSource> {
    source: S,
    base_ticks: u64,
    base_ns: u64,
    last_ns: AtomicU64,
    frequency_hz: u64,
    mask: u64,
}

impl<S: ClockSource> MonotonicClock<S> {
    /// Construct after the source is initialized and its frequency is known.
    pub fn new(source: S, initial_time_ns: u64) -> Option<Self> {
        let frequency_hz = source.frequency_hz();
        let mask = source.mask();
        if frequency_hz == 0 || mask == 0 { return None; }
        let base_ticks = source.read_ticks() & mask;
        Some(Self { source, base_ticks, base_ns: initial_time_ns,
            last_ns: AtomicU64::new(initial_time_ns), frequency_hz, mask })
    }

    /// Read monotonic nanoseconds. Saturates if the timeline overflows.
    pub fn now_ns(&self) -> u64 {
        let current = self.source.read_ticks() & self.mask;
        let elapsed_ticks = current.wrapping_sub(self.base_ticks) & self.mask;
        let elapsed_ns = ticks_to_ns(elapsed_ticks, self.frequency_hz).unwrap_or(u64::MAX);
        let candidate = self.base_ns.saturating_add(elapsed_ns);
        self.last_ns.fetch_max(candidate, Ordering::Relaxed).max(candidate)
    }

    pub const fn frequency_hz(&self) -> u64 { self.frequency_hz }
    pub const fn base_time_ns(&self) -> u64 { self.base_ns }
    pub fn source(&self) -> &S { &self.source }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicU64, Ordering};
    struct MockClock { ticks: AtomicU64, hz: u64, mask: u64 }
    impl ClockSource for MockClock {
        fn read_ticks(&self) -> u64 { self.ticks.load(Ordering::Relaxed) }
        fn frequency_hz(&self) -> u64 { self.hz }
        fn mask(&self) -> u64 { self.mask }
    }
    #[test]
    fn converts_ticks_without_floating_point() {
        assert_eq!(ticks_to_ns(24_000_000, 24_000_000), Some(1_000_000_000));
        assert_eq!(ns_to_ticks_ceil(1_000_001, 24_000_000), Some(24_001));
        assert_eq!(ticks_to_ns(1, 0), None);
    }
    #[test]
    fn tracks_wraparound_and_never_regresses() {
        let source = MockClock { ticks: AtomicU64::new(250), hz: 1_000, mask: 0xff };
        let clock = MonotonicClock::new(source, 7_000_000).unwrap();
        assert_eq!(clock.now_ns(), 7_000_000);
        clock.source().ticks.store(5, Ordering::Relaxed);
        assert_eq!(clock.now_ns(), 7_011_000);
        clock.source().ticks.store(2, Ordering::Relaxed);
        assert_eq!(clock.now_ns(), 7_011_000);
    }
    #[test]
    fn rejects_invalid_clocksource() {
        let source = MockClock { ticks: AtomicU64::new(0), hz: 0, mask: u64::MAX };
        assert!(MonotonicClock::new(source, 0).is_none());
    }
}
