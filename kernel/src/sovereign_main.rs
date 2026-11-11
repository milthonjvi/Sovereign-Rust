#![no_std]
#![no_main]

//! Sovereign Boot entry point, built separately from the Limine-compatible image.

use core::{panic::PanicInfo, ptr, slice};
use sovereign_kernel::arch::x86_64::control::{disable_interrupts, halt, read_cr4};
use sovereign_kernel::arch::x86_64::init::initialize_baseline;
use sovereign_kernel::boot::sovereign::SovereignBootInfo;
use sovereign_kernel::memory::boot_map::BootMemoryMap;
use sovereign_kernel::memory::paging::{RecursivePageTable, RECURSIVE_SLOT};

#[unsafe(no_mangle)]
pub extern "C" fn _start(boot_info_pointer: *const SovereignBootInfo) -> ! {
    // The UEFI loader enters in long mode with the new Sovereign page tables.
    // SAFETY: This is the dedicated Sovereign Boot kernel entry point.
    unsafe { disable_interrupts() };
    // SAFETY: BSP context, interrupts disabled, and no task state exists yet.
    let _cpu_info = unsafe { initialize_baseline() };

    // SAFETY: Sovereign Boot maps the boot-info page and memory-map buffer
    // identity before transferring control.
    let boot_info = unsafe {
        SovereignBootInfo::from_ptr(boot_info_pointer).unwrap_or_else(|_| fatal_halt())
    };
    // SAFETY: The loader preserves the complete memory-map buffer identity map.
    let memory_map = unsafe {
        boot_info.memory_map().unwrap_or_else(|_| fatal_halt())
    };
    let usable_memory = BootMemoryMap::new(&memory_map)
        .and_then(|map| map.usable_summary())
        .unwrap_or_else(|_| fatal_halt());
    if usable_memory.region_count == 0 || usable_memory.page_count == 0 {
        fatal_halt();
    }

    // The loader allocates all three objects as EFI_LOADER_DATA. Confirm that
    // their complete physical ranges are still described as such before
    // accepting any boot-provided pointer or adopting the kernel image.
    const EFI_LOADER_DATA: u32 = 2;
    let boot_info_physical = boot_info_pointer as u64;
    let ownership_valid = memory_map.contains_range_with_type(
            boot_info_physical,
            4096,
            EFI_LOADER_DATA,
        ).unwrap_or(false)
        && memory_map.contains_range_with_type(
            boot_info.memory_map_physical,
            boot_info.memory_map_size,
            EFI_LOADER_DATA,
        ).unwrap_or(false)
        && memory_map.contains_range_with_type(
            boot_info.kernel_physical_base,
            boot_info.kernel_image_size,
            EFI_LOADER_DATA,
        ).unwrap_or(false);
    if !ownership_valid {
        fatal_halt();
    }

    // Validate the firmware RSDP before exposing it to ACPI table discovery.
    // UEFI normally places it in ACPI reclaimable memory; reserved and NVS
    // descriptors are also valid firmware-owned locations.
    if boot_info.flags & sovereign_kernel::boot::sovereign::BOOT_INFO_FLAG_ACPI_VALID != 0 {
        let rsdp_physical = boot_info.acpi_rsdp_physical;
        let revision = unsafe { (rsdp_physical as *const u8).add(15).read() };
        let rsdp_length = if revision == 0 {
            20u64
        } else if revision >= 2 {
            let length = unsafe {
                ptr::read_unaligned((rsdp_physical as *const u8).add(20).cast::<u32>())
            } as u64;
            if !(36..=4096).contains(&length) {
                fatal_halt();
            }
            length
        } else {
            fatal_halt();
        };
        let rsdp_in_firmware_memory =
            memory_map.contains_range_with_type(rsdp_physical, rsdp_length, 0).unwrap_or(false)
            || memory_map.contains_range_with_type(rsdp_physical, rsdp_length, 9).unwrap_or(false)
            || memory_map.contains_range_with_type(rsdp_physical, rsdp_length, 10).unwrap_or(false);
        if !rsdp_in_firmware_memory {
            fatal_halt();
        }
        // SAFETY: The loader mapped the complete validated RSDP range read-only.
        let rsdp_bytes = unsafe {
            slice::from_raw_parts(rsdp_physical as *const u8, rsdp_length as usize)
        };
        let rsdp = sovereign_kernel::drivers::acpi::rsdp::Rsdp::parse(rsdp_bytes)
            .unwrap_or_else(|_| fatal_halt());
        let _root_table = rsdp.root_table_address().unwrap_or_else(|_| fatal_halt());
    }

    // Verify the recursive page-table window and the critical mappings before
    // installing interrupt state or allowing any subsystem to use memory.
    if !validate_boot_mappings(boot_info_pointer as u64, &boot_info) {
        fatal_halt();
    }

    // Install the kernel's GDT/TSS, IDT, and masked legacy PIC only after the
    // Sovereign handoff and every memory descriptor have passed validation.
    // SAFETY: BSP at CPL 0 with interrupts disabled and a validated stack.
    if unsafe { sovereign_kernel::interrupts::init::initialize_bootstrap() }.is_err() {
        fatal_halt();
    }

    // The initial Sovereign entry deliberately does not enable interrupts or
    // reclaim loader memory. Those actions wait for the memory manager and
    // subsystem ownership model to be initialized.
    let _boot_info = boot_info;
    loop {
        // SAFETY: IF remains clear; HLT is valid at CPL 0.
        unsafe { halt() };
    }
}

fn validate_boot_mappings(
    boot_info_virtual: u64,
    boot_info: &SovereignBootInfo,
) -> bool {
    // LA57 is the architectural selector for five-level paging.
    // SAFETY: The kernel is executing at CPL 0 with interrupts disabled.
    let levels = if unsafe { read_cr4() } & (1 << 12) != 0 { 5 } else { 4 };
    // SAFETY: Sovereign Boot installs the recursive root reference at slot 510.
    let page_tables = match unsafe { RecursivePageTable::new(levels, RECURSIVE_SLOT) } {
        Ok(tables) => tables,
        Err(_) => return false,
    };

    // SAFETY: The recursive window was installed by the loader and remains
    // active; interrupts are disabled and the BSP is the only running CPU.
    let boot_info_mapping = match unsafe { page_tables.translate(boot_info_virtual) } {
        Ok(mapping) => mapping,
        Err(_) => return false,
    };
    if boot_info_mapping.physical_address != boot_info_virtual
        || boot_info_mapping.writable
        || boot_info_mapping.executable
        || boot_info_mapping.user_accessible
    {
        return false;
    }

    // SAFETY: Same active, immutable bootstrap page-table state.
    let memory_map_mapping = match unsafe {
        page_tables.translate(boot_info.memory_map_physical)
    } {
        Ok(mapping) => mapping,
        Err(_) => return false,
    };
    if memory_map_mapping.physical_address != boot_info.memory_map_physical
        || memory_map_mapping.writable
        || memory_map_mapping.executable
        || memory_map_mapping.user_accessible
    {
        return false;
    }

    if boot_info.flags & sovereign_kernel::boot::sovereign::BOOT_INFO_FLAG_ACPI_VALID != 0 {
        // SAFETY: The loader mapped the RSDP identity page read-only and NX.
        let rsdp_mapping = match unsafe {
            page_tables.translate(boot_info.acpi_rsdp_physical)
        } {
            Ok(mapping) => mapping,
            Err(_) => return false,
        };
        if rsdp_mapping.physical_address != boot_info.acpi_rsdp_physical
            || rsdp_mapping.writable
            || rsdp_mapping.executable
            || rsdp_mapping.user_accessible
        {
            return false;
        }
    }

    // The kernel entry must be supervisor RX, never writable or user-accessible.
    // SAFETY: The loader mapped every kernel PT_LOAD segment before handoff.
    let entry_mapping = match unsafe { page_tables.translate(boot_info.kernel_entry) } {
        Ok(mapping) => mapping,
        Err(_) => return false,
    };
    if !entry_mapping.executable || entry_mapping.writable || entry_mapping.user_accessible {
        return false;
    }

    // Confirm that execution is using the dedicated writable, NX bootstrap stack.
    let stack_pointer: u64;
    // SAFETY: Reading RSP has no side effects and is valid in long mode.
    unsafe {
        core::arch::asm!(
            "mov {}, rsp",
            out(reg) stack_pointer,
            options(nomem, nostack, preserves_flags),
        );
    }
    // SAFETY: The stack mapping must remain present while this function runs.
    let stack_mapping = match unsafe { page_tables.translate(stack_pointer) } {
        Ok(mapping) => mapping,
        Err(_) => return false,
    };
    stack_mapping.writable
        && !stack_mapping.executable
        && !stack_mapping.user_accessible
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    fatal_halt()
}

fn fatal_halt() -> ! {
    // SAFETY: This function executes only after entering the kernel at CPL 0.
    unsafe { disable_interrupts() };
    loop {
        // SAFETY: A malformed boot handoff must not return to an unknown context.
        unsafe { halt() };
    }
}
