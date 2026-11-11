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


    /// Discover ACPI tables using Limine's HHDM and memory map.
    ///
    /// Each table is accepted only when its complete range lies inside a
    /// reported memory-map entry.
    ///
    /// # Safety
    /// Limine HHDM mappings and memory-map response storage must remain valid.
    pub unsafe fn discover_acpi(
        &self,
        rsdp: &Rsdp,
    ) -> Result<crate::drivers::acpi::discover::AcpiTables, AcpiError> {
        crate::drivers::acpi::discover::AcpiTables::discover_with_mappers(
            rsdp,
            |physical| {
                // SAFETY: The caller guarantees that Limine mappings remain active.
                unsafe { self.map_acpi_table(physical) }
            },
            |physical| {
                // SAFETY: The caller guarantees that Limine mappings remain active.
                unsafe { self.map_acpi_facs(physical) }
            },
        )
    }

    /// Map and validate one physical ACPI SDT through the HHDM.
    ///
    /// # Safety
    /// HHDM and memory-map response pointers must remain valid.
    pub unsafe fn map_acpi_table(&self, physical: u64) -> Option<&'static [u8]> {
        if physical == 0 || self.memory_map_entries.is_null() || self.hhdm_offset == 0 {
            return None;
        }
        if self.memory_map_count > 4096 {
            return None;
        }

        let mut containing_end = None;
        for index in 0..self.memory_map_count {
            // SAFETY: Limine supplied an array with memory_map_count pointers.
            let entry_ptr = unsafe { ptr::read(self.memory_map_entries.add(index)) };
            if entry_ptr.is_null() {
                continue;
            }
            // SAFETY: Each non-null entry pointer is owned by the boot response.
            let entry = unsafe { ptr::read(entry_ptr) };
            let Some(end) = entry.base.checked_add(entry.length) else {
                continue;
            };
            let header_end = physical.checked_add(36)?;
            if entry.kind != 4 && entry.kind != 7
                && physical >= entry.base && header_end <= end
            {
                containing_end = Some(end);
                break;
            }
        }
        let region_end = containing_end?;
        let virtual_address = self.hhdm_offset.checked_add(physical)?;
        let virtual_ptr = usize::try_from(virtual_address).ok()? as *const u8;

        // SAFETY: The SDT header lies inside the validated HHDM memory range.
        let length_ptr = unsafe { virtual_ptr.add(4).cast::<u32>() };
        // SAFETY: ACPI SDT length is a little-endian u32 at offset 4.
        let length = unsafe { ptr::read_unaligned(length_ptr) } as usize;
        if !(36..=16 * 1024 * 1024).contains(&length) {
            return None;
        }
        let table_end = physical.checked_add(length as u64)?;
        if table_end > region_end {
            return None;
        }

        // SAFETY: The complete table range was checked against a Limine memory
        // map entry and translated through the active HHDM.
        Some(unsafe { core::slice::from_raw_parts(virtual_ptr, length) })
    }

    /// Map a FACS through the HHDM using its distinct 64-byte header contract.
    ///
    /// Unlike an SDT, FACS has no checksum and its minimum header is 64 bytes.
    /// The physical address must also satisfy the ACPI FACS 64-byte alignment.
    ///
    /// # Safety
    /// HHDM and memory-map response pointers must remain valid, and the HHDM
    /// must map the complete accepted physical range for the returned lifetime.
    pub unsafe fn map_acpi_facs(&self, physical: u64) -> Option<&'static [u8]> {
        const FACS_MIN_LENGTH: usize = 64;
        const MAX_FACS_LENGTH: usize = 16 * 1024 * 1024;

        if physical == 0
            || physical % 64 != 0
            || self.memory_map_entries.is_null()
            || self.hhdm_offset == 0
            || self.memory_map_count > 4096
        {
            return None;
        }

        let header_end = physical.checked_add(8)?;
        let mut containing_end = None;
        for index in 0..self.memory_map_count {
            // SAFETY: Limine supplied an array with memory_map_count pointers.
            let entry_ptr = unsafe { ptr::read(self.memory_map_entries.add(index)) };
            if entry_ptr.is_null() {
                continue;
            }
            // SAFETY: Each non-null entry pointer is bootloader-owned.
            let entry = unsafe { ptr::read(entry_ptr) };
            let Some(end) = entry.base.checked_add(entry.length) else {
                continue;
            };
            if entry.kind != 4
                && entry.kind != 7
                && physical >= entry.base
                && header_end <= end
            {
                containing_end = Some(end);
                break;
            }
        }
        let region_end = containing_end?;
        let virtual_address = self.hhdm_offset.checked_add(physical)?;
        let virtual_ptr = usize::try_from(virtual_address).ok()? as *const u8;

        // SAFETY: The first eight bytes lie inside the validated memory-map entry.
        let signature = unsafe { ptr::read_unaligned(virtual_ptr.cast::<u32>()) };
        if signature != u32::from_le_bytes(*b"FACS") {
            return None;
        }
        // SAFETY: FACS length is an unaligned little-endian u32 at offset 4.
        let length = unsafe { ptr::read_unaligned(virtual_ptr.add(4).cast::<u32>()) } as usize;
        if !(FACS_MIN_LENGTH..=MAX_FACS_LENGTH).contains(&length) {
            return None;
        }
        let facs_end = physical.checked_add(length as u64)?;
        if facs_end > region_end {
            return None;
        }

        // SAFETY: The complete FACS range was checked against one memory-map
        // entry and translated through the active HHDM.
        Some(unsafe { core::slice::from_raw_parts(virtual_ptr, length) })
    }

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
