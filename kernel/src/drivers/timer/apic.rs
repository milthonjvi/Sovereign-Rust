//! Local APIC timer capability and calibration helpers.
//!
//! Programming the local APIC timer is intentionally deferred until Sovereign
//! has a validated MADT topology, local-APIC MMIO mapping, and interrupt routing.

use crate::drivers::api::DriverError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApicTimerMode {
    OneShot,
    Periodic,
    TscDeadline,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApicDivide {
    By1,
    By2,
    By4,
    By8,
    By16,
    By32,
    By64,
    By128,
}

impl ApicDivide {
    pub const fn encoding(self) -> u8 {
        match self {
            Self::By2 => 0b0000,
            Self::By4 => 0b0001,
            Self::By8 => 0b0010,
            Self::By16 => 0b0011,
            Self::By32 => 0b1000,
            Self::By64 => 0b1001,
            Self::By128 => 0b1010,
            Self::By1 => 0b1011,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculates_periodic_count() {
        assert_eq!(periodic_initial_count(100_000_000, 1000, ApicDivide::By16), Ok(6250));
    }

    #[test]
    fn rejects_zero_frequency() {
        assert_eq!(periodic_initial_count(100_000_000, 0, ApicDivide::By1), Err(DriverError::InvalidConfiguration));
    }

    #[test]
    fn divide_encodings_match_architecture() {
        assert_eq!(ApicDivide::By1.encoding(), 0b1011);
        assert_eq!(ApicDivide::By128.encoding(), 0b1010);
    }
}
