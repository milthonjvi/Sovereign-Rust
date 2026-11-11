//! x86_64 four-/five-level page-table construction for Sovereign Boot.
//!
//! This module builds tables but deliberately does not load CR3. The caller
//! must keep the active loader mappings, stack, firmware call path, and all
//! referenced buffers mapped before performing any address-space transition.

use crate::cpu::NxeEnabled;

const PAGE_SIZE: u64 = 4096;
const PRESENT: u64 = 1 << 0;
const WRITABLE: u64 = 1 << 1;
const USER: u64 = 1 << 2;
const HUGE_PAGE: u64 = 1 << 7;
const NO_EXECUTE: u64 = 1 << 63;
const ADDRESS_MASK: u64 = 0x000F_FFFF_FFFF_F000;
pub const RECURSIVE_SLOT: usize = 510;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PagePermissions {
    pub writable: bool,
    pub executable: bool,
    pub user_accessible: bool,
}

impl PagePermissions {
    pub const KERNEL_READ_ONLY: Self = Self { writable: false, executable: false, user_accessible: false };
    pub const KERNEL_READ_WRITE: Self = Self { writable: true, executable: false, user_accessible: false };
    pub const KERNEL_READ_EXECUTE: Self = Self { writable: false, executable: true, user_accessible: false };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageTableError {
    MisalignedAddress, NonCanonicalAddress, PhysicalAddressOutOfRange,
    AllocationFailed, InvalidTablePointer, AlreadyMapped, CorruptIntermediateEntry,
    WritableExecutableMapping,
    RecursiveSlotReserved,
}

/// Supplies unique, zeroed, page-aligned table pages and their writable mapping.
///
/// # Safety
/// Implementations must return unique physical pages. The virtual pointer must
/// alias that exact page, remain writable, and stay valid for the builder lifetime.
pub unsafe trait PageTableMemory {
    fn allocate_zeroed_table(&mut self) -> Result<(u64, *mut u64), PageTableError>;
    /// # Safety
    /// physical must identify a table page allocated by this provider.
    unsafe fn table_pointer(&mut self, physical: u64) -> Result<*mut u64, PageTableError>;
}

pub struct PageTableBuilder<M: PageTableMemory> {
    memory: M,
    root_physical: u64,
    root_virtual: *mut u64,
    paging_levels: u8,
}

impl<M: PageTableMemory> PageTableBuilder<M> {
    pub fn new(mut memory: M, _nxe: NxeEnabled) -> Result<Self, PageTableError> {
        let (physical, virtual_ptr) = memory.allocate_zeroed_table()?;
        validate_table_page(physical, virtual_ptr)?;
        let paging_levels = _nxe.paging_levels();
        if paging_levels != 4 && paging_levels != 5 {
            return Err(PageTableError::CorruptIntermediateEntry);
        }
        // The recursive window exposes page-table pages to the kernel without
        // assuming a physical direct map. NXE is proven active by NxeEnabled.
        unsafe {
            virtual_ptr.add(RECURSIVE_SLOT).write_volatile(
                (physical & ADDRESS_MASK) | PRESENT | WRITABLE | NO_EXECUTE,
            );
        }
        Ok(Self { memory, root_physical: physical, root_virtual: virtual_ptr, paging_levels })
    }

    pub const fn root_physical(&self) -> u64 { self.root_physical }
    pub const fn paging_levels(&self) -> u8 { self.paging_levels }

    /// Map one 4 KiB page. Existing leaf mappings are never overwritten.
    pub fn map_4k(&mut self, virtual_address: u64, physical_address: u64, permissions: PagePermissions) -> Result<(), PageTableError> {
        if virtual_address & (PAGE_SIZE - 1) != 0 || physical_address & (PAGE_SIZE - 1) != 0 {
            return Err(PageTableError::MisalignedAddress);
        }
        if !is_canonical_for_levels(virtual_address, self.paging_levels) { return Err(PageTableError::NonCanonicalAddress); }
        if permissions.writable && permissions.executable {
            return Err(PageTableError::WritableExecutableMapping);
        }
        if physical_address & !ADDRESS_MASK != 0 { return Err(PageTableError::PhysicalAddressOutOfRange); }
        let mut indices = [0usize; 5];
        for depth in 0..self.paging_levels as usize {
            let shift = 12 + 9 * (self.paging_levels as usize - depth - 1);
            indices[depth] = ((virtual_address >> shift) & 0x1ff) as usize;
        }
        if indices[0] == RECURSIVE_SLOT {
            return Err(PageTableError::RecursiveSlotReserved);
        }
        let mut table_virtual = self.root_virtual;
        for &index in &indices[..self.paging_levels as usize - 1] {
            // SAFETY: table_virtual always points to a provider-owned table page.
            let entry = unsafe { table_virtual.add(index).read_volatile() };
            if entry & PRESENT == 0 {
                let (next_physical, next_virtual) = self.memory.allocate_zeroed_table()?;
                validate_table_page(next_physical, next_virtual)?;
                let mut flags = PRESENT | WRITABLE;
                if permissions.user_accessible { flags |= USER; }
                // SAFETY: This entry belongs to the exclusively owned parent table.
                unsafe { table_virtual.add(index).write_volatile((next_physical & ADDRESS_MASK) | flags); }
                table_virtual = next_virtual;
            } else {
                if entry & HUGE_PAGE != 0 { return Err(PageTableError::CorruptIntermediateEntry); }
                if permissions.user_accessible && entry & USER == 0 {
                    // Promote ancestor permissions when adding a user mapping.
                    // SAFETY: This entry belongs to the exclusively owned table.
                    unsafe { table_virtual.add(index).write_volatile(entry | USER); }
                }
                let next_physical = entry & ADDRESS_MASK;
                if next_physical == 0 { return Err(PageTableError::CorruptIntermediateEntry); }
                // SAFETY: Provider resolves only table pages in its allocation domain.
                let next_virtual = unsafe { self.memory.table_pointer(next_physical)? };
                if next_virtual.is_null() { return Err(PageTableError::InvalidTablePointer); }
                table_virtual = next_virtual;
            }
        }
        let leaf_index = indices[self.paging_levels as usize - 1];
        // SAFETY: table_virtual is a validated page-table page.
        let leaf = unsafe { table_virtual.add(leaf_index).read_volatile() };
        if leaf & PRESENT != 0 { return Err(PageTableError::AlreadyMapped); }
        let mut flags = PRESENT;
        if permissions.writable { flags |= WRITABLE; }
        if permissions.user_accessible { flags |= USER; }
        if !permissions.executable { flags |= NO_EXECUTE; }
        // SAFETY: Leaf slot belongs to the exclusively owned page-table page.
        unsafe { table_virtual.add(leaf_index).write_volatile((physical_address & ADDRESS_MASK) | flags); }
        Ok(())
    }

    /// Transfer ownership of all allocated table pages to the caller.
    pub fn into_parts(self) -> (u64, M) { (self.root_physical, self.memory) }
}

fn validate_table_page(physical: u64, virtual_ptr: *mut u64) -> Result<(), PageTableError> {
    if physical == 0 || physical & (PAGE_SIZE - 1) != 0 { return Err(PageTableError::MisalignedAddress); }
    if physical & !ADDRESS_MASK != 0 { return Err(PageTableError::PhysicalAddressOutOfRange); }
    if virtual_ptr.is_null() || (virtual_ptr as usize) & (PAGE_SIZE as usize - 1) != 0 {
        return Err(PageTableError::InvalidTablePointer);
    }
    Ok(())
}

/// Canonical virtual-address check for four-level paging (48-bit VA).
pub const fn is_canonical(address: u64) -> bool {
    let upper = address >> 48;
    let sign = (address >> 47) & 1;
    (sign == 0 && upper == 0) || (sign == 1 && upper == 0xffff)
}

pub const fn pages_for_range(size: u64) -> Option<u64> {
    if size == 0 { return Some(0); }
    match size.checked_add(PAGE_SIZE - 1) { Some(rounded) => Some(rounded / PAGE_SIZE), None => None }
}

const fn is_canonical_for_levels(address: u64, levels: u8) -> bool {
    let sign_bit = if levels == 5 { 56 } else { 47 };
    let upper_shift = sign_bit + 1;
    let upper = address >> upper_shift;
    let sign = (address >> sign_bit) & 1;
    let upper_mask = u64::MAX >> upper_shift;
    (sign == 0 && upper == 0) || (sign == 1 && upper == upper_mask)
}
