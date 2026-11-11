mod boot_services;
mod console;
mod kernel_file;
mod protocols;
mod system_table;

pub use console::Console;
pub use system_table::{EfiHandle, EfiSystemTable, EfiTableHeader};

pub use kernel_file::inspect_kernel_file;

/// UEFI GUID representation.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct EfiGuid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

/// UEFI status values use the high bit to distinguish errors.
pub type EfiStatus = usize;

pub const EFI_SUCCESS: EfiStatus = 0;
pub const EFI_ERROR_BIT: EfiStatus = 1usize << (usize::BITS - 1);
pub const EFI_INVALID_PARAMETER: EfiStatus = EFI_ERROR_BIT | 2;
pub const EFI_UNSUPPORTED: EfiStatus = EFI_ERROR_BIT | 3;
pub const EFI_NOT_FOUND: EfiStatus = EFI_ERROR_BIT | 14;
pub const EFI_COMPROMISED_DATA: EfiStatus = EFI_ERROR_BIT | 33;

/// EFI_SYSTEM_TABLE signature ("IBI SYST" in little-endian byte order).
pub const EFI_SYSTEM_TABLE_SIGNATURE: u64 = 0x5453_5953_2049_4249;
