//! Bounded UEFI memory-map buffer management.
//!
//! Allocate the buffer before final page-table allocations. Capture the map
//! only after all other allocations are complete; GetMemoryMap itself does not
//! allocate, so its returned key can be used for the immediate exit attempt.

use core::{ffi::c_void, mem};

use super::{
    boot_services::{allocate_loader_pages, EfiBootServices, EfiMemoryDescriptor},
    EfiStatus, EFI_BUFFER_TOO_SMALL, EFI_COMPROMISED_DATA, EFI_SUCCESS, EFI_UNSUPPORTED,
};

pub const DEFAULT_MEMORY_MAP_PAGES: usize = 64;

#[derive(Clone, Copy, Debug)]
pub struct CapturedMemoryMap {
    pub physical_base: u64,
    pub capacity_bytes: usize,
    pub size_bytes: usize,
    pub descriptor_size: usize,
    pub descriptor_version: u32,
    pub map_key: usize,
    pub pages: usize,
}

/// Owns a page-backed memory-map buffer allocated as EFI_LOADER_DATA.
pub struct EfiMemoryMapBuffer {
    boot_services: *mut EfiBootServices,
    physical_base: u64,
    pages: usize,
}

impl EfiMemoryMapBuffer {
    /// Reserve the buffer before constructing final tables or collecting the
    /// map. The physical buffer is accessed through the current UEFI mapping.
    ///
    /// # Safety
    /// `boot_services` must point to the live firmware table and remain valid
    /// until this buffer has been populated and Boot Services are exited.
    pub unsafe fn allocate(boot_services: *mut c_void, pages: usize) -> Result<Self, EfiStatus> {
        if boot_services.is_null() || pages == 0 {
            return Err(super::EFI_INVALID_PARAMETER);
        }
        // SAFETY: Caller guarantees a live Boot Services table.
        let boot = unsafe { &mut *boot_services.cast::<EfiBootServices>() };
        let mut physical_base = 0u64;
        // SAFETY: The output slot and live firmware table are valid.
        let status = unsafe { allocate_loader_pages(boot, pages, &mut physical_base) };
        if status != EFI_SUCCESS { return Err(status); }
        if physical_base == 0 || physical_base & 0xfff != 0 {
            return Err(EFI_COMPROMISED_DATA);
        }
        Ok(Self { boot_services: boot_services.cast(), physical_base, pages })
    }

    pub const fn physical_base(&self) -> u64 { self.physical_base }
    pub const fn pages(&self) -> usize { self.pages }
    pub const fn capacity_bytes(&self) -> usize { self.pages * 4096 }

    /// Capture the current firmware map without allocating additional memory.
    pub fn capture(&mut self) -> Result<CapturedMemoryMap, EfiStatus> {
        // SAFETY: Allocation contract guarantees this remains a live table.
        let boot = unsafe { self.boot_services.as_mut() }.ok_or(EFI_UNSUPPORTED)?;
        let get_memory_map = boot.get_memory_map.ok_or(EFI_UNSUPPORTED)?;
        let capacity = self.capacity_bytes();
        let mut size = capacity;
        let mut map_key = 0usize;
        let mut descriptor_size = 0usize;
        let mut descriptor_version = 0u32;
        let buffer = self.physical_base as usize as *mut EfiMemoryDescriptor;
        // SAFETY: The reserved buffer is writable and capacity matches its size.
        let status = unsafe {
            get_memory_map(&mut size, buffer, &mut map_key, &mut descriptor_size, &mut descriptor_version)
        };
        if status != EFI_SUCCESS {
            return Err(if status == EFI_BUFFER_TOO_SMALL { EFI_BUFFER_TOO_SMALL } else { status });
        }
        if descriptor_size < mem::size_of::<EfiMemoryDescriptor>()
            || descriptor_size == 0 || size > capacity || size % descriptor_size != 0
            || descriptor_version == 0
        {
            return Err(EFI_COMPROMISED_DATA);
        }
        Ok(CapturedMemoryMap {
            physical_base: self.physical_base, capacity_bytes: capacity, size_bytes: size,
            descriptor_size, descriptor_version, map_key, pages: self.pages,
        })
    }

    /// Return the buffer as an untyped pointer for byte-wise ABI consumers.
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.physical_base as usize as *mut u8
    }
}
