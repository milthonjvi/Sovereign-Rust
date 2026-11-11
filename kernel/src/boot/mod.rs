//! Kernel boot handoff.
//!
//! This layer will validate and normalize bootloader-provided memory maps,
//! framebuffer metadata, firmware pointers, and command-line options before
//! exposing them to memory management and device discovery.
//!
//! Keep boot protocol parsing in the top-level boot integration; this module
//! owns the kernel-side representation and validation boundary.

pub mod limine;
pub mod sovereign;
