//! Fixed-capacity ACPI interrupt-controller topology extracted from MADT.
//!
//! This layer owns firmware interpretation only. It does not map MMIO, probe
//! I/O APIC registers, allocate memory, or enable interrupt delivery. The
//! hardware layer must validate each controller's actual redirection count
//! before constructing the final GSI ranges.

use super::madt::{InterruptSourceOverride, IoApic, Madt};

pub const MAX_MADT_IO_APICS: usize = 16;
pub const MAX_MADT_OVERRIDES: usize = 256;

/// ACPI-specific topology validation failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MadtTopologyError {
    MalformedRecord,
    CapacityExceeded,
    InvalidControllerAddress,
    DuplicateIoApicId,
    DuplicateIoApicAddress,
    DuplicateOverrideSource,
    DuplicateLocalApicAddressOverride,
    InvalidOverrideBus,
    InvalidOverrideSource,
    InvalidOverrideFlags,
}

/// Validated, allocation-free interrupt-controller metadata from one MADT.
pub struct MadtInterruptTopology {
    io_apics: [Option<IoApic>; MAX_MADT_IO_APICS],
    io_apic_count: usize,
    overrides: [Option<InterruptSourceOverride>; MAX_MADT_OVERRIDES],
    override_count: usize,
    local_apic_address_override: Option<u64>,
}

impl MadtInterruptTopology {
    /// Extract all I/O APIC descriptors, interrupt-source overrides and the
    /// optional 64-bit Local APIC address override.
    ///
    /// The table parser has already validated the SDT checksum and record
    /// boundaries. This pass validates semantic uniqueness and fixed capacity.
    pub fn parse(madt: &Madt<'_>) -> Result<Self, MadtTopologyError> {
        let mut topology = Self {
            io_apics: [None; MAX_MADT_IO_APICS],
            io_apic_count: 0,
            overrides: [None; MAX_MADT_OVERRIDES],
            override_count: 0,
            local_apic_address_override: None,
        };

        for record in madt.records() {
            let record = record.map_err(|_| MadtTopologyError::MalformedRecord)?;
            match record.record_type {
                // I/O APIC (type 1): ID, reserved, MMIO address, GSI base.
                1 => {
                    if record.data.len() != 10 || record.data[1] != 0 {
                        return Err(MadtTopologyError::MalformedRecord);
                    }
                    let descriptor = record.io_apic()
                        .ok_or(MadtTopologyError::MalformedRecord)?;
                    if descriptor.address == 0 || descriptor.address & 0xFFF != 0 {
                        return Err(MadtTopologyError::InvalidControllerAddress);
                    }
                    if topology.io_apics[..topology.io_apic_count]
                        .iter().flatten().any(|existing| existing.id == descriptor.id)
                    {
                        return Err(MadtTopologyError::DuplicateIoApicId);
                    }
                    if topology.io_apics[..topology.io_apic_count]
                        .iter().flatten().any(|existing| existing.address == descriptor.address)
                    {
                        return Err(MadtTopologyError::DuplicateIoApicAddress);
                    }
                    if topology.io_apic_count == MAX_MADT_IO_APICS {
                        return Err(MadtTopologyError::CapacityExceeded);
                    }
                    topology.io_apics[topology.io_apic_count] = Some(descriptor);
                    topology.io_apic_count += 1;
                }
                // Interrupt Source Override (type 2).
                2 => {
                    if record.data.len() != 8 {
                        return Err(MadtTopologyError::MalformedRecord);
                    }
                    let entry = record.interrupt_source_override()
                        .ok_or(MadtTopologyError::MalformedRecord)?;
                    if entry.bus != 0 {
                        return Err(MadtTopologyError::InvalidOverrideBus);
                    }
                    // ISA IRQ inputs are 0..=15. MPS INTI polarity and
                    // trigger encodings 0b10 are reserved; bits 4..15
                    // must also remain clear.
                    if entry.source > 15 {
                        return Err(MadtTopologyError::InvalidOverrideSource);
                    }
                    let polarity = entry.flags & 0b11;
                    let trigger = (entry.flags >> 2) & 0b11;
                    if polarity == 0b10 || trigger == 0b10 || entry.flags & 0xFFF0 != 0 {
                        return Err(MadtTopologyError::InvalidOverrideFlags);
                    }
                    if topology.overrides[..topology.override_count]
                        .iter().flatten().any(|existing|
                            existing.bus == entry.bus && existing.source == entry.source)
                    {
                        return Err(MadtTopologyError::DuplicateOverrideSource);
                    }
                    if topology.override_count == MAX_MADT_OVERRIDES {
                        return Err(MadtTopologyError::CapacityExceeded);
                    }
                    topology.overrides[topology.override_count] = Some(entry);
                    topology.override_count += 1;
                }
                // Processor Local APIC Address Override (type 5): reserved
                // u16 followed by a 64-bit physical address.
                5 => {
                    if record.data.len() != 10 || record.data[0] != 0 || record.data[1] != 0 {
                        return Err(MadtTopologyError::MalformedRecord);
                    }
                    let address = u64::from_le_bytes([
                        record.data[2], record.data[3], record.data[4], record.data[5],
                        record.data[6], record.data[7], record.data[8], record.data[9],
                    ]);
                    if address == 0 || address & 0xFFF != 0 {
                        return Err(MadtTopologyError::InvalidControllerAddress);
                    }
                    if topology.local_apic_address_override.replace(address).is_some() {
                        return Err(MadtTopologyError::DuplicateLocalApicAddressOverride);
                    }
                }
                _ => {}
            }
        }

        Ok(topology)
    }

    pub fn io_apics(&self) -> impl Iterator<Item = IoApic> + '_ {
        self.io_apics[..self.io_apic_count].iter().filter_map(|entry| *entry)
    }

    pub fn overrides(&self) -> impl Iterator<Item = InterruptSourceOverride> + '_ {
        self.overrides[..self.override_count].iter().filter_map(|entry| *entry)
    }

    pub fn io_apic_count(&self) -> usize { self.io_apic_count }
    pub fn override_count(&self) -> usize { self.override_count }

    /// Physical LAPIC base override from MADT type 5, if firmware supplies it.
    /// This is metadata only; x2APIC uses the APIC-base MSR and does not map it.
    pub const fn local_apic_address_override(&self) -> Option<u64> {
        self.local_apic_address_override
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drivers::acpi::{checksum, madt::MADT_SIGNATURE};

    fn madt_with_records(records: &[u8]) -> std::vec::Vec<u8> {
        let length = 44 + records.len();
        let mut bytes = std::vec![0u8; length];
        bytes[..4].copy_from_slice(&MADT_SIGNATURE);
        bytes[4..8].copy_from_slice(&(length as u32).to_le_bytes());
        bytes[8] = 1;
        // MADT fixed payload: LAPIC address and flags.
        bytes[36..40].copy_from_slice(&0xFEE0_0000u32.to_le_bytes());
        bytes[40..44].copy_from_slice(&1u32.to_le_bytes());
        bytes[44..].copy_from_slice(records);
        bytes[9] = 0u8.wrapping_sub(checksum(&bytes));
        bytes
    }

    #[test]
    fn extracts_multiple_io_apics_overrides_and_lapic_override() {
        let records = [
            1, 12, 2, 0, 0x00, 0x10, 0x00, 0xFE, 0, 0, 0, 0,
            1, 12, 5, 0, 0x00, 0x20, 0x00, 0xFE, 0x20, 0, 0, 0,
            2, 10, 0, 1, 0x25, 0, 0, 0, 0x0F, 0,
            5, 12, 0, 0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFE,
        ];
        let bytes = madt_with_records(&records);
        let madt = Madt::parse(&bytes).unwrap();
        let topology = MadtInterruptTopology::parse(&madt).unwrap();

        assert_eq!(topology.io_apic_count(), 2);
        assert_eq!(topology.override_count(), 1);
        let io: std::vec::Vec<_> = topology.io_apics().collect();
        assert_eq!(io[0].id, 2);
        assert_eq!(io[1].global_system_interrupt_base, 32);
        let overrides: std::vec::Vec<_> = topology.overrides().collect();
        assert_eq!(overrides[0].source, 1);
        assert_eq!(overrides[0].global_system_interrupt, 37);
        assert_eq!(topology.local_apic_address_override(), Some(0xFE00_0000_0000_0000));
    }

    #[test]
    fn rejects_duplicate_io_apic_ids_and_addresses() {
        let duplicate_id = [
            1, 12, 1, 0, 0x00, 0x10, 0x00, 0xFE, 0, 0, 0, 0,
            1, 12, 1, 0, 0x00, 0x20, 0x00, 0xFE, 8, 0, 0, 0,
        ];
        let bytes = madt_with_records(&duplicate_id);
        let madt = Madt::parse(&bytes).unwrap();
        assert_eq!(MadtInterruptTopology::parse(&madt).err(),
            Some(MadtTopologyError::DuplicateIoApicId));

        let duplicate_address = [
            1, 12, 1, 0, 0x00, 0x10, 0x00, 0xFE, 0, 0, 0, 0,
            1, 12, 2, 0, 0x00, 0x10, 0x00, 0xFE, 8, 0, 0, 0,
        ];
        let bytes = madt_with_records(&duplicate_address);
        let madt = Madt::parse(&bytes).unwrap();
        assert_eq!(MadtInterruptTopology::parse(&madt).err(),
            Some(MadtTopologyError::DuplicateIoApicAddress));
    }

    #[test]
    fn rejects_non_isa_interrupt_source_overrides() {
        let records = [2, 10, 1, 1, 0x25, 0, 0, 0, 0x0F, 0];
        let bytes = madt_with_records(&records);
        let madt = Madt::parse(&bytes).unwrap();
        assert_eq!(
            MadtInterruptTopology::parse(&madt).err(),
            Some(MadtTopologyError::InvalidOverrideBus),
        );
    }

    #[test]
    fn rejects_out_of_range_isa_irq_sources() {
        let records = [2, 10, 0, 16, 0x25, 0, 0, 0, 0x0F, 0];
        let bytes = madt_with_records(&records);
        let madt = Madt::parse(&bytes).unwrap();
        assert_eq!(
            MadtInterruptTopology::parse(&madt).err(),
            Some(MadtTopologyError::InvalidOverrideSource),
        );
    }

    #[test]
    fn rejects_reserved_interrupt_override_flag_encodings() {
        for flags in [0x0002u16, 0x0008, 0x0010, 0x8000] {
            let mut records = [2, 10, 0, 1, 0x25, 0, 0, 0, 0, 0];
            records[8..10].copy_from_slice(&flags.to_le_bytes());
            let bytes = madt_with_records(&records);
            let madt = Madt::parse(&bytes).unwrap();
            assert_eq!(
                MadtInterruptTopology::parse(&madt).err(),
                Some(MadtTopologyError::InvalidOverrideFlags),
                "flags {flags:#06x}",
            );
        }
    }

    #[test]
    fn rejects_nonzero_reserved_bytes_in_controller_records() {
        let io_apic = [1, 12, 1, 1, 0x00, 0x10, 0x00, 0xFE, 0, 0, 0, 0];
        let bytes = madt_with_records(&io_apic);
        let madt = Madt::parse(&bytes).unwrap();
        assert_eq!(
            MadtInterruptTopology::parse(&madt).err(),
            Some(MadtTopologyError::MalformedRecord),
        );

        let lapic_override = [5, 12, 1, 0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFE];
        let bytes = madt_with_records(&lapic_override);
        let madt = Madt::parse(&bytes).unwrap();
        assert_eq!(
            MadtInterruptTopology::parse(&madt).err(),
            Some(MadtTopologyError::MalformedRecord),
        );
    }

    #[test]
    fn rejects_duplicate_local_apic_address_overrides() {
        let override_record = [5, 12, 0, 0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFE];
        let records = [override_record.as_slice(), override_record.as_slice()].concat();
        let bytes = madt_with_records(&records);
        let madt = Madt::parse(&bytes).unwrap();
        assert_eq!(
            MadtInterruptTopology::parse(&madt).err(),
            Some(MadtTopologyError::DuplicateLocalApicAddressOverride),
        );
    }

    #[test]
    fn rejects_unaligned_io_apic_address() {
        let records = [1, 12, 1, 0, 0x01, 0x10, 0x00, 0xFE, 0, 0, 0, 0];
        let bytes = madt_with_records(&records);
        let madt = Madt::parse(&bytes).unwrap();
        assert_eq!(MadtInterruptTopology::parse(&madt).err(),
            Some(MadtTopologyError::InvalidControllerAddress));
    }
}
