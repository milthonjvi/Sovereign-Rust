//! x86_64 CPU support.
//!
//! Privileged operations are isolated in small modules. Callers must only use
//! privileged functions from ring 0 and must uphold each function's safety
//! contract.

pub mod control;
pub mod cpuid;
pub mod io;
pub mod msr;
