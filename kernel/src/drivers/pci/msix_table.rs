//! MSI-X table access over a caller-provided, already-mapped BAR aperture.
//!
//! This module does not map physical memory. The memory subsystem must provide
//! an uncached/device mapping for the BAR table region before construction.

use core::marker::PhantomData;
use core::ptr::NonNull;
use super::msi::MsixCapability;

const ENTRY_BYTES: usize = 16;
const VECTOR_CONTROL_MASK: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MsixTableError {
    InvalidMapping,
    OutOfRange,
    MisalignedMessageAddress,
    InvalidMessageData,
    InvalidVector,
    HandlerNotRegistered,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MsixEntry {
    pub message_address: u64,
    pub message_data: u32,
    pub masked: bool,
}

/// Exclusive access to an MSI-X table in a mapped BAR aperture.
pub struct MsixTable<'a> {
    base: NonNull<u8>,
    mapped_len: usize,
    entries: u16,
    _mapping_lifetime: PhantomData<&'a mut [u8]>,
}

impl<'a> MsixTable<'a> {
    /// Construct access to the table region identified by an MSI-X capability.
    ///
    /// # Safety
    /// `mapped_base` must point to a live, writable, uncached/device mapping of
    /// the MSI-X table region in the BAR selected by `capability.table_bar`.
    /// The mapping must cover at least `table_bytes()`, remain valid for `'a`,
    /// and have no concurrent aliases or other writers for the same table.
    pub unsafe fn new(
        capability: MsixCapability,
        mapped_base: *mut u8,
        mapped_len: usize,
    ) -> Result<Self, MsixTableError> {
        let required = capability.table_bytes().ok_or(MsixTableError::InvalidMapping)? as usize;
        if required == 0 || mapped_len < required {
            return Err(MsixTableError::InvalidMapping);
        }
        let base = NonNull::new(mapped_base).ok_or(MsixTableError::InvalidMapping)?;
        if (base.as_ptr() as usize) & 3 != 0 {
            return Err(MsixTableError::InvalidMapping);
        }
        Ok(Self {
            base,
            mapped_len,
            entries: capability.table_size,
            _mapping_lifetime: PhantomData,
        })
    }

    fn entry_pointer(&self, index: u16) -> Result<*mut u32, MsixTableError> {
        if index >= self.entries {
            return Err(MsixTableError::OutOfRange);
        }
        let offset = (index as usize).checked_mul(ENTRY_BYTES)
            .ok_or(MsixTableError::OutOfRange)?;
        let end = offset.checked_add(ENTRY_BYTES).ok_or(MsixTableError::OutOfRange)?;
        if end > self.mapped_len {
            return Err(MsixTableError::InvalidMapping);
        }
        // SAFETY: Construction validates the mapped extent and index bounds.
        Ok(unsafe { self.base.as_ptr().add(offset).cast::<u32>() })
    }

    /// Read one table entry using volatile MMIO loads.
    pub fn read_entry(&self, index: u16) -> Result<MsixEntry, MsixTableError> {
        let entry = self.entry_pointer(index)?;
        // SAFETY: entry_pointer returns four aligned dwords inside the mapping.
        let address_low = unsafe { core::ptr::read_volatile(entry) };
        let address_high = unsafe { core::ptr::read_volatile(entry.add(1)) };
        let message_data = unsafe { core::ptr::read_volatile(entry.add(2)) };
        let vector_control = unsafe { core::ptr::read_volatile(entry.add(3)) };
        Ok(MsixEntry {
            message_address: ((address_high as u64) << 32) | address_low as u64,
            message_data,
            masked: vector_control & VECTOR_CONTROL_MASK != 0,
        })
    }

    /// Program one entry, masking it during the update and optionally unmasking
    /// it only after address and data have been written.
    ///
    /// # Safety
    /// The MSI-X function must be disabled or function-masked while its table is
    /// being initialized. The caller must ensure the supplied APIC vector is
    /// allocated to this device and its handler remains valid for the kernel
    /// lifetime. Use `enable_msix_masked` and clear the function mask only after
    /// all intended entries have been programmed.
    pub unsafe fn program_entry(
        &mut self,
        index: u16,
        message_address: u64,
        message_data: u32,
        masked: bool,
    ) -> Result<(), MsixTableError> {
        if message_address & 0x3 != 0 {
            return Err(MsixTableError::MisalignedMessageAddress);
        }
        if message_data & !0xff != 0 {
            return Err(MsixTableError::InvalidMessageData);
        }
        let vector = message_data as u8;
        if !crate::interrupts::apic::is_apic_vector(vector) {
            return Err(MsixTableError::InvalidVector);
        }
        if !crate::interrupts::dispatch::is_registered(vector) {
            return Err(MsixTableError::HandlerNotRegistered);
        }

        let entry = self.entry_pointer(index)?;
        let control = unsafe { entry.add(3) };
        // SAFETY: The entry is inside the exclusive mapped MSI-X table.
        unsafe { core::ptr::write_volatile(control, VECTOR_CONTROL_MASK) };
        // Read-back flushes the mask write before the message fields change.
        let _ = unsafe { core::ptr::read_volatile(control) };
        unsafe { core::ptr::write_volatile(entry, message_address as u32) };
        unsafe { core::ptr::write_volatile(entry.add(1), (message_address >> 32) as u32) };
        unsafe { core::ptr::write_volatile(entry.add(2), message_data) };
        let final_control = if masked { VECTOR_CONTROL_MASK } else { 0 };
        unsafe { core::ptr::write_volatile(control, final_control) };
        let _ = unsafe { core::ptr::read_volatile(control) };
        Ok(())
    }

    /// Change only the per-vector mask bit of an existing table entry.
    ///
    /// # Safety
    /// The caller must own this table mapping exclusively and must not race
    /// another CPU or driver updating the same vector entry.
    pub unsafe fn set_vector_mask(&mut self, index: u16, masked: bool) -> Result<(), MsixTableError> {
        if !masked {
            let current = self.read_entry(index)?;
            if current.message_data & !0xff != 0 {
                return Err(MsixTableError::InvalidMessageData);
            }
            let vector = current.message_data as u8;
            if !crate::interrupts::apic::is_apic_vector(vector) {
                return Err(MsixTableError::InvalidVector);
            }
            if !crate::interrupts::dispatch::is_registered(vector) {
                return Err(MsixTableError::HandlerNotRegistered);
            }
        }
        let entry = self.entry_pointer(index)?;
        let control = unsafe { entry.add(3) };
        let value = if masked { VECTOR_CONTROL_MASK } else { 0 };
        unsafe { core::ptr::write_volatile(control, value) };
        let _ = unsafe { core::ptr::read_volatile(control) };
        Ok(())
    }
}

/// Read-only access to an MSI-X pending-bit array in a mapped BAR aperture.
pub struct MsixPendingBitArray<'a> {
    base: NonNull<u8>,
    mapped_len: usize,
    vectors: u16,
    _mapping_lifetime: PhantomData<&'a [u8]>,
}

impl<'a> MsixPendingBitArray<'a> {
    /// Construct access to the PBA region identified by an MSI-X capability.
    ///
    /// # Safety
    /// mapped_base must point to a live, uncached/device mapping of the PBA
    /// region in capability.pending_bit_array_bar. It must cover the required
    /// PBA bytes, remain valid for 'a, and not be concurrently unmapped.
    pub unsafe fn new(
        capability: MsixCapability,
        mapped_base: *mut u8,
        mapped_len: usize,
    ) -> Result<Self, MsixTableError> {
        let required = capability.pending_bit_array_bytes()
            .ok_or(MsixTableError::InvalidMapping)? as usize;
        if required == 0 || mapped_len < required {
            return Err(MsixTableError::InvalidMapping);
        }
        let base = NonNull::new(mapped_base).ok_or(MsixTableError::InvalidMapping)?;
        if (base.as_ptr() as usize) & 7 != 0 {
            return Err(MsixTableError::InvalidMapping);
        }
        Ok(Self {
            base,
            mapped_len,
            vectors: capability.table_size,
            _mapping_lifetime: PhantomData,
        })
    }

    /// Return the device-reported pending state for one MSI-X table vector.
    pub fn is_pending(&self, vector: u16) -> Result<bool, MsixTableError> {
        if vector >= self.vectors {
            return Err(MsixTableError::OutOfRange);
        }
        let word_offset = (vector as usize / 64) * 8;
        let end = word_offset.checked_add(8).ok_or(MsixTableError::OutOfRange)?;
        if end > self.mapped_len {
            return Err(MsixTableError::InvalidMapping);
        }
        // SAFETY: Constructor and bounds checks guarantee an aligned in-range u64.
        let word = unsafe {
            core::ptr::read_volatile(self.base.as_ptr().add(word_offset).cast::<u64>())
        };
        Ok(word & (1u64 << (vector % 64)) != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[repr(align(8))]
    struct TableMemory([u32; 4]);

    extern "C" fn table_test_handler(_vector: u8) {}

    fn capability() -> MsixCapability {
        MsixCapability {
            offset: 0x60, enabled: false, function_masked: true, table_size: 1,
            table_bar: 0, table_offset: 0, pending_bit_array_bar: 0,
            pending_bit_array_offset: 0x100,
        }
    }

    #[test]
    fn programs_entry_masked_then_unmasks_after_fields() {
        let mut memory = TableMemory([0; 4]);
        let mut table = unsafe {
            MsixTable::new(capability(), memory.0.as_mut_ptr().cast::<u8>(), 16).unwrap()
        };
        crate::interrupts::dispatch::register(0x50, table_test_handler).unwrap();
        let result = unsafe { table.program_entry(0, 0xfee0_0000, 0x50, false) };
        assert_eq!(result, Ok(()));
        assert_eq!(table.read_entry(0), Ok(MsixEntry {
            message_address: 0xfee0_0000, message_data: 0x50, masked: false,
        }));
        crate::interrupts::dispatch::unregister(0x50, table_test_handler).unwrap();
    }

    #[test]
    fn reads_pending_bit_array_across_multiple_words() {
        #[repr(align(8))]
        struct PendingMemory([u64; 2]);
        let mut memory = PendingMemory([0, 1]);
        let cap = MsixCapability { table_size: 65, ..capability() };
        let pba = unsafe {
            MsixPendingBitArray::new(cap, memory.0.as_mut_ptr().cast::<u8>(), 16).unwrap()
        };
        assert_eq!(pba.is_pending(63), Ok(false));
        assert_eq!(pba.is_pending(64), Ok(true));
        assert_eq!(pba.is_pending(65), Err(MsixTableError::OutOfRange));
    }

    #[test]
    fn rejects_out_of_range_table_entries() {
        let mut memory = TableMemory([0; 4]);
        let table = unsafe {
            MsixTable::new(capability(), memory.0.as_mut_ptr().cast::<u8>(), 16).unwrap()
        };
        assert_eq!(table.read_entry(1), Err(MsixTableError::OutOfRange));
    }
}
