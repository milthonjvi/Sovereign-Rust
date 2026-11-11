#![no_std]
#![no_main]

pub mod cpu;
pub mod boot_info;
pub mod page_tables;
mod uefi;

use core::{mem, panic::PanicInfo};
use crate::page_tables::{PagePermissions, PageTableBuilder};
use uefi::{
    load_kernel_file, Console, EfiHandle, EfiStatus, EfiSystemTable, UefiPageTableMemory,
    EFI_COMPROMISED_DATA, EFI_INVALID_PARAMETER, EFI_SUCCESS,
    EFI_SYSTEM_TABLE_SIGNATURE,
};

/// UEFI application entry point.
///
/// # Safety
/// UEFI must invoke this function using the x86_64 UEFI ABI and provide a
/// valid image handle and system table for the duration of the call.
#[unsafe(no_mangle)]
pub extern "efiapi" fn efi_main(
    image_handle: EfiHandle,
    system_table: *mut EfiSystemTable,
) -> EfiStatus {
    if system_table.is_null() || image_handle.is_null() {
        return EFI_INVALID_PARAMETER;
    }

    // SAFETY: UEFI guarantees this pointer for the duration of the invocation.
    let system_table = unsafe { &mut *system_table };

    if system_table.header.signature != EFI_SYSTEM_TABLE_SIGNATURE
        || (system_table.header.header_size as usize) < mem::size_of::<EfiSystemTable>()
    {
        return EFI_COMPROMISED_DATA;
    }

    if system_table.con_out.is_null() {
        return EFI_INVALID_PARAMETER;
    }

    // SAFETY: The firmware console remains valid while Boot Services are active.
    let mut console = unsafe { Console::from_ptr(system_table.con_out) };
    if !console.write_ascii("Sovereign Boot [UEFI x86_64]\r\n")
        || !console.write_ascii("Locating boot volume...\r\n")
    {
        return EFI_INVALID_PARAMETER;
    }

    let loaded_kernel = match load_kernel_file(image_handle, system_table) {
        Ok(kernel) => kernel,
        Err(status) => {
            let _ = console.write_ascii("Kernel ELF loading failed.\r\n");
            return status;
        }
    };

    if !console.write_ascii("Kernel PT_LOAD segments copied to reserved physical memory.\r\n")
        || !console.write_ascii("Kernel virtual mappings and handoff are still pending.\r\n")
    {
        return EFI_INVALID_PARAMETER;
    }

    // NX must be enabled before any PTE with the NX bit is installed.
    // SAFETY: UEFI x86_64 applications execute at CPL 0; CPUID/MSR support is
    // checked by enable_nxe before the EFER bit is changed.
    let nxe = match unsafe { cpu::enable_nxe() } {
        Ok(token) => token,
        Err(_) => return EFI_UNSUPPORTED,
    };

    // Build the kernel's high-half mappings without switching CR3 yet.
    // Firmware identity mappings remain active while the page-table pages
    // are allocated and populated.
    // SAFETY: Boot Services are live until this entry point returns.
    let table_memory = match unsafe { UefiPageTableMemory::new(system_table.boot_services) } {
        Ok(memory) => memory,
        Err(_) => return EFI_UNSUPPORTED,
    };
    let mut page_tables = match PageTableBuilder::new(table_memory, nxe) {
        Ok(builder) => builder,
        Err(_) => return EFI_UNSUPPORTED,
    };

    for segment in loaded_kernel.segments.iter().take(loaded_kernel.segment_count) {
        let segment_end = match segment.virtual_address.checked_add(segment.memory_size) {
            Some(end) => end,
            None => return EFI_COMPROMISED_DATA,
        };
        let mut virtual_page = segment.virtual_address & !0xfff;
        let rounded_end = match segment_end.checked_add(0xfff) {
            Some(end) => end & !0xfff,
            None => return EFI_COMPROMISED_DATA,
        };
        let permissions = PagePermissions {
            writable: segment.flags & 2 != 0,
            executable: segment.flags & 1 != 0,
            user_accessible: false,
        };
        while virtual_page < rounded_end {
            let physical_page = match loaded_kernel.physical_base.checked_add(
                virtual_page - loaded_kernel.virtual_base,
            ) {
                Some(address) => address,
                None => return EFI_COMPROMISED_DATA,
            };
            if page_tables.map_4k(virtual_page, physical_page, permissions).is_err() {
                return EFI_COMPROMISED_DATA;
            }
            virtual_page += 0x1000;
        }
    }

    // Root is retained for the later CR3 transition. Loader/stack/boot-info
    // identity mappings must be added before that transition is permitted.
    let _kernel_page_table_root = page_tables.root_physical();
    let _loaded_kernel = loaded_kernel;
    if !console.write_ascii("Kernel image loaded and high-half page tables constructed.\\r\\n")
        || !console.write_ascii("CR3 transition and kernel handoff remain pending.\\r\\n")
    {
        return EFI_INVALID_PARAMETER;
    }
    EFI_SUCCESS
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
