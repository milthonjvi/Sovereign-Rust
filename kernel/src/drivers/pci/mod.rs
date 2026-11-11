//! PCI/PCIe enumeration and configuration-space access.
//!
//! Planned: enumerate functions, identify BARs and capabilities, and publish
//! discovered devices through the common driver API. Probing waits for safe
//! architecture port/MMIO access and firmware routing.
