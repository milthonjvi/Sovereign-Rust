#![no_std]
#![no_main]

//! Freestanding x86_64 kernel entry point.

use core::panic::PanicInfo;
use sovereign_kernel::arch::x86_64::control::{disable_interrupts, halt};
use sovereign_kernel::arch::x86_64::init::initialize_baseline;
use sovereign_kernel::boot::limine::LimineBootInfo;

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    // Limine enters in long mode with interrupts disabled. Enforce that state
    // before touching boot responses or beginning kernel initialization.
    // SAFETY: This is the ring-0 kernel entry point.
    unsafe { disable_interrupts() };

    // Configure the baseline x86_64 floating-point control state before any
    // kernel subsystem or task can rely on it.
    // SAFETY: Still on the BSP, with interrupts disabled and no task context.
    let _cpu_info = unsafe { initialize_baseline() };

    // SAFETY: The bootloader has completed the Limine handoff before _start.
    // A missing mandatory response is a fatal boot contract violation.
    let _boot_info = unsafe {
        LimineBootInfo::from_responses()
            .unwrap_or_else(|| fatal_halt())
    };

    // The next stage will initialize diagnostics, memory, and the IDT.
    // Until then, do not enable interrupts or touch uninitialized devices.
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
    // SAFETY: This function only runs in the kernel at CPL 0.
    unsafe { disable_interrupts() };
    loop {
        // SAFETY: A fatal kernel state must not return to an unknown context.
        unsafe { halt() };
    }
}
