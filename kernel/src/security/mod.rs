//! Privilege separation, capability checks, and security policy.
//!
//! This subsystem becomes operational with user mode, process isolation, and the
//! syscall ABI. Do not claim isolation before page-table enforcement exists.
