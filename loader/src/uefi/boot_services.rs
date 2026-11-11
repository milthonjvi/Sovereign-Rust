use super::{EfiHandle, EfiStatus, EfiTableHeader};

/// UEFI AllocatePages allocation policy.
pub const EFI_ALLOCATE_ANY_PAGES: u32 = 0;

/// UEFI memory type reserved for allocations owned by a boot application.
pub const EFI_LOADER_DATA: u32 = 2;

/// UEFI memory descriptor returned by GetMemoryMap.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct EfiMemoryDescriptor {
    pub memory_type: u32,
    _padding: u32,
    pub physical_start: u64,
    pub virtual_start: u64,
    pub number_of_pages: u64,
    pub attribute: u64,
}

/// UEFI Boot Services prefix through ExitBootServices.
#[repr(C)]
pub struct EfiBootServices {
    pub header: EfiTableHeader,
    _raise_tpl: usize,
    _restore_tpl: usize,
    pub allocate_pages: Option<unsafe extern "efiapi" fn(
        allocation_type: u32,
        memory_type: u32,
        pages: usize,
        memory: *mut u64,
    ) -> EfiStatus>,
    pub free_pages: Option<unsafe extern "efiapi" fn(
        memory: u64,
        pages: usize,
    ) -> EfiStatus>,
    pub get_memory_map: Option<unsafe extern "efiapi" fn(
        memory_map_size: *mut usize,
        memory_map: *mut EfiMemoryDescriptor,
        map_key: *mut usize,
        descriptor_size: *mut usize,
        descriptor_version: *mut u32,
    ) -> EfiStatus>,
    _allocate_pool: usize,
    _free_pool: usize,
    _create_event: usize,
    _set_timer: usize,
    _wait_for_event: usize,
    _signal_event: usize,
    _close_event: usize,
    _check_event: usize,
    _install_protocol_interface: usize,
    _reinstall_protocol_interface: usize,
    _uninstall_protocol_interface: usize,
    pub handle_protocol: Option<unsafe extern "efiapi" fn(
        handle: EfiHandle,
        protocol: *const super::EfiGuid,
        interface: *mut *mut core::ffi::c_void,
    ) -> EfiStatus>,
    _reserved: usize,
    _register_protocol_notify: usize,
    _locate_handle: usize,
    _locate_device_path: usize,
    _install_configuration_table: usize,
    _load_image: usize,
    _start_image: usize,
    _exit: usize,
    _unload_image: usize,
    pub exit_boot_services: Option<unsafe extern "efiapi" fn(
        image_handle: EfiHandle,
        map_key: usize,
    ) -> EfiStatus>,
}

/// Allocate page-aligned memory owned by the UEFI loader.
///
/// # Safety
/// The table must be the live firmware Boot Services table and address must
/// be writable. The allocation must be released or transferred before exit.
pub unsafe fn allocate_loader_pages(
    boot: &mut EfiBootServices,
    pages: usize,
    address: &mut u64,
) -> EfiStatus {
    if pages == 0 {
        return super::EFI_INVALID_PARAMETER;
    }
    let Some(allocate_pages) = boot.allocate_pages else {
        return super::EFI_UNSUPPORTED;
    };

    // SAFETY: The caller guarantees a live Boot Services table and output slot.
    unsafe {
        allocate_pages(
            EFI_ALLOCATE_ANY_PAGES,
            EFI_LOADER_DATA,
            pages,
            address,
        )
    }
}

/// Release a page allocation previously returned by AllocatePages.
///
/// # Safety
/// The table must be live, and address/pages must describe an outstanding
/// allocation owned by this loader and not yet transferred to the kernel.
pub unsafe fn free_loader_pages(
    boot: &mut EfiBootServices,
    address: u64,
    pages: usize,
) -> EfiStatus {
    if address == 0 || pages == 0 {
        return super::EFI_INVALID_PARAMETER;
    }
    let Some(free_pages) = boot.free_pages else {
        return super::EFI_UNSUPPORTED;
    };

    // SAFETY: The caller guarantees the allocation is live and owned by loader.
    unsafe { free_pages(address, pages) }
}

// UEFI descriptor prefix is 40 bytes; firmware may provide a larger stride.
const _: [(); 40] = [(); core::mem::size_of::<EfiMemoryDescriptor>()];
