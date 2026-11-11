//! Kernel-side consumer for the Sovereign Boot v1 handoff ABI.

use core::ptr;

pub const BOOT_INFO_MAGIC: [u8; 8] = *b"SOVBOOT\0";
pub const BOOT_INFO_VERSION: u32 = 1;
pub const MEMORY_MAP_FORMAT_UEFI: u32 = 1;
pub const MEMORY_DESCRIPTOR_VERSION: u32 = 1;
pub const BOOT_INFO_FLAG_FRAMEBUFFER_VALID: u64 = 1 << 0;
pub const BOOT_INFO_FLAG_ACPI_VALID: u64 = 1 << 1;
const MAX_MEMORY_DESCRIPTORS: u64 = 1_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SovereignBootError {
    NullPointer, MisalignedPointer, InvalidMagic, UnsupportedVersion,
    InvalidStructureSize, UnknownFlags, InvalidMemoryMap, InvalidKernelRange,
    InvalidOptionalData, ReservedFieldNonZero, DescriptorOutOfRange,
}

/// Stable repr(C) mirror of loader/src/boot_info.rs.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SovereignBootInfo {
    pub magic: [u8; 8],
    pub version: u32,
    pub structure_size: u32,
    pub flags: u64,
    pub memory_map_physical: u64,
    pub memory_map_size: u64,
    pub memory_descriptor_size: u64,
    pub memory_descriptor_version: u32,
    pub memory_map_format: u32,
    pub kernel_physical_base: u64,
    pub kernel_virtual_base: u64,
    pub kernel_image_size: u64,
    pub kernel_entry: u64,
    pub acpi_rsdp_physical: u64,
    pub framebuffer_physical: u64,
    pub framebuffer_size: u64,
    pub framebuffer_width: u32,
    pub framebuffer_height: u32,
    pub framebuffer_pixels_per_scanline: u32,
    pub framebuffer_pixel_format: u32,
    pub reserved: [u64; 4],
}

impl SovereignBootInfo {
    /// Read and validate the handoff from its identity-mapped address.
    ///
    /// # Safety
    /// pointer must reference a readable SovereignBootInfo object for the
    /// duration of this call. Its physical buffers must remain mapped before
    /// methods that access them are used.
    pub unsafe fn from_ptr(pointer: *const Self) -> Result<Self, SovereignBootError> {
        if pointer.is_null() { return Err(SovereignBootError::NullPointer); }
        if (pointer as usize) & (core::mem::align_of::<Self>() - 1) != 0 {
            return Err(SovereignBootError::MisalignedPointer);
        }
        // SAFETY: The caller guarantees a readable ABI object at this address.
        let info = unsafe { ptr::read(pointer) };
        info.validate()?;
        Ok(info)
    }

    pub fn validate(&self) -> Result<(), SovereignBootError> {
        if self.magic != BOOT_INFO_MAGIC { return Err(SovereignBootError::InvalidMagic); }
        if self.version != BOOT_INFO_VERSION { return Err(SovereignBootError::UnsupportedVersion); }
        if self.structure_size as usize != core::mem::size_of::<Self>() {
            return Err(SovereignBootError::InvalidStructureSize);
        }
        let known_flags = BOOT_INFO_FLAG_FRAMEBUFFER_VALID | BOOT_INFO_FLAG_ACPI_VALID;
        if self.flags & !known_flags != 0 { return Err(SovereignBootError::UnknownFlags); }
        if self.memory_map_format != MEMORY_MAP_FORMAT_UEFI
            || self.memory_descriptor_version != MEMORY_DESCRIPTOR_VERSION
            || self.memory_descriptor_size < core::mem::size_of::<EfiMemoryDescriptor>() as u64
            || self.memory_map_size == 0
            || self.memory_map_size % self.memory_descriptor_size != 0
            || self.memory_map_size / self.memory_descriptor_size > MAX_MEMORY_DESCRIPTORS
            || self.memory_map_physical == 0
            || self.memory_map_physical & 0xfff != 0
            || self.memory_map_physical.checked_add(self.memory_map_size).is_none()
        {
            return Err(SovereignBootError::InvalidMemoryMap);
        }
        let kernel_end = self.kernel_virtual_base.checked_add(self.kernel_image_size)
            .ok_or(SovereignBootError::InvalidKernelRange)?;
        if self.kernel_physical_base == 0 || self.kernel_physical_base & 0xfff != 0
            || self.kernel_virtual_base & 0xfff != 0 || self.kernel_image_size == 0
            || self.kernel_image_size & 0xfff != 0
            || self.kernel_physical_base.checked_add(self.kernel_image_size).is_none()
            || !is_canonical(self.kernel_virtual_base) || !is_canonical(kernel_end - 1)
            || self.kernel_entry < self.kernel_virtual_base || self.kernel_entry >= kernel_end
            || !is_canonical(self.kernel_entry)
        {
            return Err(SovereignBootError::InvalidKernelRange);
        }
        if (self.flags & BOOT_INFO_FLAG_ACPI_VALID == 0 && self.acpi_rsdp_physical != 0)
            || (self.flags & BOOT_INFO_FLAG_ACPI_VALID != 0 && self.acpi_rsdp_physical == 0)
            || (self.flags & BOOT_INFO_FLAG_FRAMEBUFFER_VALID == 0
                && (self.framebuffer_physical != 0 || self.framebuffer_size != 0
                    || self.framebuffer_width != 0 || self.framebuffer_height != 0
                    || self.framebuffer_pixels_per_scanline != 0 || self.framebuffer_pixel_format != 0))
            || (self.flags & BOOT_INFO_FLAG_FRAMEBUFFER_VALID != 0
                && (self.framebuffer_physical == 0 || self.framebuffer_size == 0
                    || self.framebuffer_physical.checked_add(self.framebuffer_size).is_none()
                    || self.framebuffer_width == 0 || self.framebuffer_height == 0
                    || self.framebuffer_pixels_per_scanline < self.framebuffer_width))
        {
            return Err(SovereignBootError::InvalidOptionalData);
        }
        if self.reserved.iter().any(|value| *value != 0) {
            return Err(SovereignBootError::ReservedFieldNonZero);
        }
        Ok(())
    }

    /// Construct a strided view over the physical UEFI memory map.
    ///
    /// # Safety
    /// The complete physical memory-map range must be mapped at its identity
    /// address and remain readable for the returned view's lifetime.
    pub unsafe fn memory_map(&self) -> Result<EfiMemoryMap<'_>, SovereignBootError> {
        self.validate()?;
        Ok(EfiMemoryMap {
            base: self.memory_map_physical as usize as *const u8,
            byte_len: self.memory_map_size as usize,
            descriptor_stride: self.memory_descriptor_size as usize,
            _lifetime: core::marker::PhantomData,
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EfiMemoryDescriptor {
    pub memory_type: u32,
    _padding: u32,
    pub physical_start: u64,
    pub virtual_start: u64,
    pub number_of_pages: u64,
    pub attribute: u64,
}

impl EfiMemoryDescriptor {
    pub const fn new(
        memory_type: u32,
        physical_start: u64,
        virtual_start: u64,
        number_of_pages: u64,
        attribute: u64,
    ) -> Self {
        Self {
            memory_type,
            _padding: 0,
            physical_start,
            virtual_start,
            number_of_pages,
            attribute,
        }
    }
}

pub struct EfiMemoryMap<'a> {
    base: *const u8,
    byte_len: usize,
    descriptor_stride: usize,
    _lifetime: core::marker::PhantomData<&'a [u8]>,
}

impl<'a> EfiMemoryMap<'a> {
    #[cfg(test)]
    pub(crate) fn from_descriptors(descriptors: &'a [EfiMemoryDescriptor]) -> Self {
        Self {
            base: descriptors.as_ptr().cast::<u8>(),
            byte_len: core::mem::size_of_val(descriptors),
            descriptor_stride: core::mem::size_of::<EfiMemoryDescriptor>(),
            _lifetime: core::marker::PhantomData,
        }
    }
}

impl EfiMemoryMap<'_> {
    pub const fn len(&self) -> usize { self.byte_len / self.descriptor_stride }
    pub const fn is_empty(&self) -> bool { self.byte_len == 0 }

    /// Read one descriptor while respecting firmware-provided descriptor stride.
    pub fn descriptor(&self, index: usize) -> Result<EfiMemoryDescriptor, SovereignBootError> {
        if index >= self.len() { return Err(SovereignBootError::DescriptorOutOfRange); }
        let offset = index.checked_mul(self.descriptor_stride)
            .ok_or(SovereignBootError::DescriptorOutOfRange)?;
        let end = offset.checked_add(core::mem::size_of::<EfiMemoryDescriptor>())
            .ok_or(SovereignBootError::DescriptorOutOfRange)?;
        if end > self.byte_len { return Err(SovereignBootError::DescriptorOutOfRange); }
        // SAFETY: Constructor requires the complete map to be mapped; bounds
        // use the firmware stride and the fixed descriptor prefix size.
        Ok(unsafe { ptr::read_unaligned(self.base.add(offset).cast::<EfiMemoryDescriptor>()) })
    }
}

const fn is_canonical(address: u64) -> bool {
    let upper = address >> 48;
    let sign = (address >> 47) & 1;
    (sign == 0 && upper == 0) || (sign == 1 && upper == 0xffff)
}
// ABI v1 layout is fixed at 160 bytes with 8-byte alignment.
const _: [(); 160] = [(); core::mem::size_of::<SovereignBootInfo>()];
const _: [(); 8] = [(); core::mem::align_of::<SovereignBootInfo>()];

// UEFI descriptor prefix is 40 bytes; firmware may provide a larger stride.
const _: [(); 40] = [(); core::mem::size_of::<EfiMemoryDescriptor>()];
