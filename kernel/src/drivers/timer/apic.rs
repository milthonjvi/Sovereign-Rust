//! Local APIC timer register operations and calibration helpers.
//!
//! The local APIC must already be enabled, its MMIO page mapped, and its
//! interrupt vector installed before the timer is unmasked. TSC-deadline MSR
//! programming remains the responsibility of the architecture/MSR layer.

use core::ptr::{read_volatile, write_volatile, NonNull};
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
        if vector < 32 { return Err(DriverError::InvalidConfiguration); }
        if mode == ApicTimerMode::TscDeadline && initial_count != 0 {
            return Err(DriverError::InvalidConfiguration);
        }

        let mode_bits = match mode {
            ApicTimerMode::OneShot => 0,
            ApicTimerMode::Periodic => LVT_MODE_PERIODIC,
            ApicTimerMode::TscDeadline => LVT_MODE_TSC_DEADLINE,
        };
        let mut lvt = vector as u32 | mode_bits;
        if masked { lvt |= LVT_MASKED; }

        self.write(LVT_TIMER, lvt);
        if mode != ApicTimerMode::TscDeadline {
            self.write(DIVIDE_CONFIGURATION, divide.encoding() as u32);
            self.write(INITIAL_COUNT, initial_count);
        } else {
            self.write(INITIAL_COUNT, 0);
        }
        Ok(())
    }

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
    fn divide_encodings_match_architecture() {
        assert_eq!(ApicDivide::By1.encoding(), 0b1011);
        assert_eq!(ApicDivide::By128.encoding(), 0b1010);
    }
}
