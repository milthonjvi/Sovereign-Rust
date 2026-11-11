//! PCI and PCI Express bus support.
//! Discovery is read-only; BAR sizing, bus mastering and interrupt routing are
//! separate privileged operations coordinated with kernel services.
pub mod capabilities;
pub mod config;
pub mod ecam;
pub mod mechanism;
pub mod msi;
pub use capabilities::{Capability, ExtendedCapabilities, ExtendedCapability, StandardCapabilities};
pub use config::{read_bars, scan_bus, Bar, BarKind, ConfigAccess, ConfigError, EcamWindow, PciAddress, PciFunction, PciIdentity};
pub use ecam::{parse_mcfg, physical_address, EcamAccess, McfgError};
pub use mechanism::Mechanism1;
pub use msi::{MsiCapability, MsixCapability, MSI_CAPABILITY_ID, MSIX_CAPABILITY_ID};
