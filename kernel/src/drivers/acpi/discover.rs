//! ACPI root-table discovery.
//!
//! The memory manager owns physical-to-virtual mapping. This module receives a
//! short-lived mapping callback and never constructs pointers from firmware
//! addresses itself.

use super::{
    AcpiError,
    rsdp::{RootTableAddress, Rsdp},
    sdt::{RootTable, Sdt},
};

/// Physical addresses of the principal tables found in RSDT/XSDT.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AcpiTables {
    pub root_address: u64,
    pub used_xsdt: bool,
    pub madt: Option<u64>,
    pub fadt: Option<u64>,
    pub hpet: Option<u64>,
    pub dsdt: Option<u64>,
}

impl AcpiTables {
    /// Discover known tables using a caller-provided physical mapping function.
    ///
    /// The callback must return a slice covering the complete table at the
    /// requested physical address. The slice must remain valid for this call.
    /// Every returned SDT is checked for length and checksum before its
    /// signature is inspected.
    pub fn discover<F>(rsdp: &Rsdp, mut map_table: F) -> Result<Self, AcpiError>
    where
        F: FnMut(u64) -> Option<&'static [u8]>,
    {
        let root = rsdp.root_table_address()?;
        let (root_address, root_table) = match root {
            RootTableAddress::Xsdt(address) => {
                let bytes = map_table(address).ok_or(AcpiError::UnsupportedAddress)?;
                (address, RootTable::parse_xsdt(bytes)?)
            }
            RootTableAddress::Rsdt(address) => {
                let bytes = map_table(address).ok_or(AcpiError::UnsupportedAddress)?;
                (address, RootTable::parse_rsdt(bytes)?)
            }
        };

        let mut found = Self {
            root_address,
            used_xsdt: matches!(root, RootTableAddress::Xsdt(_)),
            ..Self::default()
        };

        for address in root_table.entries() {
            if address == 0 {
                continue;
            }
            let bytes = map_table(address).ok_or(AcpiError::UnsupportedAddress)?;
            let table = Sdt::parse(bytes)?;
            match table.signature() {
                *b"APIC" if found.madt.is_none() => found.madt = Some(address),
                *b"FACP" if found.fadt.is_none() => found.fadt = Some(address),
                *b"HPET" if found.hpet.is_none() => found.hpet = Some(address),
                *b"DSDT" if found.dsdt.is_none() => found.dsdt = Some(address),
                _ => {}
            }
        }
        Ok(found)
    }
}
