#![no_std]
#![no_main]

pub mod cpu;
pub mod boot_info;
pub mod page_tables;
mod uefi;

use core::{mem, panic::PanicInfo};
use crate::boot_info::{SovereignBootInfo, BOOT_INFO_MAGIC, BOOT_INFO_VERSION, MEMORY_MAP_FORMAT_UEFI};
use crate::page_tables::{PagePermissions, PageTableBuilder, PageTableError, PageTableMemory};
use uefi::{
    allocate_loader_pages, load_kernel_file, Console, EfiBootServices, EfiHandle, EfiMemoryMapBuffer,
    EfiStatus, EfiSystemTable, UefiPageTableMemory, DEFAULT_MEMORY_MAP_PAGES,
    EFI_COMPROMISED_DATA, EFI_INVALID_PARAMETER, EFI_OUT_OF_RESOURCES, EFI_SUCCESS, EFI_UNSUPPORTED,
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

    if !console.write_ascii("Kernel PT_LOAD segments copied to reserved physical memory.\r\n") {
        return EFI_INVALID_PARAMETER;
    }

    // Reserve handoff and memory-map storage before building page tables.
    // The map is captured only after every remaining loader allocation.
    // SAFETY: The system table provides the live Boot Services pointer.
    let mut memory_map_buffer = match unsafe {
        EfiMemoryMapBuffer::allocate(system_table.boot_services, DEFAULT_MEMORY_MAP_PAGES)
    } {
        Ok(buffer) => buffer,
        Err(status) => return status,
    };
    let mut boot_info_physical = 0u64;
    // SAFETY: Boot Services remain active and the output slot is writable.
    let allocation_status = unsafe {
        allocate_loader_pages(
            &mut *(system_table.boot_services as *mut EfiBootServices),
            1,
            &mut boot_info_physical,
        )
    };
    if allocation_status != EFI_SUCCESS {
        return allocation_status;
    }
    if boot_info_physical == 0 {
        return EFI_COMPROMISED_DATA;
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
        Err(PageTableError::AllocationFailed) => return EFI_OUT_OF_RESOURCES,
        Err(_) => return EFI_COMPROMISED_DATA,
    };

    // Map the loader image RX and its current stack RW/NX so a later
    // post-ExitBootServices CR3 switch can execute a minimal transition stub.
    if let Err(error) = map_identity_range(
        &mut page_tables,
        loaded_kernel.loader_image_base,
        loaded_kernel.loader_image_size,
        PagePermissions::KERNEL_READ_EXECUTE,
    ) {
        return page_table_error_status(error);
    }
    let stack_pointer = current_stack_pointer();
    let stack_start = match stack_pointer.checked_sub(256 * 1024) {
        Some(value) => value & !0xfff,
        None => return EFI_COMPROMISED_DATA,
    };
    let stack_end = match stack_pointer.checked_add(64 * 1024).and_then(align_page_up) {
        Some(value) => value,
        None => return EFI_COMPROMISED_DATA,
    };
    if let Err(error) = map_identity_range(
        &mut page_tables,
        stack_start,
        stack_end - stack_start,
        PagePermissions::KERNEL_READ_WRITE,
    ) {
        return page_table_error_status(error);
    }

    // The memory-map buffer is writable until capture and then read-only to
    // the kernel. Boot info is populated below, so it is mapped read-only too.
    if let Err(error) = map_identity_range(
        &mut page_tables,
        memory_map_buffer.physical_base(),
        (memory_map_buffer.pages() as u64) * 4096,
        PagePermissions::KERNEL_READ_ONLY,
    ) {
        return page_table_error_status(error);
    }
    if let Err(error) = map_identity_range(
        &mut page_tables,
        boot_info_physical,
        4096,
        PagePermissions::KERNEL_READ_ONLY,
    ) {
        return page_table_error_status(error);
    }

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
            if let Err(error) = page_tables.map_4k(virtual_page, physical_page, permissions) {
                return page_table_error_status(error);
            }
            virtual_page += 0x1000;
        }
    }

    // Capture only after all loader allocations and page-table construction.
    // No allocation is performed by GetMemoryMap itself.
    let captured_map = match memory_map_buffer.capture() {
        Ok(map) => map,
        Err(status) => return status,
    };
    let boot_info = SovereignBootInfo {
        magic: BOOT_INFO_MAGIC,
        version: BOOT_INFO_VERSION,
        structure_size: mem::size_of::<SovereignBootInfo>() as u32,
        flags: 0,
        memory_map_physical: captured_map.physical_base,
        memory_map_size: captured_map.size_bytes as u64,
        memory_descriptor_size: captured_map.descriptor_size as u64,
        memory_descriptor_version: captured_map.descriptor_version,
        memory_map_format: MEMORY_MAP_FORMAT_UEFI,
        kernel_physical_base: loaded_kernel.physical_base,
        kernel_virtual_base: loaded_kernel.virtual_base,
        kernel_image_size: loaded_kernel.image_size,
        kernel_entry: loaded_kernel.entry,
        acpi_rsdp_physical: 0,
        framebuffer_physical: 0,
        framebuffer_size: 0,
        framebuffer_width: 0,
        framebuffer_height: 0,
        framebuffer_pixels_per_scanline: 0,
        framebuffer_pixel_format: 0,
        reserved: [0; 4],
    };
    if !boot_info.is_well_formed() {
        return EFI_COMPROMISED_DATA;
    }
    let boot_info_pointer = boot_info_physical as usize as *mut SovereignBootInfo;
    // SAFETY: The loader reserved one writable identity-mapped page for this ABI.
    unsafe { boot_info_pointer.write(boot_info) };

    // These values are retained for the upcoming ExitBootServices + CR3 stage.
    let _kernel_page_table_root = page_tables.root_physical();
    let _boot_info_physical = boot_info_physical;
    let _map_key = captured_map.map_key;
    let _loaded_kernel = loaded_kernel;
    if !console.write_ascii("ELF segments loaded; kernel page tables and boot-info ABI prepared.\r\n")
        || !console.write_ascii("ExitBootServices and CR3 transition remain pending.\r\n")
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


fn current_stack_pointer() -> u64 {
    let stack_pointer: u64;
    // SAFETY: Reading RSP has no side effects and is valid in x86_64 UEFI mode.
    unsafe {
        core::arch::asm!(
            "mov {}, rsp",
            out(reg) stack_pointer,
            options(nomem, nostack, preserves_flags)
        );
    }
    stack_pointer
}

fn align_page_up(value: u64) -> Option<u64> {
    value.checked_add(0xfff).map(|rounded| rounded & !0xfff)
}

fn map_identity_range<M: PageTableMemory>(
    tables: &mut PageTableBuilder<M>,
    base: u64,
    size: u64,
    permissions: PagePermissions,
) -> Result<(), PageTableError> {
    if size == 0 {
        return Err(PageTableError::MisalignedAddress);
    }
    let end = base.checked_add(size).ok_or(PageTableError::PhysicalAddressOutOfRange)?;
    let mut page = base & !0xfff;
    let rounded_end = align_page_up(end).ok_or(PageTableError::PhysicalAddressOutOfRange)?;
    while page < rounded_end {
        tables.map_4k(page, page, permissions)?;
        page = page.checked_add(0x1000).ok_or(PageTableError::PhysicalAddressOutOfRange)?;
    }
    Ok(())
}

fn page_table_error_status(error: PageTableError) -> EfiStatus {
    match error {
        PageTableError::AllocationFailed => EFI_OUT_OF_RESOURCES,
        _ => EFI_COMPROMISED_DATA,
    }
}
