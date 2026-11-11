//! Local APIC timer register operations and calibration helpers.
//!
//! The local APIC must already be enabled, its MMIO page mapped, and its
//! interrupt vector installed before the timer is unmasked. TSC-deadline MSR
//! programming remains the responsibility of the architecture/MSR layer.

use core::{marker::PhantomData, ptr::{read_volatile, write_volatile, NonNull}};
use crate::drivers::api::DriverError;

pub const LVT_TIMER: usize = 0x320;
pub const INITIAL_COUNT: usize = 0x380;
pub const CURRENT_COUNT: usize = 0x390;
pub const DIVIDE_CONFIGURATION: usize = 0x3e0;

const LVT_MASKED: u32 = 1 << 16;
const LVT_MODE_PERIODIC: u32 = 1 << 17;
const LVT_MODE_TSC_DEADLINE: u32 = 1 << 18;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApicTimerMode { OneShot, Periodic, TscDeadline }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApicDivide { By1, By2, By4, By8, By16, By32, By64, By128 }

impl ApicDivide {
    pub const fn encoding(self) -> u8 {
        match self {
            Self::By2 => 0b0000, Self::By4 => 0b0001, Self::By8 => 0b0010,
            Self::By16 => 0b0011, Self::By32 => 0b1000, Self::By64 => 0b1001,
            Self::By128 => 0b1010, Self::By1 => 0b1011,
        }
    }

    pub const fn factor(self) -> u32 {
        match self {
            Self::By1 => 1, Self::By2 => 2, Self::By4 => 4, Self::By8 => 8,
            Self::By16 => 16, Self::By32 => 32, Self::By64 => 64, Self::By128 => 128,
        }
    }
}

/// Calculate the initial-count register value for a periodic APIC timer.
pub fn periodic_initial_count(
    bus_clock_hz: u64,
    requested_hz: u32,
    divide: ApicDivide,
) -> Result<u32, DriverError> {
    if bus_clock_hz == 0 || requested_hz == 0 {
        return Err(DriverError::InvalidConfiguration);
    }
    let denominator = (requested_hz as u64)
        .checked_mul(divide.factor() as u64)
        .ok_or(DriverError::InvalidConfiguration)?;
    let count = bus_clock_hz / denominator;
    if count == 0 || count > u32::MAX as u64 {
        return Err(DriverError::InvalidConfiguration);
    }
    Ok(count as u32)
}

/// Derive the local-APIC timer input clock from a measured APIC countdown
/// and an HPET main-counter interval.
///
/// `apic_elapsed_ticks` is the number of down-counter ticks elapsed during
/// `hpet_elapsed_ticks` HPET ticks. The HPET period is expressed in
/// femtoseconds per tick. The selected APIC divider is applied to recover the
/// timer input clock before division.
pub fn calibrate_bus_clock_hz(
    apic_elapsed_ticks: u32,
    hpet_elapsed_ticks: u64,
    hpet_period_fs: u32,
    divide: ApicDivide,
) -> Result<u64, DriverError> {
    if apic_elapsed_ticks == 0 || hpet_elapsed_ticks == 0 || hpet_period_fs == 0 {
        return Err(DriverError::InvalidConfiguration);
    }

    let numerator = (apic_elapsed_ticks as u128)
        .checked_mul(divide.factor() as u128)
        .and_then(|ticks| ticks.checked_mul(1_000_000_000_000_000u128))
        .ok_or(DriverError::InvalidConfiguration)?;
    let denominator = (hpet_elapsed_ticks as u128)
        .checked_mul(hpet_period_fs as u128)
        .ok_or(DriverError::InvalidConfiguration)?;
    let frequency = numerator / denominator;
    if frequency == 0 || frequency > u64::MAX as u128 {
        return Err(DriverError::InvalidConfiguration);
    }
    Ok(frequency as u64)
}

/// Calibrate and calculate the initial count for a periodic local-APIC timer.
pub fn calibrated_periodic_initial_count(
    apic_elapsed_ticks: u32,
    hpet_elapsed_ticks: u64,
    hpet_period_fs: u32,
    requested_hz: u32,
    divide: ApicDivide,
) -> Result<u32, DriverError> {
    let bus_hz = calibrate_bus_clock_hz(
        apic_elapsed_ticks,
        hpet_elapsed_ticks,
        hpet_period_fs,
        divide,
    )?;
    periodic_initial_count(bus_hz, requested_hz, divide)
}

/// A mapped xAPIC local-APIC register page.
///
/// # Safety
/// The base must be the correctly mapped local-APIC MMIO page for the current
/// CPU, remain valid for this object's lifetime, and not be concurrently
/// accessed without external serialization.
pub struct LocalApicTimer { base: NonNull<u8> }

impl LocalApicTimer {
    /// # Safety
    /// The caller must provide an enabled local APIC's mapped MMIO page.
    pub unsafe fn new(base: NonNull<u8>) -> Self { Self { base } }

    fn read(&self, offset: usize) -> u32 {
        // SAFETY: Constructor contract guarantees valid aligned APIC registers.
        unsafe { read_volatile(self.base.as_ptr().add(offset).cast::<u32>()) }
    }

    fn write(&mut self, offset: usize, value: u32) {
        // SAFETY: Constructor contract guarantees valid mapping; &mut self
        // serializes access through this handle.
        unsafe { write_volatile(self.base.as_ptr().add(offset).cast::<u32>(), value) }
    }

    pub fn current_count(&self) -> u32 { self.read(CURRENT_COUNT) }

    /// Configure the local APIC timer LVT and initial count.
    /// The vector must be an installed IDT interrupt gate (vectors below 32
    /// are reserved for CPU exceptions). In TSC-deadline mode, the caller must
    /// program IA32_TSC_DEADLINE separately before unmasking.
    pub fn configure(
        &mut self,
        vector: u8,
        mode: ApicTimerMode,
        divide: ApicDivide,
        initial_count: u32,
        masked: bool,
    ) -> Result<(), DriverError> {
        // Only vectors installed by the common APIC IDT/dispatcher are
        // valid here. This excludes exceptions, the reserved syscall vector,
        // and the APIC spurious vector.
        if !crate::interrupts::apic::is_apic_vector(vector) {
            return Err(DriverError::InvalidConfiguration);
        }
        if mode == ApicTimerMode::TscDeadline && initial_count != 0 {
            return Err(DriverError::InvalidConfiguration);
        }
        if mode != ApicTimerMode::TscDeadline && initial_count == 0 {
            return Err(DriverError::InvalidConfiguration);
        }

        let mode_bits = match mode {
            ApicTimerMode::OneShot => 0,
            ApicTimerMode::Periodic => LVT_MODE_PERIODIC,
            ApicTimerMode::TscDeadline => LVT_MODE_TSC_DEADLINE,
        };
        let mut lvt = vector as u32 | mode_bits;
        // Deadline mode must remain masked until IA32_TSC_DEADLINE is
        // programmed by the architecture/MSR layer.
        if masked || mode == ApicTimerMode::TscDeadline { lvt |= LVT_MASKED; }

        // Quiesce the previous configuration before changing divider/count.
        // Keep the new mode masked until every register is programmed.
        self.write(LVT_TIMER, lvt | LVT_MASKED);
        if mode != ApicTimerMode::TscDeadline {
            self.write(DIVIDE_CONFIGURATION, divide.encoding() as u32);
            self.write(INITIAL_COUNT, initial_count);
        } else {
            // TSC deadline is driven by IA32_TSC_DEADLINE, not INITIAL_COUNT.
            self.write(INITIAL_COUNT, 0);
        }
        self.write(LVT_TIMER, lvt);
        Ok(())
    }

    /// Stop countdown/periodic delivery by masking the LVT and clearing its
    /// initial count. In TSC-deadline mode the architecture layer must also
    /// write zero to IA32_TSC_DEADLINE; this MMIO driver cannot do that.
    pub fn stop(&mut self) {
        let lvt = self.read(LVT_TIMER);
        self.write(LVT_TIMER, lvt | LVT_MASKED);
        self.write(INITIAL_COUNT, 0);
    }

    /// Load a new countdown. The caller must ensure the LVT is configured and
    /// unmasked only after the count and interrupt path are ready.
    pub fn set_initial_count(&mut self, count: u32) { self.write(INITIAL_COUNT, count); }

    pub fn mask(&mut self) {
        let lvt = self.read(LVT_TIMER);
        self.write(LVT_TIMER, lvt | LVT_MASKED);
    }

    pub fn unmask(&mut self) {
        let lvt = self.read(LVT_TIMER);
        self.write(LVT_TIMER, lvt & !LVT_MASKED);
    }
}


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
        let hpet_start = hpet.read_counter() & mask;
        if apic.load_masked_one_shot_count(u32::MAX).is_err() {
            apic.stop_one_shot_timer();
            return Err(DriverError::InvalidConfiguration);
        }

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

        let current_count = apic.timer_current_count();
        let hpet_end = hpet.read_counter() & mask;
        let elapsed_hpet = hpet_end.wrapping_sub(hpet_start) & mask;
        apic.stop_one_shot_timer();

        let elapsed_apic = u32::MAX.saturating_sub(current_count);
        if elapsed_apic == 0 || (elapsed_hpet as u128) < target_ticks {
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
        let numerator = (delay_ns as u128)
            .checked_mul(self.frequency_hz as u128)
            .ok_or(DriverError::InvalidConfiguration)?;
        let ticks = if delay_ns == 0 {
            1
        } else {
            numerator
                .checked_add(999_999_999)
                .ok_or(DriverError::InvalidConfiguration)?
                / 1_000_000_000
        };
        let count = core::cmp::min(core::cmp::max(1, ticks), u32::MAX as u128) as u32;
        // SAFETY: Caller guarantees this CPU's x2APIC timer is configured and unmasked.
        let mut apic = unsafe { X2Apic::new() };
        apic.arm_one_shot_timer(count).map_err(map_x2apic_error)?;
        Ok(count)
    }

    /// Arm an explicit non-zero APIC countdown value.
    ///
    /// # Safety
    /// Must execute on the owning CPU at CPL 0 after unmasking the timer.
    pub unsafe fn arm_count(&mut self, count: u32) -> Result<(), DriverError> {
        // SAFETY: Caller guarantees this CPU's x2APIC timer is configured and unmasked.
        let mut apic = unsafe { X2Apic::new() };
        apic.arm_one_shot_timer(count).map_err(map_x2apic_error)
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

fn map_x2apic_error(_error: X2ApicError) -> DriverError {
    DriverError::InvalidConfiguration
}

/// IA32_TSC_DEADLINE model-specific register.
pub const IA32_TSC_DEADLINE: u32 = 0x6E0;

/// x86 TSC-deadline MSR control.
///
/// The local APIC LVT vector/mode is configured separately through
/// `LocalApicTimer::configure(..., ApicTimerMode::TscDeadline, ...)`.
/// This object only arms/disarms the per-CPU absolute TSC deadline, and works
/// with either xAPIC MMIO or x2APIC mode.
pub struct TscDeadline {
    configured: bool,
}

impl TscDeadline {
    /// Create a TSC-deadline controller after verifying CPU support.
    ///
    /// # Safety
    /// Requires CPL 0, a CPU advertising TSC-deadline support, and an already
    /// configured local APIC LVT in TSC-deadline mode on this CPU.
    pub unsafe fn new() -> Result<Self, DriverError> {
        let features = crate::arch::x86_64::cpuid::cpuid(1, 0).ecx;
        if features & (1 << 24) == 0 {
            return Err(DriverError::UnsupportedDevice);
        }
        Ok(Self { configured: true })
    }

    /// Arm an absolute TSC deadline. A value of zero disarms the timer.
    ///
    /// # Safety
    /// Must execute at CPL 0 on the target CPU. The deadline must be expressed
    /// in that CPU's TSC domain and synchronized with the selected clocksource.
    pub unsafe fn arm(&self, deadline_tsc: u64) -> Result<(), DriverError> {
        if !self.configured {
            return Err(DriverError::NotReady);
        }
        // SAFETY: Constructor checked architectural support; caller guarantees
        // the local APIC LVT has already been configured for deadline mode.
        unsafe { crate::arch::x86_64::msr::write(IA32_TSC_DEADLINE, deadline_tsc) };
        Ok(())
    }

    /// Clear the currently armed deadline.
    ///
    /// # Safety
    /// Must execute at CPL 0 on the same CPU used to construct this handle.
    pub unsafe fn stop(&self) {
        if self.configured {
            // SAFETY: IA32_TSC_DEADLINE support was checked in new().
            unsafe { crate::arch::x86_64::msr::write(IA32_TSC_DEADLINE, 0) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn divide_encodings_match_architecture() {
        assert_eq!(ApicDivide::By1.encoding(), 0b1011);
        assert_eq!(ApicDivide::By128.encoding(), 0b1010);
    }
}
