//! HPET register layout, capability decoding, and mapped-device operations.
//!
//! ACPI discovery and MMIO mapping are external prerequisites. This driver
//! never discovers or maps hardware by itself.

use core::ptr::{read_volatile, write_volatile, NonNull};
use crate::drivers::api::DriverError;

pub const GENERAL_CAPABILITIES_ID: usize = 0x000;
pub const GENERAL_CONFIGURATION: usize = 0x010;
pub const GENERAL_INTERRUPT_STATUS: usize = 0x020;
pub const MAIN_COUNTER: usize = 0x0f0;
pub const TIMER_CONFIGURATION_BASE: usize = 0x100;
pub const TIMER_COMPARATOR_BASE: usize = 0x108;
pub const TIMER_STRIDE: usize = 0x020;

const GENERAL_ENABLE: u64 = 1;
const LEGACY_REPLACEMENT_ENABLE: u64 = 1 << 1;
const TIMER_INTERRUPT_ENABLE: u64 = 1 << 2;
const TIMER_PERIODIC_MODE: u64 = 1 << 3;
const TIMER_ROUTE_MASK: u64 = 0x1f << 9;
const TIMER_FSB_ENABLE: u64 = 1 << 14;
const TIMER_WRITABLE_CONTROL: u64 =
    TIMER_INTERRUPT_ENABLE | TIMER_PERIODIC_MODE | TIMER_ROUTE_MASK | TIMER_FSB_ENABLE;

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

/// A mapped HPET register block.
///
/// # Safety
/// The base must point to a live, correctly mapped HPET MMIO region spanning
/// every register accessed by this type. The mapping must remain valid for the
/// lifetime of this object, and accesses must be serialized against other
/// agents touching the same HPET.
pub struct Hpet {
    base: NonNull<u8>,
    capabilities: HpetCapabilities,
}

impl Hpet {
    /// Construct from an ACPI-validated and already-mapped HPET base address.
    ///
    /// # Safety
    /// The caller must uphold the mapping and exclusive-access requirements
    /// documented on Hpet.
    pub unsafe fn new(base: NonNull<u8>) -> Result<Self, DriverError> {
        // SAFETY: The caller guarantees a valid mapped HPET register block.
        let raw = unsafe { read_volatile(base.as_ptr().add(GENERAL_CAPABILITIES_ID).cast::<u64>()) };
        let capabilities = HpetCapabilities::decode(raw)?;
        Ok(Self { base, capabilities })
    }

    pub const fn capabilities(&self) -> HpetCapabilities { self.capabilities }

    fn read_register(&self, offset: usize) -> u64 {
        // SAFETY: Hpet::new guarantees the mapped register block remains valid.
        unsafe { read_volatile(self.base.as_ptr().add(offset).cast::<u64>()) }
    }

    fn write_register(&mut self, offset: usize, value: u64) {
        // SAFETY: Hpet::new guarantees a valid mapping; &mut self provides
        // exclusive access through this handle.
        unsafe { write_volatile(self.base.as_ptr().add(offset).cast::<u64>(), value) }
    }

    pub fn is_enabled(&self) -> bool {
        self.read_register(GENERAL_CONFIGURATION) & GENERAL_ENABLE != 0
    }

    /// Enable the main counter, leaving legacy replacement routing disabled.
    pub fn enable(&mut self) {
        let config = self.read_register(GENERAL_CONFIGURATION);
        self.write_register(GENERAL_CONFIGURATION,
            (config & !LEGACY_REPLACEMENT_ENABLE) | GENERAL_ENABLE);
    }

    /// Disable the main counter without changing timer interrupt configuration.
    pub fn disable(&mut self) {
        let config = self.read_register(GENERAL_CONFIGURATION);
        self.write_register(GENERAL_CONFIGURATION, config & !GENERAL_ENABLE);
    }

    pub fn read_counter(&self) -> u64 {
        let value = self.read_register(MAIN_COUNTER);
        if self.capabilities.counter_64_bit { value } else { value & 0xffff_ffff }
    }

    /// Reset the main counter. The HPET must be disabled first.
    pub fn reset_counter(&mut self) -> Result<(), DriverError> {
        if self.is_enabled() { return Err(DriverError::NotReady); }
        self.write_register(MAIN_COUNTER, 0);
        Ok(())
    }

    /// Configure a timer for one-shot interrupt delivery on an I/O APIC route.
    /// This does not configure the I/O APIC or install an interrupt handler.
    pub fn configure_oneshot(&mut self, timer: u8, route: u8) -> Result<(), DriverError> {
        if timer >= self.capabilities.timer_count || route > 31 {
            return Err(DriverError::InvalidConfiguration);
        }
        let offset = TIMER_CONFIGURATION_BASE + timer as usize * TIMER_STRIDE;
        let current = self.read_register(offset);
        let config = (current & !TIMER_WRITABLE_CONTROL)
            | TIMER_INTERRUPT_ENABLE | ((route as u64) << 9);
        self.write_register(offset, config);
        Ok(())
    }

    /// Set an absolute main-counter comparator value for a configured timer.
    pub fn set_comparator(&mut self, timer: u8, value: u64) -> Result<(), DriverError> {
        if timer >= self.capabilities.timer_count {
            return Err(DriverError::InvalidConfiguration);
        }
        let config_offset = TIMER_CONFIGURATION_BASE + timer as usize * TIMER_STRIDE;
        let timer_config = self.read_register(config_offset);
        let timer_supports_64_bit = timer_config & (1 << 5) != 0;
        if !timer_supports_64_bit && value > u32::MAX as u64 {
            return Err(DriverError::InvalidConfiguration);
        }
        let offset = TIMER_COMPARATOR_BASE + timer as usize * TIMER_STRIDE;
        self.write_register(offset, value);
        Ok(())
    }

    /// Disable interrupt delivery for one timer while preserving other controls.
    pub fn mask_timer(&mut self, timer: u8) -> Result<(), DriverError> {
        if timer >= self.capabilities.timer_count {
            return Err(DriverError::InvalidConfiguration);
        }
        let offset = TIMER_CONFIGURATION_BASE + timer as usize * TIMER_STRIDE;
        let config = self.read_register(offset);
        self.write_register(offset, config & !TIMER_INTERRUPT_ENABLE);
        Ok(())
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

    #[test]
    fn rejects_unreasonable_period() {
        assert_eq!(HpetCapabilities::decode((100_000_001u64) << 32),
            Err(DriverError::InvalidConfiguration));
    }
}
