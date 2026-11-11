//! PCI and PCI Express bus support.
//! Discovery is read-only; BAR sizing, bus mastering and interrupt routing are
//! separate privileged operations coordinated with kernel services.
pub mod config;
pub use config::{scan_bus, Bar, BarKind, ConfigAccess, ConfigError, EcamWindow, PciAddress, PciFunction, PciIdentity};
