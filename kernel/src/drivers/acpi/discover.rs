//! ACPI root-table discovery.
//!
//! The memory manager owns physical-to-virtual mapping. This module receives a
//! short-lived mapping callback and never constructs pointers from firmware
//! addresses itself. The DSDT is reached through the FADT, not the RSDT/XSDT.

use super::{
    AcpiError,
    fadt::Fadt,
    facs::Facs,
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
    pub facs: Option<u64>,
    pub hpet: Option<u64>,
    pub dsdt: Option<u64>,
}

/// Failure while converting discovered firmware data into interrupt topology.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MadtTopologyDiscoveryError {
    MissingMadt,
    Acpi(AcpiError),
    Topology(super::topology::MadtTopologyError),
}

impl AcpiTables {
    /// Parse the discovered MADT into a validated fixed-capacity interrupt
    /// topology. The supplied mapper remains owned by the boot/memory layer.
    pub fn interrupt_topology<'a, F>(
        &self,
        mut map_table: F,
    ) -> Result<super::topology::MadtInterruptTopology, MadtTopologyDiscoveryError>
    where
        F: FnMut(u64) -> Option<&'a [u8]>,
    {
        let address = self.madt.ok_or(MadtTopologyDiscoveryError::MissingMadt)?;
        let bytes = map_table(address).ok_or(
            MadtTopologyDiscoveryError::Acpi(AcpiError::UnsupportedAddress)
        )?;
        let madt = super::madt::Madt::parse(bytes)
            .map_err(MadtTopologyDiscoveryError::Acpi)?;
        super::topology::MadtInterruptTopology::parse(&madt)
            .map_err(MadtTopologyDiscoveryError::Topology)
    }

    /// Discover known tables using a caller-provided physical mapping function.
    ///
    /// The callback must return a slice covering the complete table at the
    /// requested physical address. The slice must remain valid for this call.
    /// Every returned SDT is checked for length and checksum before its
    /// signature is inspected. If a FADT is present, its legacy or extended
    /// DSDT pointer is followed and the target must validate as a DSDT.
    pub fn discover<'a, F>(rsdp: &Rsdp, map_table: F) -> Result<Self, AcpiError>
    where
        F: FnMut(u64) -> Option<&'a [u8]>,
    {
        // Share the legacy mapper between the two callbacks without creating
        // overlapping mutable captures of the same FnMut value.
        let mapper = core::cell::RefCell::new(map_table);
        Self::discover_with_mappers(
            rsdp,
            |address| (&mut *mapper.borrow_mut())(address),
            |address| (&mut *mapper.borrow_mut())(address),
        )
    }

    /// Discover ACPI tables with distinct SDT and FACS mapping policies.
    ///
    /// FACS is not an SDT: it has no checksum and uses a different minimum
    /// header size. Keeping its mapper separate prevents accidental use of the
    /// SDT mapping contract for firmware control structures.
    pub fn discover_with_mappers<'a, F, G>(
        rsdp: &Rsdp,
        mut map_table: F,
        mut map_facs: G,
    ) -> Result<Self, AcpiError>
    where
        F: FnMut(u64) -> Option<&'a [u8]>,
        G: FnMut(u64) -> Option<&'a [u8]>,
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

            // FACS is referenced by FADT.FIRMWARE_CTRL / X_FIRMWARE_CTRL,
            // not listed in the RSDT/XSDT. It has no SDT checksum. Prefer a
            // nonzero extended pointer, but retry the legacy pointer if the
            // mapping layer cannot use the extended address. A mapped but
            // malformed FACS is an error, not a fallback case.
            let extended_facs = fadt.x_firmware_ctrl.filter(|address| *address != 0);
            let mut facs_mapped = false;
            if let Some(address) = extended_facs {
                if let Some(bytes) = map_facs(address) {
                    let _facs = Facs::parse(address, bytes)?;
                    found.facs = Some(address);
                    facs_mapped = true;
                }
            }
            if !facs_mapped && fadt.firmware_ctrl != 0 {
                let address = fadt.firmware_ctrl as u64;
                let bytes = map_facs(address).ok_or(AcpiError::UnsupportedAddress)?;
                let _facs = Facs::parse(address, bytes)?;
                found.facs = Some(address);
            }

            // DSDT is reached through FADT, never through the root table.
            // Apply the same extended-first / legacy-if-unusable policy.
            // Once a candidate maps, its SDT and signature must validate;
            // malformed mapped tables must not be hidden by legacy fallback.
            let extended_dsdt = fadt.x_dsdt.filter(|address| *address != 0);
            let mut dsdt_mapped = false;
            if let Some(address) = extended_dsdt {
                if let Some(bytes) = map_table(address) {
                    let dsdt = Sdt::parse(bytes)?;
                    if dsdt.signature() != *b"DSDT" {
                        return Err(AcpiError::InvalidSignature);
                    }
                    found.dsdt = Some(address);
                    dsdt_mapped = true;
                }
            }
            if !dsdt_mapped && fadt.dsdt != 0 {
                let address = fadt.dsdt as u64;
                let bytes = map_table(address).ok_or(AcpiError::UnsupportedAddress)?;
                let dsdt = Sdt::parse(bytes)?;
                if dsdt.signature() != *b"DSDT" {
                    return Err(AcpiError::InvalidSignature);
                }
                found.dsdt = Some(address);
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
    fn follows_fadt_facs_pointer_without_applying_sdt_checksum() {
        let root = make_xsdt(0x2000);
        let mut fadt = make_fadt(0x3000, 0);
        fadt[36..40].copy_from_slice(&0x5000u32.to_le_bytes());
        fadt[9] = 0;
        let checksum = 0u8.wrapping_sub(fadt.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        fadt[9] = checksum;

        let mut facs = std::vec![0u8; 64];
        facs[0..4].copy_from_slice(b"FACS");
        facs[4..8].copy_from_slice(&64u32.to_le_bytes());
        facs[9] = 0xA5; // FACS has no checksum field.

        let rsdp = Rsdp {
            revision: 2,
            rsdt_address: 0,
            xsdt_address: Some(0x1000),
            length: 36,
        };

        let result = AcpiTables::discover(&rsdp, |address| match address {
            0x1000 => Some(root.as_slice()),
            0x2000 => Some(fadt.as_slice()),
            0x5000 => Some(facs.as_slice()),
            _ => None,
        }).unwrap();

        assert_eq!(result.facs, Some(0x5000));
    }

    #[test]
    fn uses_dedicated_mapper_for_facs() {
        let root = make_xsdt(0x2000);
        let mut fadt = make_fadt(0, 0);
        fadt[36..40].copy_from_slice(&0x5000u32.to_le_bytes());
        fadt[9] = 0;
        let checksum = 0u8.wrapping_sub(fadt.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        fadt[9] = checksum;
        let mut facs = std::vec![0u8; 64];
        facs[0..4].copy_from_slice(b"FACS");
        facs[4..8].copy_from_slice(&64u32.to_le_bytes());
        let rsdp = Rsdp {
            revision: 2,
            rsdt_address: 0,
            xsdt_address: Some(0x1000),
            length: 36,
        };

        let result = AcpiTables::discover_with_mappers(
            &rsdp,
            |address| match address {
                0x1000 => Some(root.as_slice()),
                0x2000 => Some(fadt.as_slice()),
                _ => None,
            },
            |address| if address == 0x5000 { Some(facs.as_slice()) } else { None },
        ).unwrap();
        assert_eq!(result.facs, Some(0x5000));
    }

    #[test]
    fn falls_back_to_legacy_dsdt_when_extended_pointer_cannot_be_mapped() {
        let root = make_xsdt(0x2000);
        let fadt = make_fadt(0x3000, 0x1_0000_3000);
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
            0x3000 => Some(dsdt.as_slice()),
            _ => None,
        }).unwrap();

        assert_eq!(result.dsdt, Some(0x3000));
    }

    #[test]
    fn does_not_hide_malformed_mapped_extended_dsdt_with_legacy_pointer() {
        let root = make_xsdt(0x2000);
        let fadt = make_fadt(0x3000, 0x4000);
        let wrong_table = make_sdt(*b"APIC", 36);
        let legacy_dsdt = make_sdt(*b"DSDT", 36);
        let rsdp = Rsdp {
            revision: 2,
            rsdt_address: 0,
            xsdt_address: Some(0x1000),
            length: 36,
        };

        let result = AcpiTables::discover(&rsdp, |address| match address {
            0x1000 => Some(root.as_slice()),
            0x2000 => Some(fadt.as_slice()),
            0x3000 => Some(legacy_dsdt.as_slice()),
            0x4000 => Some(wrong_table.as_slice()),
            _ => None,
        });

        assert_eq!(result, Err(AcpiError::InvalidSignature));
    }

    #[test]
    fn falls_back_to_legacy_facs_when_extended_pointer_cannot_be_mapped() {
        let root = make_xsdt(0x2000);
        let mut fadt = make_fadt(0, 0);
        fadt[36..40].copy_from_slice(&0x5000u32.to_le_bytes());
        fadt[132..140].copy_from_slice(&0x1_0000_5000u64.to_le_bytes());
        fadt[9] = 0;
        let checksum = 0u8.wrapping_sub(fadt.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        fadt[9] = checksum;

        let mut facs = std::vec![0u8; 64];
        facs[0..4].copy_from_slice(b"FACS");
        facs[4..8].copy_from_slice(&64u32.to_le_bytes());
        let rsdp = Rsdp {
            revision: 2,
            rsdt_address: 0,
            xsdt_address: Some(0x1000),
            length: 36,
        };

        let result = AcpiTables::discover_with_mappers(
            &rsdp,
            |address| match address {
                0x1000 => Some(root.as_slice()),
                0x2000 => Some(fadt.as_slice()),
                _ => None,
            },
            |address| if address == 0x5000 { Some(facs.as_slice()) } else { None },
        ).unwrap();

        assert_eq!(result.facs, Some(0x5000));
    }

    #[test]
    fn does_not_hide_malformed_mapped_extended_facs_with_legacy_pointer() {
        let root = make_xsdt(0x2000);
        let mut fadt = make_fadt(0, 0);
        fadt[36..40].copy_from_slice(&0x5000u32.to_le_bytes());
        fadt[132..140].copy_from_slice(&0x6000u64.to_le_bytes());
        fadt[9] = 0;
        fadt[9] = 0u8.wrapping_sub(fadt.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));

        let mut malformed_facs = std::vec![0u8; 64];
        malformed_facs[0..4].copy_from_slice(b"BAD!");
        malformed_facs[4..8].copy_from_slice(&64u32.to_le_bytes());
        let mut legacy_facs = std::vec![0u8; 64];
        legacy_facs[0..4].copy_from_slice(b"FACS");
        legacy_facs[4..8].copy_from_slice(&64u32.to_le_bytes());
        let rsdp = Rsdp {
            revision: 2,
            rsdt_address: 0,
            xsdt_address: Some(0x1000),
            length: 36,
        };

        let result = AcpiTables::discover_with_mappers(
            &rsdp,
            |address| match address {
                0x1000 => Some(root.as_slice()),
                0x2000 => Some(fadt.as_slice()),
                _ => None,
            },
            |address| match address {
                0x6000 => Some(malformed_facs.as_slice()),
                0x5000 => Some(legacy_facs.as_slice()),
                _ => None,
            },
        );

        assert_eq!(result, Err(AcpiError::InvalidSignature));
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
