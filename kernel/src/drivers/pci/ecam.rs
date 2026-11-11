//! PCIe ECAM discovery and memory-mapped configuration access.
use core::marker::PhantomData;
use core::ptr::NonNull;
use super::config::{ConfigAccess, ConfigError, EcamWindow, PciAddress, PCIE_CONFIG_SIZE};
use crate::drivers::acpi::{checksum_valid, AcpiError};

pub const MCFG_HEADER_SIZE: usize = 44;
pub const MCFG_ENTRY_SIZE: usize = 16;
const ECAM_BUS_SIZE: usize = 1 << 20;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McfgError { Acpi(AcpiError), InvalidSignature, InvalidLength, InvalidEntry, OutputTooSmall }
impl From<AcpiError> for McfgError {
    fn from(value: AcpiError) -> Self { Self::Acpi(value) }
}
fn u16le(b: &[u8], i: usize) -> u16 { u16::from_le_bytes([b[i], b[i + 1]]) }
fn u32le(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}
fn u64le(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes([b[i], b[i+1], b[i+2], b[i+3], b[i+4], b[i+5], b[i+6], b[i+7]])
}

/// Parse validated MCFG bytes; this never dereferences physical addresses.
pub fn parse_mcfg(table: &[u8], out: &mut [EcamWindow]) -> Result<usize, McfgError> {
    if table.len() < MCFG_HEADER_SIZE { return Err(McfgError::Acpi(AcpiError::BufferTooSmall)); }
    if &table[..4] != b"MCFG" { return Err(McfgError::InvalidSignature); }
    let length = u32le(table, 4) as usize;
    if length < MCFG_HEADER_SIZE || length > table.len()
        || (length - MCFG_HEADER_SIZE) % MCFG_ENTRY_SIZE != 0 {
        return Err(McfgError::InvalidLength);
    }
    if !checksum_valid(&table[..length]) { return Err(McfgError::Acpi(AcpiError::InvalidChecksum)); }
    if table[36..44].iter().any(|v| *v != 0) { return Err(McfgError::InvalidEntry); }
    let count = (length - MCFG_HEADER_SIZE) / MCFG_ENTRY_SIZE;
    if count > out.len() { return Err(McfgError::OutputTooSmall); }
    for (i, e) in table[MCFG_HEADER_SIZE..length].chunks_exact(MCFG_ENTRY_SIZE).enumerate() {
        let base = u64le(e, 0);
        let start = e[10];
        let end = e[11];
        if base & 0x000f_ffff != 0 || start > end || u32le(e, 12) != 0 {
            return Err(McfgError::InvalidEntry);
        }
        out[i] = EcamWindow { segment: u16le(e, 8), start_bus: start, end_bus: end, physical_base: base };
    }
    Ok(count)
}

/// Calculate an ECAM physical address without mapping or dereferencing it.
pub fn physical_address(w: EcamWindow, a: PciAddress, r: u16) -> Result<u64, ConfigError> {
    w.physical_base.checked_add(w.offset(a, r).ok_or(ConfigError::OutOfRange)?)
        .ok_or(ConfigError::OutOfRange)
}

/// Access an ECAM range already mapped into the kernel virtual address space.
pub struct EcamAccess<'a> {
    window: EcamWindow,
    mapped_base: NonNull<u8>,
    mapped_len: usize,
    _mapping_lifetime: PhantomData<&'a mut [u8]>,
}

impl<'a> EcamAccess<'a> {
    /// Create an accessor for a pre-existing MMIO mapping.
    ///
    /// # Safety
    /// mapped_base must begin a live writable device-memory mapping of mapped_len
    /// bytes corresponding to window's physical ECAM range. It must remain mapped
    /// for 'a, have appropriate device/uncached page attributes, and must not be
    /// aliased by ordinary Rust references during this lifetime.
    pub unsafe fn new(
        window: EcamWindow,
        mapped_base: *mut u8,
        mapped_len: usize,
    ) -> Result<Self, ConfigError> {
        if window.start_bus > window.end_bus || window.physical_base & 0x000f_ffff != 0 {
            return Err(ConfigError::MappingUnavailable);
        }
        let buses = (window.end_bus as usize).checked_sub(window.start_bus as usize)
            .and_then(|n| n.checked_add(1)).ok_or(ConfigError::MappingUnavailable)?;
        let required = buses.checked_mul(ECAM_BUS_SIZE).ok_or(ConfigError::MappingUnavailable)?;
        if mapped_len < required { return Err(ConfigError::MappingUnavailable); }
        let mapped_base = NonNull::new(mapped_base).ok_or(ConfigError::MappingUnavailable)?;
        if (mapped_base.as_ptr() as usize) & 3 != 0 {
            return Err(ConfigError::MappingUnavailable);
        }
        Ok(Self { window, mapped_base, mapped_len, _mapping_lifetime: PhantomData })
    }

    fn register_pointer(&self, a: PciAddress, r: u16) -> Result<*mut u32, ConfigError> {
        if a.device >= 32 || a.function >= 8 { return Err(ConfigError::InvalidAddress); }
        if r & 3 != 0 { return Err(ConfigError::UnalignedRegister); }
        if r >= PCIE_CONFIG_SIZE { return Err(ConfigError::OutOfRange); }
        let offset = self.window.offset(a, r).ok_or(ConfigError::OutOfRange)?;
        let end = offset.checked_add(4).ok_or(ConfigError::OutOfRange)?;
        if end > self.mapped_len as u64 { return Err(ConfigError::MappingUnavailable); }
        // The constructor contract guarantees this pointer is inside the live mapping.
        Ok(unsafe { self.mapped_base.as_ptr().add(offset as usize).cast::<u32>() })
    }
}

impl ConfigAccess for EcamAccess<'_> {
    fn read_u32(&self, a: PciAddress, r: u16) -> Result<u32, ConfigError> {
        let ptr = self.register_pointer(a, r)?;
        Ok(unsafe { core::ptr::read_volatile(ptr) })
    }
    fn write_u32(&self, a: PciAddress, r: u16, value: u32) -> Result<(), ConfigError> {
        let ptr = self.register_pointer(a, r)?;
        unsafe { core::ptr::write_volatile(ptr, value) };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[repr(align(4096))]
    struct Aligned([u8; ECAM_BUS_SIZE]);

    #[test]
    fn physical_address_rejects_bus_outside_window() {
        let w = EcamWindow { segment: 0, start_bus: 4, end_bus: 4, physical_base: 0xe000_0000 };
        let a = PciAddress { segment: 0, bus: 5, device: 0, function: 0 };
        assert_eq!(physical_address(w, a, 0), Err(ConfigError::OutOfRange));
    }

    #[test]
    fn mapped_ecam_reads_and_writes_volatile_registers() {
        let mut memory = Aligned([0; ECAM_BUS_SIZE]);
        let w = EcamWindow { segment: 0, start_bus: 0, end_bus: 0, physical_base: 0xe000_0000 };
        let access = unsafe { EcamAccess::new(w, memory.0.as_mut_ptr(), memory.0.len()).unwrap() };
        let a = PciAddress { segment: 0, bus: 0, device: 2, function: 1 };
        access.write_u32(a, 0x10, 0x1234_5678).unwrap();
        assert_eq!(access.read_u32(a, 0x10), Ok(0x1234_5678));
    }

    #[test]
    fn mapped_ecam_validates_register_alignment_and_bounds() {
        let mut memory = Aligned([0; ECAM_BUS_SIZE]);
        let w = EcamWindow { segment: 0, start_bus: 0, end_bus: 0, physical_base: 0xe000_0000 };
        let access = unsafe { EcamAccess::new(w, memory.0.as_mut_ptr(), memory.0.len()).unwrap() };
        let a = PciAddress { segment: 0, bus: 0, device: 0, function: 0 };
        assert_eq!(access.read_u32(a, 2), Err(ConfigError::UnalignedRegister));
        assert_eq!(access.read_u32(a, PCIE_CONFIG_SIZE), Err(ConfigError::OutOfRange));
    }

    #[test]
    fn mapped_ecam_requires_entire_window() {
        let mut memory = [0u8; 4096];
        let w = EcamWindow { segment: 0, start_bus: 0, end_bus: 0, physical_base: 0xe000_0000 };
        assert!(matches!(unsafe { EcamAccess::new(w, memory.as_mut_ptr(), memory.len()) },
            Err(ConfigError::MappingUnavailable)));
    }
}
