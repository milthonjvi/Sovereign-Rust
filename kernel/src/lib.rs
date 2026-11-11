#![no_std]

#[cfg(test)]
extern crate std;

//! Sovereign Core kernel.
//!
//! The kernel is split into portable policy and architecture-specific mechanisms.
//! Hardware access is isolated behind architecture and driver interfaces.

pub mod arch;
pub mod boot;
pub mod drivers;
pub mod firmware;
pub mod fs;
pub mod interrupts;
pub mod ipc;
pub mod logging;
pub mod memory;
pub mod power;
pub mod security;
pub mod sync;
pub mod syscall;
pub mod task;
pub mod time;
