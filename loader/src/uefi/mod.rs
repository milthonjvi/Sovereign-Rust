mod console;
mod system_table;

pub use console::Console;
pub use system_table::{EfiHandle, EfiSystemTable};

/// UEFI status values use the high bit to distinguish errors.
pub type EfiStatus = usize;

pub const EFI_SUCCESS: EfiStatus = 0;
pub const EFI_ERROR_BIT: EfiStatus = 1usize << (usize::BITS - 1);
pub const EFI_INVALID_PARAMETER: EfiStatus = EFI_ERROR_BIT | 2;
pub const EFI_COMPROMISED_DATA: EfiStatus = EFI_ERROR_BIT | 33;

/// EFI_SYSTEM_TABLE signature ("IBI SYST" in little-endian byte order).
pub const EFI_SYSTEM_TABLE_SIGNATURE: u64 = 0x5453_5953_2049_4249;
