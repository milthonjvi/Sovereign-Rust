//! Intel 8253/8254-compatible PIT channel 0 driver.
//!
//! The PIT is programmed through legacy port I/O. The caller must first reserve
//! the ports and ensure the interrupt path is configured before enabling IRQ0.

use crate::arch::x86_64::io;
use crate::drivers::api::{DeviceInfo, Driver, DriverError};

pub const INPUT_HZ: u32 = 1_193_182;
pub const CHANNEL0_DATA: u16 = 0x40;
pub const COMMAND: u16 = 0x43;
const CHANNEL0_RATE_GENERATOR: u8 = 0x34;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PitConfig {
    /// Hardware divisor. A value of zero encodes 65,536.
    pub divisor: u16,
    /// Effective frequency after integer divisor rounding.
    pub actual_hz: u32,
}

impl PitConfig {
    /// Convert a requested periodic rate into the PIT's representable divisor.
    pub fn from_frequency(requested_hz: u32) -> Result<Self, DriverError> {
        if requested_hz == 0 || requested_hz > INPUT_HZ {
            return Err(DriverError::InvalidConfiguration);
        }

        let rounded = (INPUT_HZ + requested_hz / 2) / requested_hz;
        if rounded == 0 || rounded > 65_536 {
            return Err(DriverError::InvalidConfiguration);
        }
        let divisor = if rounded == 65_536 { 0 } else { rounded as u16 };
        Ok(Self {
            divisor,
            actual_hz: INPUT_HZ / rounded,
        })
    }

    /// Return the effective divisor as a non-encoded integer.
    pub const fn divisor_value(self) -> u32 {
        if self.divisor == 0 { 65_536 } else { self.divisor as u32 }
    }
}

/// PIT channel 0, mode 2/3-compatible periodic rate generator.
pub struct Pit {
    config: Option<PitConfig>,
}

impl Pit {
    pub const fn new() -> Self {
        Self { config: None }
    }

    pub const fn config(&self) -> Option<PitConfig> {
        self.config
    }

    /// Program channel 0. This does not unmask PIC IRQ0 or enable interrupts.
    ///
    /// # Safety
    /// The caller must own ports 0x40/0x43, run at CPL 0, and arrange a valid
    /// IRQ0 handler before relying on periodic interrupts.
    pub unsafe fn configure(&mut self, requested_hz: u32) -> Result<PitConfig, DriverError> {
        let config = PitConfig::from_frequency(requested_hz)?;
        // SAFETY: Port ownership and privilege are guaranteed by this function's caller.
        unsafe {
            io::out8(COMMAND, CHANNEL0_RATE_GENERATOR);
            io::out8(CHANNEL0_DATA, config.divisor as u8);
            io::out8(CHANNEL0_DATA, (config.divisor >> 8) as u8);
        }
        self.config = Some(config);
        Ok(config)
    }
}

impl Default for Pit {
    fn default() -> Self { Self::new() }
}

impl Driver for Pit {
    fn name(&self) -> &'static str { "i8254-pit" }

    fn supports(&self, device: &DeviceInfo) -> bool {
        device.class == crate::drivers::api::DeviceClass::Timer
            && device.vendor_id.is_none()
            && device.device_id.is_none()
    }

    fn initialize(&mut self, _device: &DeviceInfo) -> Result<(), DriverError> {
        // Configuration requires an explicit frequency and unsafe port ownership.
        Err(DriverError::InvalidConfiguration)
    }

    fn shutdown(&mut self) -> Result<(), DriverError> {
        // The PIT has no universally safe "stop" operation; leave its current mode.
        self.config = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_common_tick_rates() {
        let config = PitConfig::from_frequency(1000).unwrap();
        assert_eq!(config.divisor_value(), 1193);
        assert_eq!(config.actual_hz, 1000);
    }

    #[test]
    fn encodes_maximum_divisor_as_zero() {
        let config = PitConfig { divisor: 0, actual_hz: 18 };
        assert_eq!(config.divisor_value(), 65_536);
    }

    #[test]
    fn rejects_unrepresentable_rates() {
        assert_eq!(PitConfig::from_frequency(0), Err(DriverError::InvalidConfiguration));
        assert_eq!(PitConfig::from_frequency(INPUT_HZ + 1), Err(DriverError::InvalidConfiguration));
    }
}
