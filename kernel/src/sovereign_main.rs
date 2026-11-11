#![no_std]
#![no_main]

//! Sovereign Boot entry point, built separately from the Limine-compatible image.

use core::panic::PanicInfo;
use sovereign_kernel::arch::x86_64::control::{disable_interrupts, halt};
use sovereign_kernel::arch::x86_64::init::initialize_baseline;
use sovereign_kernel::boot::sovereign::{SovereignBootInfo, SovereignBootError};

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
    for index in 0..memory_map.len() {
        let descriptor = memory_map.descriptor(index).unwrap_or_else(|_| fatal_halt());
        let region_size = descriptor.number_of_pages.checked_mul(4096)
            .unwrap_or_else(|| fatal_halt());
        let _region_end = descriptor.physical_start.checked_add(region_size)
            .unwrap_or_else(|| fatal_halt());
        if descriptor.number_of_pages == 0 {
            fatal_halt();
        }
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
