//! Invariant-TSC clocksource discovery for x86_64.
//!
//! The TSC is accepted only when CPUID advertises invariant TSC and leaf 0x15
//! supplies a complete crystal ratio. Frequency is never guessed from nominal
//! CPU MHz because that value is not a reliable TSC calibration source.

use crate::arch::x86_64::cpuid::cpuid;
use super::clock::ClockSource;

/// Architectural invariant-TSC capability bit in CPUID 0x80000007:EDX.
const INVARIANT_TSC: u32 = 1 << 8;

/// IA32_TSC read-only clocksource.
pub struct TscClockSource {
    frequency_hz: u64,
}

impl TscClockSource {
    /// Discover a stable TSC frequency from CPUID leaf 0x15.
    ///
    /// Returns None when the CPU does not advertise invariant TSC, does not
    /// implement the frequency-ratio leaf, or omits its crystal frequency.
    pub fn detect() -> Option<Self> {
        let max_basic = cpuid(0, 0).eax;
        let max_extended = cpuid(0x8000_0000, 0).eax;
        if max_basic < 0x15 || max_extended < 0x8000_0007 {
            return None;
        }

        let invariant = cpuid(0x8000_0007, 0).edx & INVARIANT_TSC != 0;
        if !invariant {
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

impl ClockSource for TscClockSource {
    #[inline]
    fn read_ticks(&self) -> u64 { Self::read_tsc() }

    #[inline]
    fn frequency_hz(&self) -> u64 { self.frequency_hz }

    #[inline]
    fn mask(&self) -> u64 { u64::MAX }
}
