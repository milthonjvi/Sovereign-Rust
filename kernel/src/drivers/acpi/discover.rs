//! ACPI root-table discovery.
//!
//! The memory manager owns physical-to-virtual mapping. This module receives a
//! short-lived mapping callback and never constructs pointers from firmware
//! addresses itself. The DSDT is reached through the FADT, not the RSDT/XSDT.

use super::{
    AcpiError,
    fadt::Fadt,
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
    /// signature is inspected. If a FADT is present, its legacy or extended
    /// DSDT pointer is followed and the target must validate as a DSDT.
    pub fn discover<'a, F>(rsdp: &Rsdp, mut map_table: F) -> Result<Self, AcpiError>
    where
        F: FnMut(u64) -> Option<&'a [u8]>,
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
            let signature = table.signature();
            if signature == *b"APIC" && found.madt.is_none() {
                found.madt = Some(address);
            } else if signature == *b"FACP" && found.fadt.is_none() {
                found.fadt = Some(address);
            } else if signature == *b"HPET" && found.hpet.is_none() {
                found.hpet = Some(address);
            }
        }

        // ACPI locates the DSDT through FADT.X_DSDT (preferred when nonzero),
        // falling back to FADT.DSDT for older firmware.
        if let Some(fadt_address) = found.fadt {
            let fadt_bytes = map_table(fadt_address).ok_or(AcpiError::UnsupportedAddress)?;
            let fadt = Fadt::parse(fadt_bytes)?;
            let dsdt_address = fadt.dsdt_address();
            if dsdt_address != 0 {
                let dsdt_bytes = map_table(dsdt_address)
                    .ok_or(AcpiError::UnsupportedAddress)?;
                let dsdt = Sdt::parse(dsdt_bytes)?;
                if dsdt.signature() != *b"DSDT" {
                    return Err(AcpiError::InvalidSignature);
                }
                found.dsdt = Some(dsdt_address);
            }
        }

        Ok(found)
    }
}
