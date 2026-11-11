//! Device-driver framework and hardware-family boundaries.
//!
//! These modules define contracts and ownership only. A module declaration is
//! not a claim that a hardware driver is implemented or tested.

pub mod api;
pub mod acpi;
pub mod framebuffer;
pub mod input;
pub mod ioapic;
pub mod network;
pub mod pci;
pub mod serial;
pub mod storage;
pub mod timer;
pub mod usb;
