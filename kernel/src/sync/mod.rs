//! Kernel synchronization primitives.
//!
//! Interrupt-safe locks disable local maskable interrupts before acquiring
//! the SMP lock, preventing same-CPU IRQ/task self-deadlocks.

pub mod irq_lock;
