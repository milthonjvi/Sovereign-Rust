//! x86_64 four-level page-table construction for Sovereign Boot.
//!
//! This module builds tables but deliberately does not load CR3. The caller
//! must keep the active loader mappings, stack, firmware call path, and all
//! referenced buffers mapped before performing any address-space transition.

const PAGE_SIZE: u64 = 4096;
const PRESENT: u64 = 1 << 0;
const WRITABLE: u64 = 1 << 1;
const USER: u64 = 1 << 2;
const HUGE_PAGE: u64 = 1 << 7;
const NO_EXECUTE: u64 = 1 << 63;
const ADDRESS_MASK: u64 = 0x000F_FFFF_FFFF_F000;

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
    AllocationFailed, InvalidTablePointer, AlreadyMapped, CorruptIntermediateEntry,\n    NxeNotEnabled, WritableExecutableAlias,
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
}

impl<M: PageTableMemory> PageTableBuilder<M> {
    pub fn new(mut memory: M, nxe_enabled: bool) -> Result<Self, PageTableError> {\n        if !nxe_enabled { return Err(PageTableError::NxeNotEnabled); }
        let (physical, virtual_ptr) = memory.allocate_zeroed_table()?;
        validate_table_page(physical, virtual_ptr)?;
        Ok(Self { memory, root_physical: physical, root_virtual: virtual_ptr })
    }

    pub const fn root_physical(&self) -> u64 { self.root_physical }

    /// Map one 4 KiB page. Existing leaf mappings are never overwritten.
    pub fn map_4k(&mut self, virtual_address: u64, physical_address: u64, permissions: PagePermissions) -> Result<(), PageTableError> {
        if virtual_address & (PAGE_SIZE - 1) != 0 || physical_address & (PAGE_SIZE - 1) != 0 {
            return Err(PageTableError::MisalignedAddress);
        }
        if !is_canonical(virtual_address) { return Err(PageTableError::NonCanonicalAddress); }\n        if permissions.writable && permissions.executable {\n            return Err(PageTableError::WritableExecutableAlias);\n        }
        if physical_address & !ADDRESS_MASK != 0 { return Err(PageTableError::PhysicalAddressOutOfRange); }
        let indices = [
            ((virtual_address >> 39) & 0x1ff) as usize,
            ((virtual_address >> 30) & 0x1ff) as usize,
            ((virtual_address >> 21) & 0x1ff) as usize,
            ((virtual_address >> 12) & 0x1ff) as usize,
        ];
        let mut table_virtual = self.root_virtual;
        for &index in &indices[..3] {
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
        let leaf_index = indices[3];
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
