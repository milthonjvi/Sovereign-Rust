//! Hardware clock and timer sources.
//!
//! Candidate backends: PIT, HPET, and APIC timer. Clocksource measurement and
//! scheduler tick policy belong in the time subsystem, not individual drivers.
