//! Hardware clock and timer device backends.
//!
//! Device programming is kept separate from timekeeping policy in `time/`.
//! HPET and APIC helpers currently decode/calibrate only; they do not touch
//! hardware until ACPI, MMIO mapping, and interrupt routing are integrated.

pub mod apic;
pub mod hpet;
pub mod pit;

pub use apic::{ApicDivide, ApicTimerMode, periodic_initial_count};
pub use hpet::HpetCapabilities;
pub use pit::{Pit, PitConfig};
