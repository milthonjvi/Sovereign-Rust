//! Legacy i8042 PS/2 controller support.
pub mod controller;
pub use controller::{Ps2Controller, Ps2DataByte, Ps2Diagnostics, Ps2Error, Ps2MouseProtocol};
