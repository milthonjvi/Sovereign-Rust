use core::ffi::c_void;

pub type EfiHandle = *mut c_void;

/// Header shared by UEFI system, boot-service, runtime-service, and protocol tables.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct EfiTableHeader {
    pub signature: u64,
    pub revision: u32,
    pub header_size: u32,
    pub crc32: u32,
    pub reserved: u32,
}

/// UEFI system table fields in specification-defined order.
#[repr(C)]
pub struct EfiSystemTable {
    pub header: EfiTableHeader,
    pub firmware_vendor: *mut u16,
    pub firmware_revision: u32,
    _padding: u32,
    pub console_in_handle: EfiHandle,
    pub con_in: *mut c_void,
    pub console_out_handle: EfiHandle,
    pub con_out: *mut super::console::EfiSimpleTextOutputProtocol,
    pub console_err_handle: EfiHandle,
    pub std_err: *mut super::console::EfiSimpleTextOutputProtocol,
    pub runtime_services: *mut c_void,
    pub boot_services: *mut c_void,
    pub number_of_table_entries: usize,
    pub configuration_table: *mut c_void,
}


/// Entry in the UEFI system configuration-table array.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct EfiConfigurationTable {
    pub vendor_guid: super::EfiGuid,
    pub vendor_table: *mut c_void,
}

const _: [(); 120] = [(); core::mem::size_of::<EfiSystemTable>()];
const _: [(); 24] = [(); core::mem::size_of::<EfiConfigurationTable>()];
const _: [(); 16] = [(); core::mem::size_of::<super::EfiGuid>()];

const _: [(); 104] = [(); core::mem::offset_of!(EfiSystemTable, number_of_table_entries)];
const _: [(); 112] = [(); core::mem::offset_of!(EfiSystemTable, configuration_table)];
const _: [(); 16] = [(); core::mem::offset_of!(EfiConfigurationTable, vendor_table)];
