#![no_std]
#![no_main]

//! Freestanding x86_64 kernel entry point.

use core::panic::PanicInfo;
use sovereign_kernel::arch::x86_64::control::{disable_interrupts, halt};
use sovereign_kernel::arch::x86_64::init::initialize_baseline;
use sovereign_kernel::boot::limine::LimineBootInfo;
use sovereign_kernel::drivers::framebuffer::{Color, FramebufferSurface};

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

    // ACPI is optional. When firmware provides an RSDP, validate it and
    // discover the platform tables through the bootloader's HHDM mapping.
    // A missing or malformed ACPI table set is non-fatal at this stage;
    // interrupt controllers and timers remain disabled until later setup.
    let _acpi_tables = unsafe {
        _boot_info.acpi_rsdp()
            .ok()
            .flatten()
            .and_then(|rsdp| _boot_info.discover_acpi(&rsdp).ok())
    };

    // Initialize the boot-provided display when Limine exposes a supported
    // 32-bit RGB/BGR framebuffer. Unsupported formats remain non-fatal.
    // SAFETY: LimineBootInfo was collected after handoff and its response
    // structures and framebuffer mapping remain valid during this boot stage.
    if let Some((address, framebuffer_info)) = unsafe { _boot_info.framebuffer_info() } {
        if let Ok(mapped_len) = framebuffer_info.validate() {
            // SAFETY: Limine supplies a writable framebuffer mapping; the
            // surface is used only here and does not escape this scope.
            if let Ok(mut display) = unsafe {
                FramebufferSurface::from_mapped(address, mapped_len, framebuffer_info)
            } {
                display.clear(Color::BLACK);
                display.fill_rect(0, 0, framebuffer_info.width, 4, Color::with_alpha(32, 160, 255, 0));
                let _ = display.draw_text(
                    12,
                    16,
                    "SOVEREIGN CORE",
                    Color::WHITE,
                    None,
                );
                let _ = display.draw_text(
                    12,
                    34,
                    "KERNEL INITIALIZED",
                    Color::with_alpha(32, 160, 255, 0),
                    None,
                );
            }
        }
    }

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
