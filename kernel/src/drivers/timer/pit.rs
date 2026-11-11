//! Intel 8253/8254-compatible PIT channel 0 driver.
//!
//! The PIT is programmed through legacy port I/O. The caller must first reserve
//! the ports and ensure the interrupt path is configured before enabling IRQ0.

use crate::arch::x86_64::io;
use crate::drivers::api::{DeviceInfo, Driver, DriverError};
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

static PIT_IRQ_ACTIVE: AtomicBool = AtomicBool::new(false);
static PIT_TICKS: AtomicU64 = AtomicU64::new(0);
static PIT_FREQUENCY_HZ: AtomicU32 = AtomicU32::new(0);
static PIT_WORK_PENDING: AtomicBool = AtomicBool::new(false);

extern "C" fn pit_irq_handler(_vector: u8) {
    PIT_TICKS.fetch_add(1, Ordering::Relaxed);
    PIT_WORK_PENDING.store(true, Ordering::Release);
}

/// Errors while connecting PIT channel 0 to the legacy PIC IRQ0 path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PitIrqError {
    AlreadyActive,
    NotActive,
    Interrupt(crate::interrupts::init::LegacyIrqError),
    Driver(DriverError),
}

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
        // The PIT's maximum divisor produces approximately 18.2 Hz. Accept
        // an 18 Hz request using that closest representable hardware rate.
        let rounded = if requested_hz == 18 && rounded > 65_536 {
            65_536
        } else {
            rounded
        };
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

    /// Latch and read the current channel-0 down-counter value.
    ///
    /// A returned value of 65,536 represents the hardware's zero encoding.
    ///
    /// # Safety
    /// The caller must own PIT channel 0 ports, run at CPL 0, and serialize
    /// access with any other code programming or reading the PIT.
    pub unsafe fn read_current_count(&mut self) -> u32 {
        // SAFETY: Port ownership and privilege are caller obligations.
        unsafe {
            io::out8(COMMAND, 0x00);
            let low = io::in8(CHANNEL0_DATA) as u16;
            let high = io::in8(CHANNEL0_DATA) as u16;
            let raw = low | (high << 8);
            if raw == 0 { 65_536 } else { raw as u32 }
        }
    }

    /// Configure a periodic PIT tick and connect it to legacy PIC IRQ0.
    ///
    /// The IRQ handler only increments an atomic tick counter and requests
    /// deferred timer work. PIC EOI remains owned by the common IRQ dispatcher.
    ///
    /// # Safety
    /// Must run on the BSP at CPL 0 with IF clear, after interrupt bootstrap
    /// has initialized the legacy PIC. The caller must own PIT ports 0x40/0x43
    /// and ensure IRQ0 is not being configured by another subsystem.
    pub unsafe fn initialize_periodic_irq(
        &mut self,
        requested_hz: u32,
    ) -> Result<PitConfig, PitIrqError> {
        let config = PitConfig::from_frequency(requested_hz)
            .map_err(PitIrqError::Driver)?;
        PIT_IRQ_ACTIVE.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| PitIrqError::AlreadyActive)?;

        // Register while IRQ0 is masked; the interrupt layer owns PIC state
        // and will acknowledge every delivered IRQ after this callback.
        if let Err(error) = unsafe {
            crate::interrupts::init::register_legacy_irq(0, pit_irq_handler)
        } {
            PIT_IRQ_ACTIVE.store(false, Ordering::Release);
            return Err(PitIrqError::Interrupt(error));
        }

        PIT_TICKS.store(0, Ordering::Relaxed);
        PIT_WORK_PENDING.store(false, Ordering::Release);
        PIT_FREQUENCY_HZ.store(config.actual_hz, Ordering::Release);
        if let Err(error) = unsafe { self.configure(requested_hz) } {
            let _ = unsafe {
                crate::interrupts::init::unregister_legacy_irq(0, pit_irq_handler)
            };
            PIT_FREQUENCY_HZ.store(0, Ordering::Release);
            PIT_IRQ_ACTIVE.store(false, Ordering::Release);
            return Err(PitIrqError::Driver(error));
        }

        if let Err(error) = unsafe { crate::interrupts::init::unmask_legacy_irq(0) } {
            let _ = unsafe {
                crate::interrupts::init::unregister_legacy_irq(0, pit_irq_handler)
            };
            PIT_FREQUENCY_HZ.store(0, Ordering::Release);
            PIT_IRQ_ACTIVE.store(false, Ordering::Release);
            return Err(PitIrqError::Interrupt(error));
        }
        Ok(config)
    }

    /// Mask IRQ0 and unregister the PIT tick callback.
    ///
    /// The PIT hardware continues counting; only its interrupt delivery is
    /// disabled. This method does not change the PIT channel programming.
    ///
    /// # Safety
    /// Must run on the BSP at CPL 0 with IF clear and no PIT handler in flight.
    pub unsafe fn shutdown_periodic_irq(&mut self) -> Result<(), PitIrqError> {
        if !PIT_IRQ_ACTIVE.load(Ordering::Acquire) {
            return Err(PitIrqError::NotActive);
        }
        unsafe {
            crate::interrupts::init::unregister_legacy_irq(0, pit_irq_handler)
        }.map_err(PitIrqError::Interrupt)?;
        PIT_FREQUENCY_HZ.store(0, Ordering::Release);
        PIT_IRQ_ACTIVE.store(false, Ordering::Release);
        self.config = None;
        Ok(())
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


/// Whether PIT periodic IRQ0 is currently registered and unmasked.
#[inline]
pub fn periodic_irq_active() -> bool {
    PIT_IRQ_ACTIVE.load(Ordering::Acquire)
}

/// Current PIT tick count since periodic IRQ initialization.
#[inline]
pub fn periodic_ticks() -> u64 {
    PIT_TICKS.load(Ordering::Acquire)
}

/// Effective integer tick frequency configured for the PIT.
#[inline]
pub fn periodic_frequency_hz() -> u32 {
    PIT_FREQUENCY_HZ.load(Ordering::Acquire)
}

/// Atomically consume the PIT-specific deferred-work signal.
#[inline]
pub fn take_periodic_work_pending() -> bool {
    PIT_WORK_PENDING.swap(false, Ordering::AcqRel)
}

/// Reassert PIT deferred work when a consumer cannot drain its event buffer.
#[inline]
pub(crate) fn signal_periodic_work_pending() {
    PIT_WORK_PENDING.store(true, Ordering::Release);
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
        let config = PitConfig::from_frequency(18).unwrap();
        assert_eq!(config.divisor, 0);
        assert_eq!(config.divisor_value(), 65_536);
    }

    #[test]
    fn rejects_unrepresentable_rates() {
        assert_eq!(PitConfig::from_frequency(0), Err(DriverError::InvalidConfiguration));
        assert_eq!(PitConfig::from_frequency(INPUT_HZ + 1), Err(DriverError::InvalidConfiguration));
    }
}
