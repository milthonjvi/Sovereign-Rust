//! HPET register layout and capability decoding.
//!
//! Mapping the HPET MMIO region and routing its interrupts require the memory,
//! ACPI, and interrupt subsystems. This module deliberately performs no probing.

use crate::drivers::api::DriverError;

pub const GENERAL_CAPABILITIES_ID: usize = 0x000;
pub const GENERAL_CONFIGURATION: usize = 0x010;
pub const GENERAL_INTERRUPT_STATUS: usize = 0x020;
pub const MAIN_COUNTER: usize = 0x0f0;
pub const TIMER_CONFIGURATION_BASE: usize = 0x100;
pub const TIMER_COMPARATOR_BASE: usize = 0x108;
pub const TIMER_STRIDE: usize = 0x020;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HpetCapabilities {
    pub revision: u8,
    pub timer_count: u8,
    pub counter_64_bit: bool,
    pub legacy_replacement: bool,
    /// Counter period in femtoseconds.
    pub period_fs: u32,
}

impl HpetCapabilities {
    pub fn decode(capabilities_id: u64) -> Result<Self, DriverError> {
        let period_fs = (capabilities_id >> 32) as u32;
        let timer_count = (((capabilities_id >> 8) & 0x1f) + 1) as u8;
        if period_fs == 0 || period_fs > 100_000_000 {
            return Err(DriverError::InvalidConfiguration);
        }
        Ok(Self {
            revision: capabilities_id as u8,
            timer_count,
            counter_64_bit: capabilities_id & (1 << 13) != 0,
            legacy_replacement: capabilities_id & (1 << 15) != 0,
            period_fs,
        })
    }

    /// Convert a counter delta to nanoseconds without floating point.
    pub fn ticks_to_nanoseconds(self, ticks: u64) -> Result<u64, DriverError> {
        let scaled = (ticks as u128)
            .checked_mul(self.period_fs as u128)
            .ok_or(DriverError::InvalidConfiguration)?;
        let nanos = scaled / 1_000_000;
        u64::try_from(nanos).map_err(|_| DriverError::InvalidConfiguration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_capabilities_and_period() {
        let raw = (100_000_000u64 << 32) | (2 << 8) | (1 << 13) | 1;
        let caps = HpetCapabilities::decode(raw).unwrap();
        assert_eq!(caps.revision, 1);
        assert_eq!(caps.timer_count, 3);
        assert!(caps.counter_64_bit);
        assert_eq!(caps.ticks_to_nanoseconds(10).unwrap(), 1000);
    }

    #[test]
    fn rejects_zero_period() {
        assert_eq!(HpetCapabilities::decode(1), Err(DriverError::InvalidConfiguration));
    }
}
