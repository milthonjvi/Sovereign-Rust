#![no_std]
#![no_main]

pub mod cpu;
pub mod boot_info;
pub mod page_tables;
mod uefi;

use core::{mem, panic::PanicInfo, ptr};
use crate::boot_info::{SovereignBootInfo, BOOT_INFO_MAGIC, BOOT_INFO_VERSION, MEMORY_MAP_FORMAT_UEFI};
use crate::page_tables::{PagePermissions, PageTableBuilder, PageTableError, PageTableMemory};
use uefi::{
    allocate_loader_pages, inspect_loaded_image, load_kernel_file, Console, EfiBootServices, EfiHandle, EfiMemoryMapBuffer,
    EfiStatus, EfiSystemTable, EfiConfigurationTable, UefiPageTableMemory, DEFAULT_MEMORY_MAP_PAGES,
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

    if loaded_kernel.pages as u64 != loaded_kernel.image_size / 4096
        || loaded_kernel.image_size & 0xfff != 0
    {
        return EFI_COMPROMISED_DATA;
    }

    if !console.write_ascii("Kernel PT_LOAD segments copied to reserved physical memory.\r\n") {
        return EFI_INVALID_PARAMETER;
    }

    // UEFI configuration tables are the authoritative source for the ACPI
    // RSDP. Prefer ACPI 2.0+ and fall back to the legacy ACPI 1.0 GUID.
    // SAFETY: UEFI guarantees the configuration table array remains readable
    // until ExitBootServices; the selected RSDP is validated before retention.
    let acpi_rsdp = unsafe { find_acpi_rsdp(system_table) };

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

    // Reserve a dedicated 256 KiB kernel bootstrap stack. The transition
    // never relies on the firmware-owned stack after CR3 is loaded.
    const KERNEL_STACK_PAGES: usize = 64;
    const KERNEL_STACK_GUARD_PAGES: u64 = 1;
    const KERNEL_STACK_ALLOCATION_PAGES: usize =
        KERNEL_STACK_PAGES + (KERNEL_STACK_GUARD_PAGES as usize * 2);
    let mut kernel_stack_physical = 0u64;
    // SAFETY: Boot Services remain active and the output slot is writable.
    let stack_status = unsafe {
        allocate_loader_pages(
            &mut *(system_table.boot_services as *mut EfiBootServices),
            KERNEL_STACK_ALLOCATION_PAGES,
            &mut kernel_stack_physical,
        )
    };
    if stack_status != EFI_SUCCESS {
        return stack_status;
    }
    if kernel_stack_physical == 0 || kernel_stack_physical & 0xfff != 0 {
        return EFI_COMPROMISED_DATA;
    }
    // SAFETY: AllocatePages returned KERNEL_STACK_PAGES writable pages.
    unsafe {
        ptr::write_bytes(
            kernel_stack_physical as usize as *mut u8,
            0,
            KERNEL_STACK_ALLOCATION_PAGES * 4096,
        );
    }

    // Reserve an emergency IDT page whose handlers remain mapped across CR3.
    let mut emergency_idt_physical = 0u64;
    // SAFETY: Boot Services remain active and the output slot is writable.
    let idt_status = unsafe {
        allocate_loader_pages(
            &mut *(system_table.boot_services as *mut EfiBootServices),
            1,
            &mut emergency_idt_physical,
        )
    };
    if idt_status != EFI_SUCCESS {
        return idt_status;
    }
    if emergency_idt_physical == 0 || emergency_idt_physical & 0xfff != 0 {
        return EFI_COMPROMISED_DATA;
    }
    // SAFETY: AllocatePages returned one writable page for 256 IDT gates.
    unsafe { build_emergency_idt(emergency_idt_physical) };

    // Snapshot the map before page-table allocations. ACPI reclaimable and
    // NVS ranges are stable across those later LoaderData allocations.
    let initial_map = match memory_map_buffer.capture() {
        Ok(map) => map,
        Err(status) => return status,
    };

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

    // Inspect the loaded PE image and preserve section-level W^X permissions.
    // Headers are read-only/NX; executable sections are RX; data sections RW/NX.
    // SAFETY: LoadedImageProtocol supplied this resident image range.
    let loader_layout = match unsafe {
        inspect_loaded_image(loaded_kernel.loader_image_base, loaded_kernel.loader_image_size)
    } {
        Ok(layout) => layout,
        Err(_) => return EFI_COMPROMISED_DATA,
    };
    let firmware_entry = efi_main as *const () as usize as u64;
    let expected_firmware_entry = match loaded_kernel.loader_image_base.checked_add(
        loader_layout.entry_point_rva,
    ) {
        Some(address) => address,
        None => return EFI_COMPROMISED_DATA,
    };
    if loader_layout.image_size > loaded_kernel.loader_image_size
        || expected_firmware_entry != firmware_entry
    {
        return EFI_COMPROMISED_DATA;
    }

    if let Err(error) = map_identity_range(
        &mut page_tables,
        loaded_kernel.loader_image_base,
        loader_layout.headers_size,
        PagePermissions::KERNEL_READ_ONLY,
    ) {
        return page_table_error_status(error);
    }
    for section in loader_layout.sections.iter().take(loader_layout.section_count) {
        let section_base = match loaded_kernel.loader_image_base.checked_add(section.relative_address) {
            Some(address) => address,
            None => return EFI_COMPROMISED_DATA,
        };
        if let Err(error) = map_identity_range(
            &mut page_tables,
            section_base,
            section.size,
            section.permissions,
        ) {
            return page_table_error_status(error);
        }
    }

    // Preserve the active firmware GDT for exception-gate selector resolution.
    let (gdt_base, gdt_limit) = current_gdt();
    if gdt_base == 0 || gdt_limit < 0x0f {
        return EFI_COMPROMISED_DATA;
    }
    if let Err(error) = map_identity_range(
        &mut page_tables,
        gdt_base,
        (gdt_limit as u64) + 1,
        PagePermissions::KERNEL_READ_ONLY,
    ) {
        return page_table_error_status(error);
    }
    if let Err(error) = map_identity_range(
        &mut page_tables,
        emergency_idt_physical,
        4096,
        PagePermissions::KERNEL_READ_ONLY,
    ) {
        return page_table_error_status(error);
    }

    // Map firmware ACPI memory so the kernel can safely follow RSDP ->
    // XSDT/RSDT -> FADT/DSDT/MADT/HPET and the NVS FACS. ACPI reclaimable
    // tables are RO/NX; ACPI NVS remains RW/NX for the FACS global lock.
    let descriptor_count = initial_map.size_bytes / initial_map.descriptor_size;
    const MAX_ACPI_MAPPING_SIZE: u64 = 256 * 1024 * 1024;
    let mut mapped_acpi_bytes = 0u64;
    let mut rsdp_in_acpi_memory = false;
    let mut rsdp_in_reserved_memory = false;
    let rsdp_range = match acpi_rsdp {
        Some((base, length)) => Some((
            base,
            match base.checked_add(length) {
                Some(end) => end,
                None => return EFI_COMPROMISED_DATA,
            },
        )),
        None => None,
    };
    for index in 0..descriptor_count {
        if acpi_rsdp.is_none() {
            break;
        }
        let descriptor = match memory_map_buffer.descriptor(initial_map, index) {
            Ok(descriptor) => descriptor,
            Err(status) => return status,
        };
        let descriptor_size = match descriptor.number_of_pages.checked_mul(4096) {
            Some(size) if size != 0 => size,
            _ => return EFI_COMPROMISED_DATA,
        };
        let descriptor_end = match descriptor.physical_start.checked_add(descriptor_size) {
            Some(end) => end,
            None => return EFI_COMPROMISED_DATA,
        };
        if let Some((rsdp_start, rsdp_end)) = rsdp_range {
            if rsdp_start >= descriptor.physical_start && rsdp_end <= descriptor_end {
                if descriptor.memory_type == 9 || descriptor.memory_type == 10 {
                    rsdp_in_acpi_memory = true;
                } else if descriptor.memory_type == 0 {
                    rsdp_in_reserved_memory = true;
                }
            }
        }
        if descriptor.memory_type != 9 && descriptor.memory_type != 10 {
            continue;
        }
        mapped_acpi_bytes = match mapped_acpi_bytes.checked_add(descriptor_size) {
            Some(total) if total <= MAX_ACPI_MAPPING_SIZE => total,
            _ => return EFI_OUT_OF_RESOURCES,
        };
        let mut page = descriptor.physical_start;
        while page < descriptor_end {
            let page_end = match page.checked_add(4096) {
                Some(end) => end,
                None => return EFI_COMPROMISED_DATA,
            };
            let permissions = if descriptor.memory_type == 9 {
                PagePermissions::KERNEL_READ_ONLY
            } else {
                PagePermissions::KERNEL_READ_WRITE
            };
            if let Err(error) = page_tables.map_4k(page, page, permissions) {
                return page_table_error_status(error);
            }
            page = page_end;
        }
    }
    if let Some((rsdp_physical, rsdp_length)) = acpi_rsdp {
        if !rsdp_in_acpi_memory && !rsdp_in_reserved_memory {
            return EFI_COMPROMISED_DATA;
        }
        if rsdp_in_reserved_memory {
            if let Err(error) = map_identity_range(
                &mut page_tables,
                rsdp_physical,
                rsdp_length,
                PagePermissions::KERNEL_READ_ONLY,
            ) {
                return page_table_error_status(error);
            }
        }
    }

    // Leave one unmapped guard page on both sides of the bootstrap stack.
    // This turns stack underflow/overflow into a page fault instead of silently
    // corrupting adjacent loader-owned state.
    let kernel_stack_base = match kernel_stack_physical.checked_add(4096) {
        Some(base) => base,
        None => return EFI_COMPROMISED_DATA,
    };
    let kernel_stack_size = (KERNEL_STACK_PAGES as u64) * 4096;
    if let Err(error) = map_identity_range(
        &mut page_tables,
        kernel_stack_base,
        kernel_stack_size,
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
    if let Some((rsdp_physical, rsdp_length)) = acpi_rsdp {
        if let Err(error) = map_identity_range(
            &mut page_tables,
            rsdp_physical,
            rsdp_length,
            PagePermissions::KERNEL_READ_ONLY,
        ) {
            return page_table_error_status(error);
        }
    }

    for segment in loaded_kernel.segments.iter().take(loaded_kernel.segment_count) {
        if segment.file_size > segment.memory_size {
            return EFI_COMPROMISED_DATA;
        }
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
    let mut boot_info = SovereignBootInfo {
        magic: BOOT_INFO_MAGIC,
        version: BOOT_INFO_VERSION,
        structure_size: mem::size_of::<SovereignBootInfo>() as u32,
        flags: if acpi_rsdp.is_some() { crate::boot_info::BOOT_INFO_FLAG_ACPI_VALID } else { 0 },
        memory_map_physical: captured_map.physical_base,
        memory_map_size: captured_map.size_bytes as u64,
        memory_descriptor_size: captured_map.descriptor_size as u64,
        memory_descriptor_version: captured_map.descriptor_version,
        memory_map_format: MEMORY_MAP_FORMAT_UEFI,
        kernel_physical_base: loaded_kernel.physical_base,
        kernel_virtual_base: loaded_kernel.virtual_base,
        kernel_image_size: loaded_kernel.image_size,
        kernel_entry: loaded_kernel.entry,
        acpi_rsdp_physical: acpi_rsdp.map(|(physical, _)| physical).unwrap_or(0),
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

    if !console.write_ascii("Kernel mappings and boot-info prepared; exiting UEFI Boot Services.\r\n") {
        return EFI_INVALID_PARAMETER;
    }

    let kernel_page_table_root = page_tables.root_physical();
    let kernel_stack_top_offset = (KERNEL_STACK_PAGES as u64 + KERNEL_STACK_GUARD_PAGES) * 4096;
    let kernel_stack_top = match kernel_stack_physical.checked_add(kernel_stack_top_offset) {
        Some(top) => top,
        None => return EFI_COMPROMISED_DATA,
    };

    // Refresh the memory map and update the already-allocated boot-info page
    // immediately before each ExitBootServices attempt. The callback performs
    // only bounded memory writes: no firmware calls and no allocations.
    let mut handoff_valid = true;
    let expected_map_pages = memory_map_buffer.pages();
    let expected_map_capacity = memory_map_buffer.capacity_bytes();
    // SAFETY: The image handle and Boot Services table remain valid. The
    // callback only mutates the preallocated boot-info page.
    let final_map = unsafe {
        memory_map_buffer.exit_boot_services_with(image_handle, |map| {
            handoff_valid = map.pages == expected_map_pages
                && map.capacity_bytes == expected_map_capacity
                && map.size_bytes <= map.capacity_bytes;
            boot_info.memory_map_physical = map.physical_base;
            boot_info.memory_map_size = map.size_bytes as u64;
            boot_info.memory_descriptor_size = map.descriptor_size as u64;
            boot_info.memory_descriptor_version = map.descriptor_version;
            handoff_valid &= boot_info.is_well_formed();
            if handoff_valid {
                // SAFETY: This page was allocated and mapped before the final map.
                boot_info_pointer.write(boot_info);
            }
        })
    };
    if final_map.is_err() || !handoff_valid {
        loader_halt();
    }

    // No firmware service may be called after ExitBootServices succeeds.
    // The new root maps the loader transition code, dedicated kernel stack,
    // boot-info page, memory map, and all kernel PT_LOAD segments.
    transition_to_kernel(
        kernel_page_table_root,
        loaded_kernel.entry,
        boot_info_physical,
        kernel_stack_top,
        emergency_idt_physical,
    );
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}


#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    attributes: u8,
    offset_middle: u16,
    offset_high: u32,
    reserved: u32,
}

#[repr(C, packed)]
struct DescriptorTablePointer {
    limit: u16,
    base: u64,
}

const _: [(); 16] = [(); mem::size_of::<IdtEntry>()];
const _: [(); 10] = [(); mem::size_of::<DescriptorTablePointer>()];

unsafe fn build_emergency_idt(physical_base: u64) {
    let selector: u16;
    // SAFETY: Reading CS is valid in x86_64 UEFI long mode.
    unsafe {
        core::arch::asm!(
            "mov {0:x}, cs",
            out(reg) selector,
            options(nomem, nostack, preserves_flags)
        );
    }
    let handler = sovereign_emergency_fault as *const () as usize as u64;
    let gate = IdtEntry {
        offset_low: handler as u16,
        selector,
        ist: 0,
        attributes: 0x8e,
        offset_middle: (handler >> 16) as u16,
        offset_high: (handler >> 32) as u32,
        reserved: 0,
    };
    let table = physical_base as usize as *mut IdtEntry;
    for vector in 0..256 {
        // SAFETY: The caller reserved a page for exactly 256 16-byte gates.
        unsafe { ptr::write(table.add(vector), gate) };
    }
}

#[unsafe(no_mangle)]
extern "C" fn sovereign_emergency_fault() -> ! {
    loop {
        // SAFETY: Emergency exception path; do not return to an unknown state.
        unsafe { core::arch::asm!("cli; hlt", options(nomem, nostack)); }
    }
}

fn current_gdt() -> (u64, u16) {
    let mut pointer = DescriptorTablePointer { limit: 0, base: 0 };
    // SAFETY: SGDT writes the current GDT descriptor to this 10-byte buffer.
    unsafe {
        core::arch::asm!(
            "sgdt [{}]",
            in(reg) &mut pointer,
            options(nostack, preserves_flags)
        );
    }
    // SAFETY: The descriptor is packed and may be unaligned.
    let base = unsafe { ptr::addr_of!(pointer.base).read_unaligned() };
    // SAFETY: The first two bytes are the descriptor limit.
    let limit = unsafe { ptr::addr_of!(pointer.limit).read_unaligned() };
    (base, limit)
}

const ACPI_20_TABLE_GUID: crate::uefi::EfiGuid = crate::uefi::EfiGuid {
    data1: 0x8868_e871,
    data2: 0xe4f1,
    data3: 0x11d3,
    data4: [0xbc, 0x22, 0x00, 0x80, 0xc7, 0x3c, 0x88, 0x81],
};
const ACPI_10_TABLE_GUID: crate::uefi::EfiGuid = crate::uefi::EfiGuid {
    data1: 0xeb9d_2d30,
    data2: 0x2d88,
    data3: 0x11d3,
    data4: [0x9a, 0x16, 0x00, 0x90, 0x27, 0x3f, 0xc1, 0x4d],
};

fn guid_matches(left: &crate::uefi::EfiGuid, right: &crate::uefi::EfiGuid) -> bool {
    left.data1 == right.data1
        && left.data2 == right.data2
        && left.data3 == right.data3
        && left.data4 == right.data4
}

/// Locate and validate an ACPI RSDP from the UEFI configuration table.
unsafe fn find_acpi_rsdp(system_table: &EfiSystemTable) -> Option<(u64, u64)> {
    if system_table.number_of_table_entries == 0
        || system_table.number_of_table_entries > 4096
        || system_table.configuration_table.is_null()
    {
        return None;
    }

    let entries = system_table.configuration_table.cast::<EfiConfigurationTable>();
    let mut legacy = None;
    for index in 0..system_table.number_of_table_entries {
        // SAFETY: The system table advertises this many configuration entries.
        let entry = unsafe { ptr::read(entries.add(index)) };
        let is_acpi_20 = guid_matches(&entry.vendor_guid, &ACPI_20_TABLE_GUID);
        let is_acpi_10 = guid_matches(&entry.vendor_guid, &ACPI_10_TABLE_GUID);
        if !is_acpi_20 && !is_acpi_10 {
            continue;
        }
        // SAFETY: Firmware owns the configuration-table vendor pointer.
        let Some(length) = (unsafe { validate_rsdp(entry.vendor_table.cast()) }) else {
            continue;
        };
        let candidate = (entry.vendor_table as usize as u64, length);
        if is_acpi_20 {
            return Some(candidate);
        }
        legacy = Some(candidate);
    }
    legacy
}

/// Validate RSDP signature, legacy checksum, revision and extended checksum.
unsafe fn validate_rsdp(address: *const u8) -> Option<u64> {
    if address.is_null() || (address as usize) & 0xf != 0 {
        return None;
    }
    // SAFETY: Caller receives this pointer from a firmware configuration entry.
    let signature = unsafe { core::slice::from_raw_parts(address, 8) };
    if signature != b"RSD PTR " {
        return None;
    }
    // SAFETY: The ACPI RSDP fixed v1 prefix is 20 bytes.
    let legacy = unsafe { core::slice::from_raw_parts(address, 20) };
    if legacy.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)) != 0 {
        return None;
    }
    // SAFETY: Revision is byte 15 in the validated fixed prefix.
    let revision = unsafe { address.add(15).read() };
    let length = if revision == 0 {
        20u64
    } else if revision >= 2 {
        // SAFETY: ACPI 2.0+ extends the fixed prefix with a length at offset 20.
        let length = unsafe { ptr::read_unaligned(address.add(20).cast::<u32>()) } as u64;
        if !(36..=4096).contains(&length) {
            return None;
        }
        // SAFETY: The RSDP length is firmware-supplied; checksum reads are
        // bounded by the explicit 4 KiB maximum above.
        let extended = unsafe { core::slice::from_raw_parts(address, length as usize) };
        if extended.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)) != 0 {
            return None;
        }
        length
    } else {
        return None;
    };
    (address as u64).checked_add(length)?;
    Some(length)
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


fn transition_to_kernel(
    page_table_root: u64,
    kernel_entry: u64,
    boot_info_physical: u64,
    stack_top: u64,
    emergency_idt_physical: u64,
) -> ! {
    let new_stack = match (stack_top & !0xf).checked_sub(8) {
        Some(stack) => stack,
        None => loader_halt(),
    };
    // Provide the conventional dummy return slot expected at a SysV x86_64
    // function entry. The kernel entry never returns to the loader.
    // SAFETY: The dedicated stack is a live writable EFI_LOADER_DATA allocation.
    unsafe { ptr::write(new_stack as usize as *mut u64, 0) };

    let idtr = DescriptorTablePointer { limit: 4095, base: emergency_idt_physical };
    // SAFETY: ExitBootServices succeeded. The emergency IDT page and loader
    // handler are mapped in the new root; the IDTR descriptor itself is read
    // before RSP/CR3 change while the firmware stack is still active.
    unsafe {
        core::arch::asm!(
            "cli",
            "cld",
            "lidt [r9]",
            // Switch to the already-mapped bootstrap stack before changing
            // CR3. This keeps NMI/exception delivery off the firmware stack
            // throughout the address-space transition window.
            // Flush firmware global translations while its code mapping is
            // still active; otherwise a stale global entry could fault on
            // the very first instruction fetched after the CR3 switch.
            "mov r9, cr4",
            "test r9, 0x80",
            "jz 2f",
            "btr r9, 7",
            "mov cr4, r9",
            "bts r9, 7",
            "mov cr4, r9",
            "2:",
            "mov rsp, rcx",
            "mov cr3, rax",
            "mov r9, cr0",
            "or r9, 0x10000",
            "mov cr0, r9",
            "mov rdi, rdx",
            "jmp r8",
            in("rax") page_table_root,
            in("rdx") boot_info_physical,
            in("rcx") new_stack,
            in("r8") kernel_entry,
            in("r9") &idtr,
            options(noreturn)
        )
    }
}

fn loader_halt() -> ! {
    loop {
        // SAFETY: Fatal loader transition state; remain in ring 0 without
        // returning to firmware after a possibly partial ExitBootServices.
        unsafe { core::arch::asm!("cli; hlt", options(nomem, nostack)); }
    }
}
