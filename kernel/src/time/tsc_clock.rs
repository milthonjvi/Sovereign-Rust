//! Invariant-TSC clocksource discovery for x86_64.
//!
//! The TSC is accepted only when CPUID advertises invariant TSC. Frequency is
//! taken from a complete CPUID 0x15 crystal ratio when available, otherwise it
//! can be calibrated against an enabled HPET. Nominal CPU MHz is never guessed.

use crate::{arch::x86_64::cpuid::cpuid, drivers::timer::Hpet};
use super::clock::ClockSource;

/// Architectural invariant-TSC capability bit in CPUID 0x80000007:EDX.
const INVARIANT_TSC: u32 = 1 << 8;

/// IA32_TSC read-only clocksource.
pub struct TscClockSource {
    frequency_hz: u64,
}

impl TscClockSource {
    /// Prefer the architectural CPUID ratio and fall back to bounded HPET calibration.
    ///
    /// HPET is consulted only when CPUID leaf 0x15 cannot provide a complete
    /// frequency ratio.
    pub fn detect_or_calibrate(
        hpet: Option<&Hpet>,
        sample_ms: u32,
        max_spin_iterations: usize,
    ) -> Option<Self> {
        Self::detect().or_else(|| {
            hpet.and_then(|device| {
                Self::calibrate_with_hpet(device, sample_ms, max_spin_iterations)
            })
        })
    }

    /// Calibrate invariant TSC against an enabled HPET main counter.
    ///
    /// This is the fallback when CPUID leaf 0x15 does not provide a complete
    /// crystal ratio. The bounded spin limit prevents a broken/stopped HPET
    /// from hanging bootstrap forever.
    ///
    /// `sample_ms` controls the measurement interval; 20-100 ms is generally
    /// a useful balance between bootstrap latency and measurement precision.
    pub fn calibrate_with_hpet(
        hpet: &Hpet,
        sample_ms: u32,
        max_spin_iterations: usize,
    ) -> Option<Self> {
        if !invariant_tsc_supported() || !hpet.is_enabled()
            || sample_ms == 0 || max_spin_iterations == 0
        {
            return None;
        }

        let capabilities = hpet.capabilities();
        let period_fs = capabilities.period_fs;
        if period_fs == 0 {
            return None;
        }
        let mask = if capabilities.counter_64_bit { u64::MAX } else { u32::MAX as u64 };
        let target_ticks = ((sample_ms as u128)
            .checked_mul(1_000_000_000_000)?
            .checked_add(period_fs as u128 - 1)?)
            / period_fs as u128;
        if target_ticks == 0 || target_ticks > mask as u128 {
            return None;
        }

        // Bracket each HPET sample with TSC reads and use their midpoint to
        // reduce the fixed ordering error between the two independent counters.
        let tsc_before_start = Self::read_tsc();
        let hpet_start = hpet.read_counter() & mask;
        let tsc_after_start = Self::read_tsc();
        let tsc_start = midpoint(tsc_before_start, tsc_after_start);

        let mut elapsed_hpet = 0u64;
        let mut hpet_end = hpet_start;
        for _ in 0..max_spin_iterations {
            hpet_end = hpet.read_counter() & mask;
            elapsed_hpet = hpet_end.wrapping_sub(hpet_start) & mask;
            if elapsed_hpet as u128 >= target_ticks {
                break;
            }
            core::hint::spin_loop();
        }
        if elapsed_hpet as u128 < target_ticks {
            return None;
        }

        let tsc_before_end = Self::read_tsc();
        hpet_end = hpet.read_counter() & mask;
        elapsed_hpet = hpet_end.wrapping_sub(hpet_start) & mask;
        let tsc_after_end = Self::read_tsc();
        let tsc_end = midpoint(tsc_before_end, tsc_after_end);
        if elapsed_hpet as u128 < target_ticks {
            return None;
        }
        let elapsed_tsc = tsc_end.wrapping_sub(tsc_start);
        if elapsed_tsc == 0 {
            return None;
        }

        Some(Self {
            frequency_hz: calibrated_frequency(elapsed_tsc, elapsed_hpet, period_fs)?,
        })
    }

    /// Discover a stable TSC frequency from CPUID leaf 0x15.
    ///
    /// Returns None when the CPU does not advertise invariant TSC, does not
    /// implement the frequency-ratio leaf, or omits its crystal frequency.
    pub fn detect() -> Option<Self> {
        let max_basic = cpuid(0, 0).eax;
        if max_basic < 0x15 || !invariant_tsc_supported() {
            return None;
        }

        let ratio = cpuid(0x15, 0);
        if ratio.eax == 0 || ratio.ebx == 0 || ratio.ecx == 0 {
            return None;
        }

        let frequency = (ratio.ecx as u128)
            .checked_mul(ratio.ebx as u128)?
            / ratio.eax as u128;
        if frequency == 0 || frequency > u64::MAX as u128 {
            return None;
        }

        Some(Self { frequency_hz: frequency as u64 })
    }

    /// Construct from an externally calibrated TSC frequency.
    ///
    /// # Safety
    /// The caller must guarantee that the frequency is measured accurately,
    /// the TSC is invariant, and the counter is suitable for the intended
    /// CPU-sharing policy (including cross-CPU synchronization where needed).
    pub const unsafe fn from_calibrated_frequency(frequency_hz: u64) -> Option<Self> {
        if frequency_hz == 0 { None } else { Some(Self { frequency_hz }) }
    }

    /// Read the serialized timestamp counter.
    #[inline]
    pub fn read_tsc() -> u64 {
        let low: u32;
        let high: u32;
        // LFENCE orders prior loads before RDTSC and subsequent loads after it.
        // SAFETY: RDTSC is available in x86_64 user and kernel mode.
        unsafe {
            core::arch::asm!(
                "lfence",
                "rdtsc",
                "lfence",
                out("eax") low,
                out("edx") high,
                options(nomem, nostack)
            );
        }
        ((high as u64) << 32) | low as u64
    }
}


/// Whether CPUID advertises an invariant TSC.
#[inline]
fn invariant_tsc_supported() -> bool {
    cpuid(0x8000_0000, 0).eax >= 0x8000_0007
        && cpuid(0x8000_0007, 0).edx & INVARIANT_TSC != 0
}

/// Midpoint of two adjacent samples, avoiding overflow.
#[inline]
fn midpoint(a: u64, b: u64) -> u64 {
    a.wrapping_add(b.wrapping_sub(a) / 2)
}

/// Convert paired HPET/TSC deltas into an integer TSC frequency.
fn calibrated_frequency(tsc_ticks: u64, hpet_ticks: u64, period_fs: u32) -> Option<u64> {
    if tsc_ticks == 0 || hpet_ticks == 0 || period_fs == 0 {
        return None;
    }
    let elapsed_fs = (hpet_ticks as u128).checked_mul(period_fs as u128)?;
    let frequency = (tsc_ticks as u128)
        .checked_mul(1_000_000_000_000_000u128)?
        / elapsed_fs;
    if frequency == 0 || frequency > u64::MAX as u128 { None } else { Some(frequency as u64) }
}

impl ClockSource for TscClockSource {
    #[inline]
    fn read_ticks(&self) -> u64 { Self::read_tsc() }

    #[inline]
    fn frequency_hz(&self) -> u64 { self.frequency_hz }

    #[inline]
    fn mask(&self) -> u64 { u64::MAX }
}

/// Convert an absolute monotonic deadline to the corresponding absolute TSC
/// deadline using a simultaneous monotonic/TSC sample.
///
/// Returns None for an infinite deadline, zero frequency, or arithmetic
/// overflow. Expired deadlines are mapped to the next TSC tick, never zero
/// (zero has the architectural meaning "disarmed").
pub fn monotonic_deadline_to_tsc(
    deadline_ns: u64,
    now_ns: u64,
    now_tsc: u64,
    tsc_frequency_hz: u64,
) -> Option<u64> {
    if deadline_ns == u64::MAX || tsc_frequency_hz == 0 {
        return None;
    }
    let remaining_ns = deadline_ns.saturating_sub(now_ns);
    let delta = if remaining_ns == 0 {
        1
    } else {
        let numerator = (remaining_ns as u128).checked_mul(tsc_frequency_hz as u128)?;
        let ticks = numerator.checked_add(999_999_999)? / 1_000_000_000;
        if ticks > u64::MAX as u128 { return None; }
        core::cmp::max(1, ticks as u64)
    };
    let absolute = now_tsc.checked_add(delta)?;
    if absolute == 0 { None } else { Some(absolute) }
}

#[cfg(test)]
mod deadline_conversion_tests {
    use super::*;

    #[test]
    fn converts_relative_monotonic_deadline_to_absolute_tsc() {
        assert_eq!(
            monotonic_deadline_to_tsc(2_000_000_000, 1_000_000_000, 500, 2_500_000_000),
            Some(2_500_000_500)
        );
    }

    #[test]
    fn expired_deadline_is_armed_for_next_tsc_tick() {
        assert_eq!(monotonic_deadline_to_tsc(10, 20, 100, 1_000_000_000), Some(101));
    }

    #[test]
    fn infinite_or_unrepresentable_deadlines_are_rejected() {
        assert_eq!(monotonic_deadline_to_tsc(u64::MAX, 0, 1, 1_000_000_000), None);
        assert_eq!(monotonic_deadline_to_tsc(1, 0, u64::MAX, 1_000_000_000), None);
    }
}

#[cfg(test)]
mod calibration_tests {
    use super::*;

    #[test]
    fn converts_hpet_tsc_sample_to_frequency() {
        assert_eq!(calibrated_frequency(2_500_000, 10_000, 100_000_000), Some(2_500_000_000));
        assert_eq!(calibrated_frequency(0, 10_000, 100_000_000), None);
        assert_eq!(calibrated_frequency(2_500_000, 0, 100_000_000), None);
        assert_eq!(calibrated_frequency(2_500_000, 10_000, 0), None);
    }

    #[test]
    fn midpoint_handles_counter_wrap_without_overflow() {
        assert_eq!(midpoint(u64::MAX - 3, 4), 0);
        assert_eq!(midpoint(10, 20), 15);
    }
}
