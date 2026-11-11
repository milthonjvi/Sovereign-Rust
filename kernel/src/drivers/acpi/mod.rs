//! ACPI firmware-table discovery and validated platform metadata.
//!
//! This module is allocation-free and does not dereference firmware physical
//! addresses. The boot/memory subsystem must map a physical table and pass its
//! bytes to these parsers. All lengths and checksums are validated before data
//! is exposed.

pub mod discover;
pub mod fadt;
pub mod madt;
pub mod rsdp;
pub mod sdt;

/// Errors returned while discovering or parsing ACPI data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AcpiError {
    BufferTooSmall,
    InvalidSignature,
    InvalidRevision,
    InvalidLength,
    InvalidChecksum,
    UnsupportedAddress,
    MisalignedTable,
    MalformedTable,
    UnknownRecord,
}

/// ACPI specification signatures are four-byte ASCII identifiers.
pub type Signature = [u8; 4];

/// Compute the ACPI 8-bit additive checksum.
pub fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte))
}

/// A checksum is valid when the wrapping sum of all bytes is zero.
pub fn checksum_valid(bytes: &[u8]) -> bool {
    checksum(bytes) == 0
}
