//! PCI and PCI Express bus support.
//! Discovery and capability decoding are passive; BAR sizing/programming,
//! Command-register changes, and interrupt activation are explicit operations.
pub mod bar_probe;
pub mod bar_program;
pub mod capabilities;
pub mod config;
pub mod command;
pub mod ecam;
pub mod enumeration;
pub mod mechanism;
pub mod pcie;
pub mod msi;
pub mod msix_table;
pub mod resources;
pub mod resource_allocator;
pub use bar_probe::{probe_bar_sizes, BarProbeError, BarSize};
pub use bar_program::{program_bar, BarProgramError};
pub use capabilities::{Capability, ExtendedCapabilities, ExtendedCapability, StandardCapabilities};
pub use config::{read_bars, scan_bus, Bar, BarKind, ConfigAccess, ConfigError, EcamWindow, PciAddress, PciFunction, PciIdentity};
pub use command::{read_command, set_bus_master, set_intx_disabled, set_io_space, set_memory_space, update_command, COMMAND_BUS_MASTER, COMMAND_INTERRUPT_DISABLE, COMMAND_IO_SPACE, COMMAND_MEMORY_SPACE};
pub use ecam::{parse_mcfg, physical_address, EcamAccess, McfgError};
pub use enumeration::{scan_hierarchy, EnumerationReport};
pub use mechanism::Mechanism1;
pub use pcie::{PcieCapability, PcieDeviceCapabilities, PcieLinkCapabilities, PcieLinkSpeed, PcieLinkStatus, PciePortType, PCIE_CAPABILITY_ID};
pub use msi::{configure_msi, disable_msi, disable_msix, enable_msix_masked, set_msix_function_mask, MsiCapability, MsiConfigError, MsixCapability, MSI_CAPABILITY_ID, MSIX_CAPABILITY_ID};
pub use msix_table::{MsixEntry, MsixPendingBitArray, MsixTable, MsixTableError};
pub use resources::{BarResource, ResourceError};
pub use resource_allocator::{AllocationError, ResourceAllocation, ResourcePool, ResourceRequest, ResourceSpace, ResourceWindow};
