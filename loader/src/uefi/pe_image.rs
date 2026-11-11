//! PE32+ image section inspection for permission-correct UEFI runtime mappings.

use core::ptr;

use crate::page_tables::PagePermissions;

const DOS_MAGIC: u16 = 0x5a4d;
const PE_MAGIC: u32 = 0x0000_4550;
const PE32_PLUS_MAGIC: u16 = 0x020b;
const MAX_SECTIONS: usize = 96;
const PAGE_SIZE: u64 = 4096;
const IMAGE_SCN_MEM_EXECUTE: u32 = 0x2000_0000;
const IMAGE_SCN_MEM_READ: u32 = 0x4000_0000;
const IMAGE_SCN_MEM_WRITE: u32 = 0x8000_0000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeImageError {
    InvalidImage, UnsupportedImage, OutOfBounds, InvalidAlignment,
    TooManySections, OverlappingSections, WritableExecutableSection,
}

#[derive(Clone, Copy, Debug)]
pub struct PeImageSection {
    pub relative_address: u64,
    pub size: u64,
    pub permissions: PagePermissions,
}

impl PeImageSection {
    const EMPTY: Self = Self {
        relative_address: 0,
        size: 0,
        permissions: PagePermissions::KERNEL_READ_ONLY,
    };
}

#[derive(Clone, Copy, Debug)]
pub struct PeImageLayout {
    pub headers_size: u64,
    pub image_size: u64,
    pub sections: [PeImageSection; MAX_SECTIONS],
    pub section_count: usize,
}

/// Parse a firmware-loaded PE32+ image already resident at image_base.
///
/// # Safety
/// image_base must point to image_size readable bytes in the current firmware
/// address space for the duration of this call.
pub unsafe fn inspect_loaded_image(
    image_base: u64,
    image_size: u64,
) -> Result<PeImageLayout, PeImageError> {
    if image_base == 0 || image_size < 0x100 || image_base & (PAGE_SIZE - 1) != 0 {
        return Err(PeImageError::InvalidImage);
    }
    let base = image_base as usize as *const u8;
    let dos = read_u16(base, image_size, 0)?;
    if dos != DOS_MAGIC { return Err(PeImageError::InvalidImage); }
    let pe_offset = read_u32(base, image_size, 0x3c)? as u64;
    if read_u32(base, image_size, pe_offset)? != PE_MAGIC {
        return Err(PeImageError::InvalidImage);
    }
    let coff = pe_offset.checked_add(4).ok_or(PeImageError::OutOfBounds)?;
    let section_count = read_u16(base, image_size, coff + 2)? as usize;
    let optional_size = read_u16(base, image_size, coff + 16)? as u64;
    if section_count == 0 || section_count > MAX_SECTIONS {
        return Err(PeImageError::TooManySections);
    }
    let optional = coff.checked_add(20).ok_or(PeImageError::OutOfBounds)?;
    let optional_end = optional.checked_add(optional_size).ok_or(PeImageError::OutOfBounds)?;
    if optional_size < 64 || optional_end > image_size {
        return Err(PeImageError::OutOfBounds);
    }
    if read_u16(base, image_size, optional)? != PE32_PLUS_MAGIC {
        return Err(PeImageError::UnsupportedImage);
    }
    let section_alignment = read_u32(base, image_size, optional + 32)? as u64;
    let size_of_image = read_u32(base, image_size, optional + 56)? as u64;
    let size_of_headers = read_u32(base, image_size, optional + 60)? as u64;
    if section_alignment < PAGE_SIZE || !section_alignment.is_power_of_two()
        || size_of_image == 0 || size_of_image > image_size
        || size_of_headers == 0 || size_of_headers > size_of_image
    {
        return Err(PeImageError::InvalidAlignment);
    }
    let table_size = (section_count as u64).checked_mul(40).ok_or(PeImageError::OutOfBounds)?;
    let table_end = optional_end.checked_add(table_size).ok_or(PeImageError::OutOfBounds)?;
    if table_end > size_of_headers || table_end > image_size {
        return Err(PeImageError::OutOfBounds);
    }

    let mut sections = [PeImageSection::EMPTY; MAX_SECTIONS];
    let mut count = 0usize;
    for index in 0..section_count {
        let header = optional_end + (index as u64) * 40;
        let virtual_size = read_u32(base, image_size, header + 8)? as u64;
        let relative_address = read_u32(base, image_size, header + 12)? as u64;
        let raw_size = read_u32(base, image_size, header + 16)? as u64;
        let characteristics = read_u32(base, image_size, header + 36)?;
        let size = virtual_size.max(raw_size);
        if size == 0 { continue; }
        if relative_address % section_alignment != 0 {
            return Err(PeImageError::InvalidAlignment);
        }
        let end = relative_address.checked_add(size).ok_or(PeImageError::OutOfBounds)?;
        if end > size_of_image { return Err(PeImageError::OutOfBounds); }
        let readable = characteristics & IMAGE_SCN_MEM_READ != 0;
        let writable = characteristics & IMAGE_SCN_MEM_WRITE != 0;
        let executable = characteristics & IMAGE_SCN_MEM_EXECUTE != 0;
        if !readable { return Err(PeImageError::UnsupportedImage); }
        if writable && executable {
            return Err(PeImageError::WritableExecutableSection);
        }
        let permissions = PagePermissions {
            writable, executable, user_accessible: false,
        };
        let page_start = relative_address & !(PAGE_SIZE - 1);
        let page_end = align_up(end).ok_or(PeImageError::OutOfBounds)?;
        for previous in sections.iter().take(count) {
            let previous_start = previous.relative_address & !(PAGE_SIZE - 1);
            let previous_end = align_up(previous.relative_address + previous.size)
                .ok_or(PeImageError::OutOfBounds)?;
            if page_start < previous_end && previous_start < page_end {
                return Err(PeImageError::OverlappingSections);
            }
        }
        sections[count] = PeImageSection { relative_address, size, permissions };
        count += 1;
    }
    if count == 0 { return Err(PeImageError::InvalidImage); }
    Ok(PeImageLayout {
        headers_size: size_of_headers,
        image_size: size_of_image,
        sections,
        section_count: count,
    })
}

fn read_u16(base: *const u8, image_size: u64, offset: u64) -> Result<u16, PeImageError> {
    if offset.checked_add(2).filter(|end| *end <= image_size).is_none() {
        return Err(PeImageError::OutOfBounds);
    }
    // SAFETY: Bounds were checked and unaligned reads are permitted.
    Ok(unsafe { ptr::read_unaligned(base.add(offset as usize).cast::<u16>()) })
}

fn read_u32(base: *const u8, image_size: u64, offset: u64) -> Result<u32, PeImageError> {
    if offset.checked_add(4).filter(|end| *end <= image_size).is_none() {
        return Err(PeImageError::OutOfBounds);
    }
    // SAFETY: Bounds were checked and unaligned reads are permitted.
    Ok(unsafe { ptr::read_unaligned(base.add(offset as usize).cast::<u32>()) })
}

fn align_up(value: u64) -> Option<u64> {
    value.checked_add(PAGE_SIZE - 1).map(|rounded| rounded & !(PAGE_SIZE - 1))
}