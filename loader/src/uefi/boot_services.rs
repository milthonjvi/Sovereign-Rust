use super::{EfiHandle, EfiStatus, EfiTableHeader};

/// UEFI Boot Services prefix through HandleProtocol.
#[repr(C)]
pub struct EfiBootServices {
    pub header: EfiTableHeader,
    _raise_tpl: usize,
    _restore_tpl: usize,
    _allocate_pages: usize,
    _free_pages: usize,
    _get_memory_map: usize,
    _allocate_pool: usize,
    _free_pool: usize,
    _create_event: usize,
    _set_timer: usize,
    _wait_for_event: usize,
    _signal_event: usize,
    _close_event: usize,
    _check_event: usize,
    _install_protocol_interface: usize,
    _reinstall_protocol_interface: usize,
    _uninstall_protocol_interface: usize,
    pub handle_protocol: Option<unsafe extern "efiapi" fn(
        handle: EfiHandle,
        protocol: *const super::EfiGuid,
        interface: *mut *mut core::ffi::c_void,
    ) -> EfiStatus>,
}
