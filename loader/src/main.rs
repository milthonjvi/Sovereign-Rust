#![no_std]
#![no_main]

mod boot_info;
mod uefi;

use core::{mem, panic::PanicInfo};
use uefi::{
    inspect_kernel_file, Console, EfiHandle, EfiStatus, EfiSystemTable,
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

    let status = inspect_kernel_file(image_handle, system_table);
    if status != EFI_SUCCESS {
        let _ = console.write_ascii("Kernel file lookup or ELF validation failed.\r\n");
        return status;
    }

    if !console.write_ascii("Kernel ELF64 header validated.\r\n") {
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
