use core::ffi::c_void;

use super::{EfiGuid, EfiHandle, EfiStatus};

/// EFI_LOADED_IMAGE_PROTOCOL.
#[repr(C)]
pub struct EfiLoadedImageProtocol {
    pub revision: u32,
    pub parent_handle: EfiHandle,
    pub system_table: *mut c_void,
    pub device_handle: EfiHandle,
    pub file_path: *mut c_void,
    pub reserved: *mut c_void,
    pub load_options_size: u32,
    _padding: u32,
    pub load_options: *mut c_void,
    pub image_base: *mut c_void,
    pub image_size: usize,
    pub image_code_type: u32,
    pub image_data_type: u32,
    pub unload: Option<unsafe extern "efiapi" fn(image_handle: EfiHandle) -> EfiStatus>,
}

/// EFI_SIMPLE_FILE_SYSTEM_PROTOCOL.
#[repr(C)]
pub struct EfiSimpleFileSystemProtocol {
    pub revision: u64,
    pub open_volume: Option<unsafe extern "efiapi" fn(
        this: *mut EfiSimpleFileSystemProtocol,
        root: *mut *mut EfiFileProtocol,
    ) -> EfiStatus>,
}

/// EFI_FILE_PROTOCOL prefix through GetInfo, preserving specification slot order.
#[repr(C)]
pub struct EfiFileProtocol {
    pub revision: u64,
    pub open: Option<unsafe extern "efiapi" fn(
        this: *mut EfiFileProtocol,
        new_handle: *mut *mut EfiFileProtocol,
        file_name: *const u16,
        open_mode: u64,
        attributes: u64,
    ) -> EfiStatus>,
    pub close: Option<unsafe extern "efiapi" fn(this: *mut EfiFileProtocol) -> EfiStatus>,
    _delete: usize,
    pub read: Option<unsafe extern "efiapi" fn(
        this: *mut EfiFileProtocol,
        buffer_size: *mut usize,
        buffer: *mut c_void,
    ) -> EfiStatus>,
    _write: usize,
    _get_position: usize,
    pub set_position: Option<unsafe extern "efiapi" fn(
        this: *mut EfiFileProtocol,
        position: u64,
    ) -> EfiStatus>,
    pub get_info: Option<unsafe extern "efiapi" fn(
        this: *mut EfiFileProtocol,
        information_type: *const EfiGuid,
        buffer_size: *mut usize,
        buffer: *mut c_void,
    ) -> EfiStatus>,
}

pub const EFI_LOADED_IMAGE_PROTOCOL_GUID: EfiGuid = EfiGuid {
    data1: 0x5B1B_31A1,
    data2: 0x9562,
    data3: 0x11D2,
    data4: [0x8E, 0x3F, 0x00, 0xA0, 0xC9, 0x69, 0x72, 0x3B],
};

pub const EFI_SIMPLE_FILE_SYSTEM_PROTOCOL_GUID: EfiGuid = EfiGuid {
    data1: 0x964E_5B22,
    data2: 0x6459,
    data3: 0x11D2,
    data4: [0x8E, 0x39, 0x00, 0xA0, 0xC9, 0x69, 0x72, 0x3B],
};

/// EFI_FILE_INFO GUID.
pub const EFI_FILE_INFO_GUID: EfiGuid = EfiGuid {
    data1: 0x0957_6E92,
    data2: 0x6D3F,
    data3: 0x11D2,
    data4: [0x8E, 0x39, 0x00, 0xA0, 0xC9, 0x69, 0x72, 0x3B],
};

pub const EFI_FILE_MODE_READ: u64 = 1;
