#![no_std]

//! Sovereign Core kernel.
//!
//! Keep this crate independent from host operating-system services. Architecture-
//! specific instructions and CPU state belong under `arch/`; portable kernel
//! policy belongs in separate subsystem modules.

pub mod arch;
