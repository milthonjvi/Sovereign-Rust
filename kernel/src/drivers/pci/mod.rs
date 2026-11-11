//! PCI and PCI Express bus support.
//! Discovery is read-only; BAR sizing, bus mastering and interrupt routing are
//! separate privileged operations coordinated with kernel services.
pub mod bar_probe;
pub mod capabilities;
pub mod config;
pub mod ecam;
pub mod enumeration;
pub mod mechanism;
pub mod msi;
pub mod resources;
pub use bar_probe::{probe_bar_sizes, BarProbeError, BarSize};
pub use capabilities::{Capability, ExtendedCapabilities, ExtendedCapability, StandardCapabilities};
pub use config::{read_bars, scan_bus, Bar, BarKind, ConfigAccess, ConfigError, EcamWindow, PciAddress, PciFunction, PciIdentity};
pub use ecam::{parse_mcfg, physical_address, EcamAccess, McfgError};
pub use enumeration::{scan_hierarchy, EnumerationReport};
pub use mechanism::Mechanism1;
pub use msi::{MsiCapability, MsixCapability, MSI_CAPABILITY_ID, MSIX_CAPABILITY_ID};
pub use resources::{BarResource, ResourceError};
