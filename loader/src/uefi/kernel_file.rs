use core::{ffi::c_void, ptr};

use super::{
    boot_services::EfiBootServices,
    protocols::{
        EfiFileProtocol, EfiLoadedImageProtocol, EfiSimpleFileSystemProtocol,
        EFI_FILE_MODE_READ, EFI_LOADED_IMAGE_PROTOCOL_GUID,
        EFI_SIMPLE_FILE_SYSTEM_PROTOCOL_GUID,
    },
    EfiHandle, EfiStatus, EfiSystemTable, EFI_COMPROMISED_DATA,
    EFI_INVALID_PARAMETER, EFI_SUCCESS, EFI_UNSUPPORTED,
};

const ELF_HEADER_SIZE: usize = 64;
const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
const EV_CURRENT: u32 = 1;
const EM_X86_64: u16 = 62;
const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;
const ELF64_PROGRAM_HEADER_SIZE: u16 = 56;

/// Standard fallback location on the EFI System Partition.
const KERNEL_PATH: [u16; 31] = [
    92, 69, 70, 73, 92, 66, 79, 79, 84, 92,
    115, 111, 118, 101, 114, 101, 105, 103, 110, 45,
    107, 101, 114, 110, 101, 108, 46, 101, 108, 102, 0,
];

/// Locate the boot device, open its EFI filesystem, and validate the kernel ELF
/// identification and fixed header fields. Only the 64-byte ELF header is read.
pub fn inspect_kernel_file(
    image_handle: EfiHandle,
    system_table: &mut EfiSystemTable,
) -> EfiStatus {
    if image_handle.is_null() || system_table.boot_services.is_null() {
        return EFI_INVALID_PARAMETER;
    }

    // SAFETY: UEFI supplies this Boot Services table in EFI_SYSTEM_TABLE.
    let boot = unsafe { &mut *(system_table.boot_services as *mut EfiBootServices) };
    let Some(handle_protocol) = boot.handle_protocol else {
        return EFI_UNSUPPORTED;
    };

    let mut loaded_image_ptr: *mut c_void = ptr::null_mut();
    // SAFETY: GUID and output storage are valid for this synchronous firmware call.
    let status = unsafe {
        handle_protocol(
            image_handle,
            &EFI_LOADED_IMAGE_PROTOCOL_GUID,
            &mut loaded_image_ptr,
        )
    };
    if status != EFI_SUCCESS {
        return status;
    }
    if loaded_image_ptr.is_null() {
        return EFI_COMPROMISED_DATA;
    }

    // SAFETY: HandleProtocol returned an EFI_LOADED_IMAGE_PROTOCOL interface.
    let loaded_image = unsafe { &mut *(loaded_image_ptr as *mut EfiLoadedImageProtocol) };
    if loaded_image.device_handle.is_null() {
        return EFI_COMPROMISED_DATA;
    }

    let mut filesystem_ptr: *mut c_void = ptr::null_mut();
    // SAFETY: Device handle originates from the firmware's loaded-image protocol.
    let status = unsafe {
        handle_protocol(
            loaded_image.device_handle,
            &EFI_SIMPLE_FILE_SYSTEM_PROTOCOL_GUID,
            &mut filesystem_ptr,
        )
    };
    if status != EFI_SUCCESS {
        return status;
    }
    if filesystem_ptr.is_null() {
        return EFI_COMPROMISED_DATA;
    }

    // SAFETY: HandleProtocol returned an EFI_SIMPLE_FILE_SYSTEM_PROTOCOL.
    let filesystem = unsafe { &mut *(filesystem_ptr as *mut EfiSimpleFileSystemProtocol) };
    let Some(open_volume) = filesystem.open_volume else {
        return EFI_UNSUPPORTED;
    };

    let mut root: *mut EfiFileProtocol = ptr::null_mut();
    // SAFETY: Root output pointer is valid and filesystem is firmware-owned.
    let status = unsafe { open_volume(filesystem, &mut root) };
    if status != EFI_SUCCESS {
        return status;
    }
    if root.is_null() {
        return EFI_COMPROMISED_DATA;
    }

    // SAFETY: Root is a valid protocol handle returned by OpenVolume.
    let Some(open_file) = (unsafe { root.as_ref() }).and_then(|file| file.open) else {
        close_file(root);
        return EFI_UNSUPPORTED;
    };

    let mut kernel: *mut EfiFileProtocol = ptr::null_mut();
    // SAFETY: Path is null-terminated UTF-16 and output storage is valid.
    let status = unsafe {
        open_file(root, &mut kernel, KERNEL_PATH.as_ptr(), EFI_FILE_MODE_READ, 0)
    };
    if status != EFI_SUCCESS {
        close_file(root);
        return status;
    }
    if kernel.is_null() {
        close_file(root);
        return EFI_COMPROMISED_DATA;
    }

    let result = read_and_validate_header(kernel);
    close_file(kernel);
    close_file(root);
    result
}

fn read_and_validate_header(file: *mut EfiFileProtocol) -> EfiStatus {
    // SAFETY: File was returned by EFI_FILE_PROTOCOL.Open.
    let Some(read) = (unsafe { file.as_ref() }).and_then(|file| file.read) else {
        return EFI_UNSUPPORTED;
    };

    let mut header = [0u8; ELF_HEADER_SIZE];
    let mut size = header.len();
    // SAFETY: Header is writable for size bytes; firmware reports bytes read.
    let status = unsafe { read(file, &mut size, header.as_mut_ptr().cast()) };
    if status != EFI_SUCCESS {
        return status;
    }
    if size != ELF_HEADER_SIZE {
        return EFI_COMPROMISED_DATA;
    }

    let file_type = u16::from_le_bytes([header[16], header[17]]);
    let machine = u16::from_le_bytes([header[18], header[19]]);
    let version = u32::from_le_bytes([header[20], header[21], header[22], header[23]]);
    let program_header_offset = u64::from_le_bytes([
        header[32], header[33], header[34], header[35],
        header[36], header[37], header[38], header[39],
    ]);
    let program_header_size = u16::from_le_bytes([header[54], header[55]]);
    let program_header_count = u16::from_le_bytes([header[56], header[57]]);

    if header[0..4] != ELF_MAGIC
        || header[4] != ELFCLASS64
        || header[5] != ELFDATA2LSB
        || header[6] != 1
        || !matches!(file_type, ET_EXEC | ET_DYN)
        || machine != EM_X86_64
        || version != EV_CURRENT
        || u16::from_le_bytes([header[52], header[53]]) != ELF_HEADER_SIZE as u16
        || program_header_offset < ELF_HEADER_SIZE as u64
        || program_header_size != ELF64_PROGRAM_HEADER_SIZE
        || program_header_count == 0
    {
        return EFI_COMPROMISED_DATA;
    }

    EFI_SUCCESS
}

fn close_file(file: *mut EfiFileProtocol) {
    // SAFETY: Callers pass handles returned by Open or OpenVolume.
    if let Some(close) = unsafe { file.as_ref() }.and_then(|file| file.close) {
        // SAFETY: The handle is still open and owned by this function.
        let _ = unsafe { close(file) };
    }
}
