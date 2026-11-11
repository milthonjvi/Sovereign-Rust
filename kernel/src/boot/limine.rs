//! Limine boot protocol declarations for the x86_64 kernel.
//!
//! Protocol reference: https://github.com/Limine-Bootloader/limine-protocol
//! Base revision 6 is requested. The x86_64 machine-state guarantees introduced
//! in revision 5 are therefore available when the bootloader supports revision 6.

use core::ptr;

use crate::drivers::framebuffer::{FramebufferInfo, PixelFormat};
use crate::drivers::acpi::{AcpiError, rsdp::Rsdp};

const COMMON_MAGIC_0: u64 = 0xc7b1_dd30_df4c_8b88;
const COMMON_MAGIC_1: u64 = 0x0a82_e883_a194_f07b;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LimineRequest<T> {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: *mut T,
}

// The bootloader writes response pointers before transferring control to us.
unsafe impl<T> Sync for LimineRequest<T> {}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RsdpResponse {
    pub revision: u64,
    pub address: *const u8,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct HhdmResponse {
    pub revision: u64,
    pub offset: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MemoryMapEntry {
    pub base: u64,
    pub length: u64,
    pub kind: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MemoryMapResponse {
    pub revision: u64,
    pub entry_count: u64,
    pub entries: *mut *mut MemoryMapEntry,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Framebuffer {
    pub address: *mut u8,
    pub width: u64,
    pub height: u64,
    pub pitch: u64,
    pub bpp: u16,
    pub memory_model: u8,
    pub red_mask_size: u8,
    pub red_mask_shift: u8,
    pub green_mask_size: u8,
    pub green_mask_shift: u8,
    pub blue_mask_size: u8,
    pub blue_mask_shift: u8,
    pub unused: [u8; 7],
    pub edid_size: u64,
    pub edid: *mut u8,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct FramebufferResponse {
    pub revision: u64,
    pub framebuffer_count: u64,
    pub framebuffers: *mut *mut Framebuffer,
}

#[used]
#[unsafe(link_section = ".limine_requests_start")]
#[unsafe(no_mangle)]
pub static LIMINE_REQUESTS_START_MARKER: [u64; 4] = [
    0xf6b8_f4b3_9de7_d1ae,
    0xfab9_1a69_40fc_b9cf,
    0x785c_6ed0_15d3_e316,
    0x181e_920a_7852_b9d9,
];

#[used]
#[unsafe(link_section = ".limine_requests")]
#[unsafe(no_mangle)]
pub static mut LIMINE_BASE_REVISION: [u64; 3] = [
    0xf956_2b2d_5c95_a6c8,
    0x6a7b_3849_4453_6bdc,
    6,
];

#[used]
#[unsafe(link_section = ".limine_requests")]
#[unsafe(no_mangle)]
pub static mut LIMINE_RSDP_REQUEST: LimineRequest<RsdpResponse> = LimineRequest {
    id: [COMMON_MAGIC_0, COMMON_MAGIC_1, 0xc5e7_7b6b_397e_7b43, 0x2763_7845_accd_cf3c],
    revision: 0,
    response: ptr::null_mut(),
};

#[used]
#[unsafe(link_section = ".limine_requests")]
#[unsafe(no_mangle)]
pub static mut LIMINE_HHDM_REQUEST: LimineRequest<HhdmResponse> = LimineRequest {
    id: [COMMON_MAGIC_0, COMMON_MAGIC_1, 0x48dc_f1cb_8ad2_b852, 0x6398_4e95_9a98_244b],
    revision: 0,
    response: ptr::null_mut(),
};

#[used]
#[unsafe(link_section = ".limine_requests")]
#[unsafe(no_mangle)]
pub static mut LIMINE_MEMMAP_REQUEST: LimineRequest<MemoryMapResponse> = LimineRequest {
    id: [COMMON_MAGIC_0, COMMON_MAGIC_1, 0x67cf_3d9d_378a_806f, 0xe304_acdf_c50c_3c62],
    revision: 0,
    response: ptr::null_mut(),
};

#[used]
#[unsafe(link_section = ".limine_requests")]
#[unsafe(no_mangle)]
pub static mut LIMINE_FRAMEBUFFER_REQUEST: LimineRequest<FramebufferResponse> = LimineRequest {
    id: [COMMON_MAGIC_0, COMMON_MAGIC_1, 0x9d58_27dc_d881_dd75, 0xa314_8604_f6fa_b11b],
    revision: 0,
    response: ptr::null_mut(),
};

#[used]
#[unsafe(link_section = ".limine_requests_end")]
#[unsafe(no_mangle)]
pub static LIMINE_REQUESTS_END_MARKER: [u64; 2] = [
    0xadc0_e053_1bb1_0d03,
    0x9572_709f_3176_4c62,
];

#[derive(Clone, Copy)]
pub struct LimineBootInfo {
    pub hhdm_offset: u64,
    pub memory_map_entries: *mut *mut MemoryMapEntry,
    pub memory_map_count: usize,
    pub framebuffer: *mut FramebufferResponse,
    pub rsdp_address: *const u8,
}

impl LimineBootInfo {

    /// Parse the firmware RSDP supplied by Limine.
    ///
    /// Limine base revision 6 supplies a virtual address, so this pointer must
    /// not be adjusted by the HHDM offset.
    ///
    /// # Safety
    /// The bootloader-provided RSDP pointer must refer to readable memory.
    pub unsafe fn acpi_rsdp(&self) -> Result<Option<Rsdp>, AcpiError> {
        if self.rsdp_address.is_null() {
            return Ok(None);
        }

        // SAFETY: Guaranteed by the caller and Limine's RSDP response contract.
        let revision = unsafe { ptr::read(self.rsdp_address.add(15)) };
        let length = if revision == 0 {
            20usize
        } else {
            // SAFETY: ACPI 2.0+ RSDP has a 36-byte fixed prefix.
            let length_ptr = unsafe { self.rsdp_address.add(20).cast::<u32>() };
            // SAFETY: The RSDP response points to a readable ACPI structure.
            let length = unsafe { ptr::read_unaligned(length_ptr) } as usize;
            if !(36..=4096).contains(&length) {
                return Err(AcpiError::InvalidLength);
            }
            length
        };

        // SAFETY: The validated RSDP length bounds this byte range; firmware
        // memory remains readable during early boot.
        let bytes = unsafe { core::slice::from_raw_parts(self.rsdp_address, length) };
        Rsdp::parse(bytes).map(Some)
    }

    /// Return the first boot-provided framebuffer and normalized metadata.
    ///
    /// # Safety
    /// Must only be called after Limine handoff while its response structures
    /// and framebuffer mapping remain valid.
    pub unsafe fn framebuffer_info(&self) -> Option<(*mut u8, FramebufferInfo)> {
        if self.framebuffer.is_null() {
            return None;
        }
        // SAFETY: The caller guarantees Limine response memory remains valid.
        let response = unsafe { ptr::read(self.framebuffer) };
        if response.framebuffer_count == 0 || response.framebuffers.is_null() {
            return None;
        }
        // SAFETY: Limine provides an array with framebuffer_count entries.
        let framebuffer_ptr = unsafe { ptr::read(response.framebuffers) };
        if framebuffer_ptr.is_null() {
            return None;
        }
        // SAFETY: The first framebuffer pointer is non-null and bootloader-owned.
        let framebuffer = unsafe { ptr::read(framebuffer_ptr) };
        if framebuffer.address.is_null() {
            return None;
        }

        let pixel_format = match (
            framebuffer.red_mask_size,
            framebuffer.green_mask_size,
            framebuffer.blue_mask_size,
            framebuffer.red_mask_shift,
            framebuffer.green_mask_shift,
            framebuffer.blue_mask_shift,
        ) {
            (8, 8, 8, 0, 8, 16) => PixelFormat::Rgb,
            (8, 8, 8, 16, 8, 0) => PixelFormat::Bgr,
            _ => return None,
        };

        let info = FramebufferInfo {
            address: framebuffer.address as u64,
            width: usize::try_from(framebuffer.width).ok()?,
            height: usize::try_from(framebuffer.height).ok()?,
            pitch: usize::try_from(framebuffer.pitch).ok()?,
            bits_per_pixel: u8::try_from(framebuffer.bpp).ok()?,
            pixel_format,
        };
        info.validate().ok()?;
        Some((framebuffer.address, info))
    }

    /// Collect the responses written by a Limine-compatible bootloader.
    ///
    /// # Safety
    /// Must be called after protocol handoff, with the Limine request sections
    /// retained in the loaded image and the bootloader-provided mappings active.
    pub unsafe fn from_responses() -> Option<Self> {
        // SAFETY: Caller guarantees the bootloader has completed handoff.
        let hhdm_request = unsafe { ptr::read_volatile(ptr::addr_of!(LIMINE_HHDM_REQUEST.response)) };
        // SAFETY: Same handoff guarantee applies to the memory-map request.
        let memory_request = unsafe { ptr::read_volatile(ptr::addr_of!(LIMINE_MEMMAP_REQUEST.response)) };
        // SAFETY: Same handoff guarantee applies to the framebuffer request.
        let framebuffer_request = unsafe { ptr::read_volatile(ptr::addr_of!(LIMINE_FRAMEBUFFER_REQUEST.response)) };
        // ACPI is optional; a null response means firmware did not expose ACPI.
        let rsdp_request = unsafe { ptr::read_volatile(ptr::addr_of!(LIMINE_RSDP_REQUEST.response)) };

        if hhdm_request.is_null() || memory_request.is_null() {
            return None;
        }

        // A supported base revision is acknowledged by the bootloader by
        // clearing the third word of the revision tag.
        // SAFETY: The bootloader has completed handoff and the tag is retained.
        let base_revision_ack = unsafe {
            ptr::read_volatile(ptr::addr_of!(LIMINE_BASE_REVISION[2]))
        };
        if base_revision_ack != 0 {
            return None;
        }

        // SAFETY: Non-null response pointers are supplied by the bootloader.
        let hhdm = unsafe { ptr::read(hhdm_request) };
        // SAFETY: Same response contract applies to the memory-map response.
        let memory_map = unsafe { ptr::read(memory_request) };
        if hhdm.offset == 0 || memory_map.entry_count == 0 || memory_map.entries.is_null() {
            return None;
        }
        let memory_map_count = usize::try_from(memory_map.entry_count).ok()?;

        let rsdp_address = if rsdp_request.is_null() {
            ptr::null()
        } else {
            // SAFETY: Limine owns this response for the kernel lifetime.
            unsafe { ptr::read(rsdp_request) }.address
        };

        Some(Self {
            hhdm_offset: hhdm.offset,
            memory_map_entries: memory_map.entries,
            memory_map_count,
            framebuffer: framebuffer_request,
            rsdp_address,
        })
    }
}
