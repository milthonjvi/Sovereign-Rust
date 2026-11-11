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

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sdt(signature: [u8; 4], length: usize) -> std::vec::Vec<u8> {
        let mut bytes = std::vec![0u8; length];
        bytes[0..4].copy_from_slice(&signature);
        bytes[4..8].copy_from_slice(&(length as u32).to_le_bytes());
        bytes[8] = 1;
        let checksum = 0u8.wrapping_sub(bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        bytes[9] = checksum;
        bytes
    }

    fn make_xsdt(fadt_address: u64) -> std::vec::Vec<u8> {
        let mut bytes = make_sdt(*b"XSDT", 44);
        bytes[36..44].copy_from_slice(&fadt_address.to_le_bytes());
        bytes[9] = 0;
        let checksum = 0u8.wrapping_sub(bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        bytes[9] = checksum;
        bytes
    }

    fn make_fadt(legacy_dsdt: u32, extended_dsdt: u64) -> std::vec::Vec<u8> {
        let mut bytes = make_sdt(*b"FACP", 148);
        bytes[40..44].copy_from_slice(&legacy_dsdt.to_le_bytes());
        bytes[140..148].copy_from_slice(&extended_dsdt.to_le_bytes());
        bytes[9] = 0;
        let checksum = 0u8.wrapping_sub(bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        bytes[9] = checksum;
        bytes
    }

    #[test]
    fn follows_extended_fadt_dsdt_pointer() {
        let root = make_xsdt(0x2000);
        let fadt = make_fadt(0x3000, 0x4000);
        let dsdt = make_sdt(*b"DSDT", 36);
        let rsdp = Rsdp {
            revision: 2,
            rsdt_address: 0,
            xsdt_address: Some(0x1000),
            length: 36,
        };

        let result = AcpiTables::discover(&rsdp, |address| match address {
            0x1000 => Some(root.as_slice()),
            0x2000 => Some(fadt.as_slice()),
            0x4000 => Some(dsdt.as_slice()),
            _ => None,
        }).unwrap();

        assert!(result.used_xsdt);
        assert_eq!(result.fadt, Some(0x2000));
        assert_eq!(result.dsdt, Some(0x4000));
    }

    #[test]
    fn rejects_fadt_dsdt_pointer_to_wrong_signature() {
        let root = make_xsdt(0x2000);
        let fadt = make_fadt(0x3000, 0);
        let wrong_table = make_sdt(*b"APIC", 36);
        let rsdp = Rsdp {
            revision: 2,
            rsdt_address: 0,
            xsdt_address: Some(0x1000),
            length: 36,
        };

        let result = AcpiTables::discover(&rsdp, |address| match address {
            0x1000 => Some(root.as_slice()),
            0x2000 => Some(fadt.as_slice()),
            0x3000 => Some(wrong_table.as_slice()),
            _ => None,
        });

        assert_eq!(result, Err(AcpiError::InvalidSignature));
    }
}
