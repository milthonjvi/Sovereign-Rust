//! ACPI table discovery and platform description.
//!
//! Planned tables include RSDP, XSDT/RSDT, MADT, FADT, and HPET where present.
//! Parsing must validate lengths and checksums before exposing table data.
