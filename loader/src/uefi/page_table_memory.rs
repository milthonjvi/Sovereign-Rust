//! UEFI-backed physical page provider for the x86_64 page-table builder.
//!
//! UEFI identity-maps allocated pages while Boot Services are active, allowing
//! the loader to initialize table pages through their physical addresses.

use core::{ffi::c_void, ptr};

use crate::page_tables::{PageTableError, PageTableMemory};

use super::boot_services::{allocate_loader_pages, EfiBootServices};

/// Supplies page-table pages from UEFI AllocatePages.
///
/// # Safety
/// The pointer must refer to the live firmware Boot Services table. This
/// provider is valid only before ExitBootServices and while UEFI's current
/// identity mapping for allocated physical pages remains active.
pub struct UefiPageTableMemory {
    boot_services: *mut EfiBootServices,
}

impl UefiPageTableMemory {
    /// Create a provider bound to the live UEFI Boot Services table.
    ///
    /// # Safety
    /// `boot_services` must remain valid and callable for the provider's
    /// entire lifetime. No allocation may occur after ExitBootServices.
    pub unsafe fn new(boot_services: *mut c_void) -> Result<Self, PageTableError> {
        if boot_services.is_null() {
            return Err(PageTableError::InvalidTablePointer);
        }
        Ok(Self { boot_services: boot_services.cast::<EfiBootServices>() })
    }
}

// SAFETY: allocations are unique UEFI LoaderData pages, zeroed before return;
// the identity-mapped virtual pointer aliases the returned physical page.
unsafe impl PageTableMemory for UefiPageTableMemory {
    fn allocate_zeroed_table(&mut self) -> Result<(u64, *mut u64), PageTableError> {
        // SAFETY: Constructor contract guarantees a live Boot Services table.
        let boot = unsafe { self.boot_services.as_mut() }
            .ok_or(PageTableError::InvalidTablePointer)?;
        let mut physical = 0u64;
        // SAFETY: Boot Services is live and physical is a valid output slot.
        let status = unsafe { allocate_loader_pages(boot, 1, &mut physical) };
        if status != super::EFI_SUCCESS || physical == 0 {
            return Err(PageTableError::AllocationFailed);
        }
        let virtual_ptr = physical as usize as *mut u64;
        if (virtual_ptr as usize) & 0xfff != 0 {
            return Err(PageTableError::InvalidTablePointer);
        }
        // SAFETY: AllocatePages returned one writable 4 KiB page; UEFI keeps
        // the page identity-mapped while Boot Services are active.
        unsafe { ptr::write_bytes(virtual_ptr, 0, 4096 / core::mem::size_of::<u64>()) };
        Ok((physical, virtual_ptr))
    }

    unsafe fn table_pointer(&mut self, physical: u64) -> Result<*mut u64, PageTableError> {
        if physical == 0 || physical & 0xfff != 0 || physical & !0x000F_FFFF_FFFF_F000 != 0 {
            return Err(PageTableError::PhysicalAddressOutOfRange);
        }
        // SAFETY: Builder only passes pages allocated by this provider;
        // UEFI identity mapping makes the physical address writable.
        Ok(physical as usize as *mut u64)
    }
}
