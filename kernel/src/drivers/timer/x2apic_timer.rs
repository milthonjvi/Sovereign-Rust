//! x2APIC local timer countdown driver and calibration.
//!
//! This module provides a one-shot countdown backend for x2APIC systems that
//! do not support the TSC-deadline MSR. Calibration can use HPET or invariant TSC.

use core::{
    marker::PhantomData,
    sync::atomic::{compiler_fence, Ordering},
};

use crate::{
    arch::x86_64::x2apic::{X2Apic, X2ApicError},
    drivers::api::DriverError,
};

use super::{apic::{calibrate_bus_clock_hz, ApicDivide}, hpet::Hpet};

/// x2APIC one-shot countdown timer using the architectural initial-count MSR.
///
/// Unlike TSC-deadline mode, this backend works on x2APIC-capable CPUs that do
/// not advertise the TSC-deadline feature. Its resolution depends on the
/// calibrated local-APIC timer input clock and configured divider.
pub struct X2ApicTimer {
    vector: u8,
    divide: ApicDivide,
    frequency_hz: u64,
    _cpu_local: PhantomData<*mut ()>,
}

impl X2ApicTimer {
    /// Configure the x2APIC local timer in masked one-shot mode.
    ///
    /// # Safety
    /// Must execute at CPL 0 on a CPU whose x2APIC mode is already enabled.
    /// The supplied vector must have an IDT gate installed and a handler
    /// registered before the timer is unmasked.
    pub unsafe fn configure(
        vector: u8,
        divide: ApicDivide,
        frequency_hz: u64,
    ) -> Result<Self, DriverError> {
        if frequency_hz == 0 {
            return Err(DriverError::InvalidConfiguration);
        }
        // SAFETY: Caller guarantees x2APIC is enabled on this CPU.
        let mut apic = unsafe { X2Apic::new() };
        apic.configure_one_shot_timer(vector, divide.encoding())
            .map_err(map_x2apic_error)?;
        Ok(Self { vector, divide, frequency_hz, _cpu_local: PhantomData })
    }

    /// Calibrate the x2APIC countdown against an enabled HPET counter.
    ///
    /// The timer remains masked on return. The bounded polling loop prevents a
    /// stopped HPET from hanging bootstrap. A 20-100 ms sample is recommended.
    ///
    /// # Safety
    /// Must execute at CPL 0 on a CPU with x2APIC enabled. The caller must own
    /// the local timer LVT during calibration and ensure the HPET is mapped,
    /// enabled, and not concurrently reconfigured.
    pub unsafe fn calibrate_with_hpet(
        vector: u8,
        divide: ApicDivide,
        hpet: &Hpet,
        sample_ms: u32,
        max_spin_iterations: usize,
    ) -> Result<Self, DriverError> {
        if !hpet.is_enabled() || sample_ms == 0 || max_spin_iterations == 0 {
            return Err(DriverError::InvalidConfiguration);
        }
        let capabilities = hpet.capabilities();
        let period_fs = capabilities.period_fs;
        if period_fs == 0 {
            return Err(DriverError::InvalidConfiguration);
        }
        let mask = if capabilities.counter_64_bit { u64::MAX } else { u32::MAX as u64 };
        let target_ticks = ((sample_ms as u128)
            .checked_mul(1_000_000_000_000)
            .and_then(|value| value.checked_add(period_fs as u128 - 1))
            .ok_or(DriverError::InvalidConfiguration)?)
            / period_fs as u128;
        if target_ticks == 0 || target_ticks > mask as u128 {
            return Err(DriverError::InvalidConfiguration);
        }

        // SAFETY: Caller guarantees this CPU's x2APIC is initialized.
        let mut apic = unsafe { X2Apic::new() };
        apic.configure_one_shot_timer(vector, divide.encoding())
            .map_err(map_x2apic_error)?;
        let hpet_before_start = hpet.read_counter() & mask;
        if apic.load_masked_one_shot_count(u32::MAX).is_err() {
            apic.stop_one_shot_timer();
            return Err(DriverError::InvalidConfiguration);
        }
        let hpet_after_start = hpet.read_counter() & mask;
        let hpet_start = midpoint_masked(hpet_before_start, hpet_after_start, mask);

        let mut reached_target = false;
        for _ in 0..max_spin_iterations {
            let sample = hpet.read_counter() & mask;
            let elapsed = sample.wrapping_sub(hpet_start) & mask;
            if (elapsed as u128) >= target_ticks {
                reached_target = true;
                break;
            }
            core::hint::spin_loop();
        }
        if !reached_target {
            apic.stop_one_shot_timer();
            return Err(DriverError::HardwareFault);
        }

        let hpet_before_end = hpet.read_counter() & mask;
        let current_count = apic.timer_current_count();
        let hpet_after_end = hpet.read_counter() & mask;
        let hpet_end = midpoint_masked(hpet_before_end, hpet_after_end, mask);
        let elapsed_hpet = hpet_end.wrapping_sub(hpet_start) & mask;
        apic.stop_one_shot_timer();

        let elapsed_apic = u32::MAX.saturating_sub(current_count);
        // A zero current count means the 32-bit one-shot counter exhausted;
        // the measured interval can no longer be trusted as a single sample.
        if current_count == 0 || elapsed_apic == 0 || (elapsed_hpet as u128) < target_ticks {
            return Err(DriverError::InvalidConfiguration);
        }
        let bus_frequency = calibrate_bus_clock_hz(
            elapsed_apic,
            elapsed_hpet,
            period_fs,
            divide,
        )?;
        let frequency_hz = bus_frequency / divide.factor() as u64;
        if frequency_hz == 0 {
            return Err(DriverError::InvalidConfiguration);
        }

        Ok(Self { vector, divide, frequency_hz, _cpu_local: PhantomData })
    }

    /// Calibrate the x2APIC countdown against an invariant TSC.
    ///
    /// This avoids requiring HPET when CPUID or another trusted source has
    /// already supplied an accurate invariant-TSC frequency. The timer
    /// remains masked on return.
    ///
    /// # Safety
    /// Must execute at CPL 0 on a CPU with x2APIC and invariant TSC enabled.
    /// The caller must own the local timer LVT and provide an accurate TSC
    /// frequency for the current CPU.
    pub unsafe fn calibrate_with_tsc(
        vector: u8,
        divide: ApicDivide,
        tsc_frequency_hz: u64,
        sample_ms: u32,
        max_spin_iterations: usize,
    ) -> Result<Self, DriverError> {
        if tsc_frequency_hz == 0 || sample_ms == 0 || max_spin_iterations == 0 {
            return Err(DriverError::InvalidConfiguration);
        }
        let max_extended = crate::arch::x86_64::cpuid::cpuid(0x8000_0000, 0).eax;
        if max_extended < 0x8000_0007
            || crate::arch::x86_64::cpuid::cpuid(0x8000_0007, 0).edx & (1 << 8) == 0
        {
            return Err(DriverError::UnsupportedDevice);
        }
        let target_tsc_ticks = (sample_ms as u128)
            .checked_mul(tsc_frequency_hz as u128)
            .and_then(|value| value.checked_add(999))
            .ok_or(DriverError::InvalidConfiguration)?
            / 1_000;
        if target_tsc_ticks == 0 || target_tsc_ticks > u64::MAX as u128 {
            return Err(DriverError::InvalidConfiguration);
        }

        // SAFETY: Caller guarantees this CPU's x2APIC is initialized.
        let mut apic = unsafe { X2Apic::new() };
        apic.configure_one_shot_timer(vector, divide.encoding())
            .map_err(map_x2apic_error)?;
        let tsc_before_start = read_tsc_serialized();
        compiler_fence(Ordering::SeqCst);
        if apic.load_masked_one_shot_count(u32::MAX).is_err() {
            apic.stop_one_shot_timer();
            return Err(DriverError::InvalidConfiguration);
        }
        compiler_fence(Ordering::SeqCst);
        let tsc_after_start = read_tsc_serialized();
        let tsc_start = midpoint_u64(tsc_before_start, tsc_after_start);

        let mut reached_target = false;
        for _ in 0..max_spin_iterations {
            let elapsed = read_tsc_serialized().wrapping_sub(tsc_start);
            if elapsed as u128 >= target_tsc_ticks {
                reached_target = true;
                break;
            }
            core::hint::spin_loop();
        }
        if !reached_target {
            apic.stop_one_shot_timer();
            return Err(DriverError::HardwareFault);
        }

        let tsc_before_end = read_tsc_serialized();
        compiler_fence(Ordering::SeqCst);
        let current_count = apic.timer_current_count();
        compiler_fence(Ordering::SeqCst);
        let tsc_after_end = read_tsc_serialized();
        let tsc_end = midpoint_u64(tsc_before_end, tsc_after_end);
        apic.stop_one_shot_timer();

        let elapsed_apic = u32::MAX.saturating_sub(current_count);
        let elapsed_tsc = tsc_end.wrapping_sub(tsc_start);
        if current_count == 0 || elapsed_apic == 0 || elapsed_tsc == 0 {
            return Err(DriverError::InvalidConfiguration);
        }
        let bus_frequency = calibrated_bus_clock_hz_from_tsc(
            elapsed_apic,
            elapsed_tsc,
            tsc_frequency_hz,
            divide,
        )?;
        let frequency_hz = bus_frequency / divide.factor() as u64;
        if frequency_hz == 0 {
            return Err(DriverError::InvalidConfiguration);
        }

        Ok(Self { vector, divide, frequency_hz, _cpu_local: PhantomData })
    }

    /// Unmask the configured one-shot LVT after the handler is registered.
    ///
    /// # Safety
    /// Must execute on the owning CPU at CPL 0.
    pub unsafe fn unmask(&mut self) -> Result<(), DriverError> {
        // SAFETY: Caller guarantees x2APIC is enabled on this CPU.
        let mut apic = unsafe { X2Apic::new() };
        apic.unmask_one_shot_timer().map_err(map_x2apic_error)
    }

    /// Arm a relative delay, rounding up so it is never intentionally early.
    ///
    /// Delays longer than the 32-bit APIC counter range are split: this first
    /// arm expires at the maximum representable interval, and deferred code
    /// must re-evaluate and arm the remaining duration.
    ///
    /// # Safety
    /// Must execute on the owning CPU at CPL 0 after unmasking the timer.
    pub unsafe fn arm_after_ns(&mut self, delay_ns: u64) -> Result<u32, DriverError> {
        let count = countdown_count_for_delay_ns(delay_ns, self.frequency_hz)?;
        // SAFETY: Caller guarantees this CPU's x2APIC timer is configured and unmasked.
        let mut apic = unsafe { X2Apic::new() };
        // SAFETY: Caller guarantees the one-shot LVT is configured and unmasked.
        unsafe { apic.arm_one_shot_timer(count) }.map_err(map_x2apic_error)?;
        Ok(count)
    }

    /// Arm an explicit non-zero APIC countdown value.
    ///
    /// # Safety
    /// Must execute on the owning CPU at CPL 0 after unmasking the timer.
    pub unsafe fn arm_count(&mut self, count: u32) -> Result<(), DriverError> {
        // SAFETY: Caller guarantees this CPU's x2APIC timer is configured and unmasked.
        let mut apic = unsafe { X2Apic::new() };
        // SAFETY: Caller guarantees the one-shot LVT is configured and unmasked.
        unsafe { apic.arm_one_shot_timer(count) }.map_err(map_x2apic_error)
    }

    /// Read the current local APIC countdown.
    ///
    /// # Safety
    /// Must execute on the owning CPU at CPL 0.
    pub unsafe fn current_count(&self) -> u32 {
        // SAFETY: Caller guarantees this CPU's x2APIC is enabled.
        let apic = unsafe { X2Apic::new() };
        apic.timer_current_count()
    }

    /// Disarm the countdown while keeping the LVT unmasked for later arms.
    ///
    /// # Safety
    /// Must execute on the owning CPU at CPL 0.
    pub unsafe fn disarm(&mut self) -> Result<(), DriverError> {
        // SAFETY: Caller guarantees this CPU's x2APIC is enabled.
        let mut apic = unsafe { X2Apic::new() };
        apic.disarm_one_shot_timer().map_err(map_x2apic_error)
    }

    /// Stop the countdown and mask its LVT source.
    ///
    /// # Safety
    /// Must execute on the owning CPU at CPL 0.
    pub unsafe fn stop(&mut self) {
        // SAFETY: Caller guarantees this CPU's x2APIC is enabled.
        let mut apic = unsafe { X2Apic::new() };
        apic.stop_one_shot_timer();
    }

    pub const fn vector(&self) -> u8 { self.vector }
    pub const fn divide(&self) -> ApicDivide { self.divide }
    pub const fn frequency_hz(&self) -> u64 { self.frequency_hz }
}

#[inline]
fn read_tsc_serialized() -> u64 {
    let low: u32;
    let high: u32;
    // SAFETY: RDTSC is available on x86_64; LFENCE brackets the sample.
    unsafe {
        core::arch::asm!(
            "lfence",
            "rdtsc",
            "lfence",
            out("eax") low,
            out("edx") high,
            options(nomem, nostack, preserves_flags),
        );
    }
    ((high as u64) << 32) | low as u64
}

#[inline]
fn midpoint_u64(a: u64, b: u64) -> u64 {
    a.wrapping_add(b.wrapping_sub(a) / 2)
}

/// Derive the local-APIC input clock from an invariant-TSC measurement.
fn calibrated_bus_clock_hz_from_tsc(
    apic_elapsed_ticks: u32,
    tsc_elapsed_ticks: u64,
    tsc_frequency_hz: u64,
    divide: ApicDivide,
) -> Result<u64, DriverError> {
    if apic_elapsed_ticks == 0 || tsc_elapsed_ticks == 0 || tsc_frequency_hz == 0 {
        return Err(DriverError::InvalidConfiguration);
    }
    let numerator = (apic_elapsed_ticks as u128)
        .checked_mul(divide.factor() as u128)
        .and_then(|value| value.checked_mul(tsc_frequency_hz as u128))
        .ok_or(DriverError::InvalidConfiguration)?;
    let frequency = numerator / tsc_elapsed_ticks as u128;
    if frequency == 0 || frequency > u64::MAX as u128 {
        return Err(DriverError::InvalidConfiguration);
    }
    Ok(frequency as u64)
}

/// Convert a relative delay into a bounded one-shot APIC countdown.
fn countdown_count_for_delay_ns(
    delay_ns: u64,
    frequency_hz: u64,
) -> Result<u32, DriverError> {
    if frequency_hz == 0 {
        return Err(DriverError::InvalidConfiguration);
    }
    let numerator = (delay_ns as u128)
        .checked_mul(frequency_hz as u128)
        .ok_or(DriverError::InvalidConfiguration)?;
    let ticks = if delay_ns == 0 {
        1
    } else {
        numerator
            .checked_add(999_999_999)
            .ok_or(DriverError::InvalidConfiguration)?
            / 1_000_000_000
    };
    Ok(core::cmp::min(core::cmp::max(1, ticks), u32::MAX as u128) as u32)
}

fn midpoint_masked(a: u64, b: u64, mask: u64) -> u64 {
    a.wrapping_add((b.wrapping_sub(a) & mask) / 2) & mask
}

fn map_x2apic_error(error: X2ApicError) -> DriverError {
    match error {
        X2ApicError::FeatureUnavailable => DriverError::UnsupportedDevice,
        X2ApicError::InvalidPhysicalBase => DriverError::ResourceUnavailable,
        X2ApicError::InvalidVector | X2ApicError::InvalidTimerConfiguration => {
            DriverError::InvalidConfiguration
        }
        X2ApicError::DeliveryTimeout => DriverError::HardwareFault,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::apic::{calibrated_periodic_initial_count, periodic_initial_count};

    #[test]
    fn calculates_periodic_count() {
        assert_eq!(periodic_initial_count(100_000_000, 1000, ApicDivide::By16), Ok(6250));
    }

    #[test]
    fn rejects_zero_frequency() {
        assert_eq!(periodic_initial_count(100_000_000, 0, ApicDivide::By1),
            Err(DriverError::InvalidConfiguration));
    }

    #[test]
    fn calibrates_apic_clock_from_hpet_interval() {
        // 6,250 divided APIC ticks over 1 ms: 6.25 MHz * 16 = 100 MHz.
        assert_eq!(
            calibrate_bus_clock_hz(6_250, 10_000, 100_000_000, ApicDivide::By16),
            Ok(100_000_000)
        );
        assert_eq!(
            calibrated_periodic_initial_count(
                6_250, 10_000, 100_000_000, 1000, ApicDivide::By16
            ),
            Ok(6250)
        );
    }

    #[test]
    fn rejects_invalid_calibration_samples() {
        assert_eq!(calibrate_bus_clock_hz(0, 10, 100_000_000, ApicDivide::By1),
            Err(DriverError::InvalidConfiguration));
        assert_eq!(calibrate_bus_clock_hz(10, 0, 100_000_000, ApicDivide::By1),
            Err(DriverError::InvalidConfiguration));
        assert_eq!(calibrate_bus_clock_hz(10, 10, 0, ApicDivide::By1),
            Err(DriverError::InvalidConfiguration));
    }

    #[test]
    fn rejects_frequency_above_calibrated_clock() {
        // Register access is intentionally not exercised by this pure helper test;
        // configuration validation is covered by the API's explicit preconditions.
        assert_eq!(periodic_initial_count(100_000_000, 1_000_000_000, ApicDivide::By1),
            Err(DriverError::InvalidConfiguration));
    }

    #[test]
    fn calibrates_apic_bus_clock_from_invariant_tsc() {
        // 6,250 APIC ticks / 25,000,000 TSC ticks at 2.5 GHz, divided by 16.
        assert_eq!(
            calibrated_bus_clock_hz_from_tsc(6_250, 25_000_000, 2_500_000_000, ApicDivide::By16),
            Ok(100_000_000)
        );
        assert_eq!(
            calibrated_bus_clock_hz_from_tsc(0, 25_000_000, 2_500_000_000, ApicDivide::By16),
            Err(DriverError::InvalidConfiguration)
        );
    }

    #[test]
    fn tsc_midpoint_handles_wraparound() {
        assert_eq!(midpoint_u64(u64::MAX - 3, 4), 0);
        assert_eq!(midpoint_u64(10, 20), 15);
    }

    #[test]
    fn countdown_delay_conversion_rounds_up_and_saturates() {
        assert_eq!(countdown_count_for_delay_ns(0, 10_000_000), Ok(1));
        assert_eq!(countdown_count_for_delay_ns(101, 10_000_000), Ok(2));
        assert_eq!(countdown_count_for_delay_ns(1_000_000_000, 100_000_000), Ok(100_000_000));
        assert_eq!(countdown_count_for_delay_ns(u64::MAX, 1_000_000_000), Ok(u32::MAX));
        assert_eq!(countdown_count_for_delay_ns(1, 0), Err(DriverError::InvalidConfiguration));
    }

    #[test]
    fn hpet_midpoint_respects_32_bit_counter_wrap() {
        assert_eq!(midpoint_masked(0xFFFF_FFFE, 2, u32::MAX as u64), 0);
        assert_eq!(midpoint_masked(10, 20, u64::MAX), 15);
    }

    #[test]
    fn divide_encodings_match_architecture() {
        assert_eq!(ApicDivide::By1.encoding(), 0b1011);
        assert_eq!(ApicDivide::By128.encoding(), 0b1010);
    }
}
