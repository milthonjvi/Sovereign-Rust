//! Hardware clock and timer device backends.
//!
//! Device programming is kept separate from timekeeping policy in time/.
//! ACPI discovery, MMIO mapping, and interrupt routing remain explicit
//! prerequisites for HPET and local-APIC operation.

pub mod apic;
pub mod hpet;
pub mod pit;

pub use apic::{
    ApicDivide, ApicTimerMode, LocalApicTimer, TscDeadline, X2ApicTimer, IA32_TSC_DEADLINE, calibrated_periodic_initial_count,
    calibrate_bus_clock_hz, periodic_initial_count,
};
pub use hpet::{counter_delta, route_is_supported, Hpet, HpetCapabilities};
pub use pit::{Pit, PitConfig, PitIrqError};
