use core::{ffi::c_void, ptr};

use super::{
    boot_services::EfiBootServices,
    protocols::{
        EfiFileProtocol, EfiLoadedImageProtocol, EfiSimpleFileSystemProtocol,
        EFI_FILE_INFO_GUID, EFI_FILE_MODE_READ, EFI_LOADED_IMAGE_PROTOCOL_GUID,
        EFI_SIMPLE_FILE_SYSTEM_PROTOCOL_GUID,
    },
    EfiHandle, EfiStatus, EfiSystemTable, EFI_COMPROMISED_DATA,
    EFI_INVALID_PARAMETER, EFI_SUCCESS, EFI_UNSUPPORTED,
};

const ELF_HEADER_SIZE: usize = 64;
const ELF_PROGRAM_HEADER_SIZE: usize = 56;
const MAX_PROGRAM_HEADERS: u16 = 128;
const FILE_INFO_BUFFER_SIZE: usize = 512;
const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
const EV_CURRENT: u32 = 1;
const EM_X86_64: u16 = 62;
const ET_EXEC: u16 = 2;
const PT_LOAD: u32 = 1;
const PF_X: u32 = 1;

/// Standard fallback location on the EFI System Partition.
const KERNEL_PATH: [u16; 31] = [
    92, 69, 70, 73, 92, 66, 79, 79, 84, 92,
    115, 111, 118, 101, 114, 101, 105, 103, 110, 45,
    107, 101, 114, 110, 101, 108, 46, 101, 108, 102, 0,
];

/// Locate the kernel on the boot device and validate its ELF header and load segments.
/// This stage validates only; it does not allocate or load kernel memory.
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

    let result = validate_kernel_file(kernel);
    close_file(kernel);
    close_file(root);
    result
}

fn validate_kernel_file(file: *mut EfiFileProtocol) -> EfiStatus {
    // SAFETY: File was returned by EFI_FILE_PROTOCOL.Open.
    let Some(protocol) = (unsafe { file.as_ref() }) else {
        return EFI_INVALID_PARAMETER;
    };
    let (Some(read), Some(set_position), Some(get_info)) =
        (protocol.read, protocol.set_position, protocol.get_info)
    else {
        return EFI_UNSUPPORTED;
    };

    let mut info = [0u8; FILE_INFO_BUFFER_SIZE];
    let mut info_size = info.len();
    // SAFETY: The GUID and writable buffer remain valid for this firmware call.
    let status = unsafe {
        get_info(file, &EFI_FILE_INFO_GUID, &mut info_size, info.as_mut_ptr().cast())
    };
    if status != EFI_SUCCESS {
        return status;
    }
    if info_size < 16 || info_size > info.len() {
        return EFI_COMPROMISED_DATA;
    }
    let file_size = read_u64(&info, 8);

    let mut header = [0u8; ELF_HEADER_SIZE];
    let mut header_size = header.len();
    // SAFETY: Header is writable for header_size bytes; firmware reports bytes read.
    let status = unsafe { read(file, &mut header_size, header.as_mut_ptr().cast()) };
    if status != EFI_SUCCESS {
        return status;
    }
    if header_size != ELF_HEADER_SIZE || file_size < ELF_HEADER_SIZE as u64 {
        return EFI_COMPROMISED_DATA;
    }

    if header[0..4] != ELF_MAGIC
        || header[4] != ELFCLASS64
        || header[5] != ELFDATA2LSB
        || header[6] != 1
        || u16::from_le_bytes([header[16], header[17]]) != ET_EXEC
        || u16::from_le_bytes([header[18], header[19]]) != EM_X86_64
        || read_u32(&header, 20) != EV_CURRENT
        || read_u16(&header, 52) as usize != ELF_HEADER_SIZE
    {
        return EFI_COMPROMISED_DATA;
    }

    let entry = read_u64(&header, 24);
    let phoff = read_u64(&header, 32);
    let phentsize = read_u16(&header, 54);
    let phnum = read_u16(&header, 56);
    if phoff < ELF_HEADER_SIZE as u64
        || phentsize as usize != ELF_PROGRAM_HEADER_SIZE
        || phnum == 0
        || phnum > MAX_PROGRAM_HEADERS
    {
        return EFI_COMPROMISED_DATA;
    }

    let table_size = match (phentsize as u64).checked_mul(phnum as u64) {
        Some(value) => value,
        None => return EFI_COMPROMISED_DATA,
    };
    let table_end = match phoff.checked_add(table_size) {
        Some(value) => value,
        None => return EFI_COMPROMISED_DATA,
    };
    if table_end > file_size {
        return EFI_COMPROMISED_DATA;
    }

    let mut loadable_segments = 0u16;
    let mut entry_is_executable = false;
    let mut program_header = [0u8; ELF_PROGRAM_HEADER_SIZE];

    for index in 0..phnum {
        let offset = match (index as u64)
            .checked_mul(ELF_PROGRAM_HEADER_SIZE as u64)
            .and_then(|delta| phoff.checked_add(delta))
        {
            Some(value) => value,
            None => return EFI_COMPROMISED_DATA,
        };

        // Each header is fetched independently to keep temporary memory bounded.
        // SAFETY: Firmware file position is an unsigned offset in the opened file.
        let status = unsafe { set_position(file, offset) };
        if status != EFI_SUCCESS {
            return status;
        }
        let mut bytes_read = program_header.len();
        // SAFETY: The fixed-size header buffer is writable for bytes_read bytes.
        let status = unsafe {
            read(file, &mut bytes_read, program_header.as_mut_ptr().cast())
        };
        if status != EFI_SUCCESS {
            return status;
        }
        if bytes_read != ELF_PROGRAM_HEADER_SIZE {
            return EFI_COMPROMISED_DATA;
        }

        if read_u32(&program_header, 0) != PT_LOAD {
            continue;
        }

        let flags = read_u32(&program_header, 4);
        let segment_offset = read_u64(&program_header, 8);
        let virtual_address = read_u64(&program_header, 16);
        let file_bytes = read_u64(&program_header, 32);
        let memory_bytes = read_u64(&program_header, 40);
        let alignment = read_u64(&program_header, 48);

        if file_bytes > memory_bytes {
            return EFI_COMPROMISED_DATA;
        }
        let segment_file_end = match segment_offset.checked_add(file_bytes) {
            Some(value) => value,
            None => return EFI_COMPROMISED_DATA,
        };
        if segment_file_end > file_size {
            return EFI_COMPROMISED_DATA;
        }
        if alignment > 1
            && (!alignment.is_power_of_two()
                || segment_offset % alignment != virtual_address % alignment)
        {
            return EFI_COMPROMISED_DATA;
        }

        if memory_bytes != 0 {
            let last_address = match virtual_address.checked_add(memory_bytes - 1) {
                Some(value) => value,
                None => return EFI_COMPROMISED_DATA,
            };
            if !is_canonical_x86_64(virtual_address)
                || !is_canonical_x86_64(last_address)
                || (virtual_address >> 47) != (last_address >> 47)
            {
                return EFI_COMPROMISED_DATA;
            }

            if flags & PF_X != 0 {
                let entry_in_segment =
                    entry >= virtual_address && entry <= last_address;
                entry_is_executable |= entry_in_segment;
            }
            loadable_segments = match loadable_segments.checked_add(1) {
                Some(value) => value,
                None => return EFI_COMPROMISED_DATA,
            };
        }
    }

    if loadable_segments == 0 || !is_canonical_x86_64(entry) || !entry_is_executable {
        return EFI_COMPROMISED_DATA;
    }

    EFI_SUCCESS
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

fn is_canonical_x86_64(address: u64) -> bool {
    let upper = address >> 47;
    upper == 0 || upper == 0x1_FFFF
}

fn close_file(file: *mut EfiFileProtocol) {
    // SAFETY: Callers pass handles returned by Open or OpenVolume.
    if let Some(close) = unsafe { file.as_ref() }.and_then(|file| file.close) {
        // SAFETY: The handle is still open and owned by this function.
        let _ = unsafe { close(file) };
    }
}
