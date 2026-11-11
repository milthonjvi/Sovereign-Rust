//! Kernel-side consumer for the Sovereign Boot v1 handoff ABI.

use core::ptr;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::drivers::acpi::discover::AcpiTables;
use crate::drivers::acpi::topology::MadtInterruptTopology;

pub const BOOT_INFO_MAGIC: [u8; 8] = *b"SOVBOOT\0";
pub const BOOT_INFO_VERSION: u32 = 1;
pub const MEMORY_MAP_FORMAT_UEFI: u32 = 1;
pub const MEMORY_DESCRIPTOR_VERSION: u32 = 1;
pub const BOOT_INFO_FLAG_FRAMEBUFFER_VALID: u64 = 1 << 0;
pub const BOOT_INFO_FLAG_ACPI_VALID: u64 = 1 << 1;
const MAX_MEMORY_DESCRIPTORS: u64 = 1_000_000;
const MAX_PHYSICAL_ADDRESS: u64 = 1 << 52;

static BOOT_INFO_READY: AtomicBool = AtomicBool::new(false);
static mut BOOT_INFO: Option<SovereignBootInfo> = None;

static ACPI_TABLES_READY: AtomicBool = AtomicBool::new(false);
static mut ACPI_TABLES: Option<AcpiTables> = None;
static MADT_TOPOLOGY_READY: AtomicBool = AtomicBool::new(false);
static mut MADT_TOPOLOGY: core::mem::MaybeUninit<MadtInterruptTopology> =
    core::mem::MaybeUninit::uninit();

/// Publish the validated Sovereign Boot handoff for later kernel subsystems.
///
/// # Safety
/// Call exactly once on the bootstrap processor after validating the handoff
/// and its identity mappings, with interrupts disabled. The boot-info and
/// memory-map pages must remain mapped and reserved for the kernel lifetime.
pub unsafe fn install_boot_info(info: SovereignBootInfo) -> bool {
    if BOOT_INFO_READY.load(Ordering::Acquire) {
        return false;
    }
    // SAFETY: The documented bootstrap contract guarantees exclusive access.
    unsafe { ptr::addr_of_mut!(BOOT_INFO).write(Some(info)) };
    BOOT_INFO_READY.store(true, Ordering::Release);
    true
}

/// Borrow the immutable, validated boot handoff.
pub fn boot_info() -> Option<&'static SovereignBootInfo> {
    if !BOOT_INFO_READY.load(Ordering::Acquire) {
        return None;
    }
    // SAFETY: Release/Acquire publication makes the immutable value visible.
    unsafe { (&*ptr::addr_of!(BOOT_INFO)).as_ref() }
}

/// Reconstruct a borrowed view of the identity-mapped UEFI memory map.
///
/// The memory manager must copy any metadata it needs before reclaiming the
/// loader-owned memory-map buffer.
pub fn memory_map() -> Option<EfiMemoryMap<'static>> {
    let info = boot_info()?;
    // SAFETY: install_boot_info is called only after validating and mapping the
    // complete memory-map buffer; the documented contract retains that buffer.
    unsafe { info.memory_map().ok() }
}

/// Publish validated ACPI table addresses for later kernel subsystems.
///
/// # Safety
/// Call exactly once during single-CPU bootstrap with interrupts disabled.
/// Readers may use the value after this function publishes readiness.
pub unsafe fn install_acpi_tables(tables: AcpiTables) -> bool {
    if ACPI_TABLES_READY.load(Ordering::Acquire) {
        return false;
    }
    // SAFETY: The documented bootstrap contract guarantees exclusive access.
    unsafe { ptr::addr_of_mut!(ACPI_TABLES).write(Some(tables)) };
    ACPI_TABLES_READY.store(true, Ordering::Release);
    true
}


/// Publish the validated MADT interrupt topology for the interrupt subsystem.
///
/// # Safety
/// Call exactly once during single-CPU bootstrap with interrupts disabled.
/// The topology is immutable after publication and remains valid forever.
pub unsafe fn install_madt_topology(topology: MadtInterruptTopology) -> bool {
    if MADT_TOPOLOGY_READY.load(Ordering::Acquire) {
        return false;
    }
    // SAFETY: The documented bootstrap contract guarantees exclusive access.
    unsafe {
        ptr::addr_of_mut!(MADT_TOPOLOGY).write(core::mem::MaybeUninit::new(topology));
    }
    MADT_TOPOLOGY_READY.store(true, Ordering::Release);
    true
}

/// Borrow the immutable MADT topology discovered during UEFI boot.
pub fn madt_topology() -> Option<&'static MadtInterruptTopology> {
    if !MADT_TOPOLOGY_READY.load(Ordering::Acquire) {
        return None;
    }
    // SAFETY: The Acquire load observes the fully initialized immutable value.
    let topology = unsafe {
        &*ptr::addr_of!(MADT_TOPOLOGY).cast::<MadtInterruptTopology>()
    };
    Some(topology)
}

/// Return the validated ACPI table address set, if firmware discovery succeeded.
pub fn acpi_tables() -> Option<AcpiTables> {
    if !ACPI_TABLES_READY.load(Ordering::Acquire) {
        return None;
    }
    // SAFETY: The Release/Acquire publication makes the immutable value visible.
    unsafe { ptr::addr_of!(ACPI_TABLES).read() }
}


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
            || self.memory_descriptor_size & 7 != 0
            || self.memory_map_size == 0
            || self.memory_map_size % self.memory_descriptor_size != 0
            || self.memory_map_size / self.memory_descriptor_size > MAX_MEMORY_DESCRIPTORS
            || self.memory_map_physical == 0
            || self.memory_map_physical & 0xfff != 0
            || self.memory_map_physical.checked_add(self.memory_map_size).is_none()
            || self.memory_map_physical >= MAX_PHYSICAL_ADDRESS
            || self.memory_map_size > MAX_PHYSICAL_ADDRESS - self.memory_map_physical
        {
            return Err(SovereignBootError::InvalidMemoryMap);
        }
        let kernel_end = self.kernel_virtual_base.checked_add(self.kernel_image_size)
            .ok_or(SovereignBootError::InvalidKernelRange)?;
        if self.kernel_physical_base == 0 || self.kernel_physical_base & 0xfff != 0
            || self.kernel_virtual_base & 0xfff != 0 || self.kernel_image_size == 0
            || self.kernel_image_size & 0xfff != 0
            || self.kernel_physical_base.checked_add(self.kernel_image_size).is_none()
            || self.kernel_physical_base >= MAX_PHYSICAL_ADDRESS
            || self.kernel_image_size > MAX_PHYSICAL_ADDRESS - self.kernel_physical_base
            || !is_canonical(self.kernel_virtual_base) || !is_canonical(kernel_end - 1)
            || self.kernel_entry < self.kernel_virtual_base || self.kernel_entry >= kernel_end
            || !is_canonical(self.kernel_entry)
        {
            return Err(SovereignBootError::InvalidKernelRange);
        }
        if (self.flags & BOOT_INFO_FLAG_ACPI_VALID == 0 && self.acpi_rsdp_physical != 0)
            || (self.flags & BOOT_INFO_FLAG_ACPI_VALID != 0
                && (self.acpi_rsdp_physical == 0
                    || self.acpi_rsdp_physical & !0x000F_FFFF_FFFF_FFFF != 0))
            || (self.flags & BOOT_INFO_FLAG_FRAMEBUFFER_VALID == 0
                && (self.framebuffer_physical != 0 || self.framebuffer_size != 0
                    || self.framebuffer_width != 0 || self.framebuffer_height != 0
                    || self.framebuffer_pixels_per_scanline != 0 || self.framebuffer_pixel_format != 0))
            || (self.flags & BOOT_INFO_FLAG_FRAMEBUFFER_VALID != 0
                && (self.framebuffer_physical == 0 || self.framebuffer_size == 0
                    || self.framebuffer_physical.checked_add(self.framebuffer_size).is_none()
                    || self.framebuffer_physical >= MAX_PHYSICAL_ADDRESS
                    || self.framebuffer_size > MAX_PHYSICAL_ADDRESS - self.framebuffer_physical
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

    /// Check whether one physical byte range is wholly contained in a
    /// descriptor of the requested UEFI memory type.
    ///
    /// This is used to verify that boot-owned buffers and the loaded kernel
    /// image still belong to EFI_LOADER_DATA before the kernel adopts them.
    pub fn contains_range_with_type(
        &self,
        physical_start: u64,
        byte_len: u64,
        memory_type: u32,
    ) -> Result<bool, SovereignBootError> {
        if byte_len == 0 {
            return Ok(false);
        }
        let requested_end = physical_start.checked_add(byte_len)
            .ok_or(SovereignBootError::InvalidMemoryMap)?;
        for index in 0..self.len() {
            let descriptor = self.descriptor(index)?;
            let descriptor_bytes = descriptor.number_of_pages.checked_mul(4096)
                .ok_or(SovereignBootError::InvalidMemoryMap)?;
            let descriptor_end = descriptor.physical_start.checked_add(descriptor_bytes)
                .ok_or(SovereignBootError::InvalidMemoryMap)?;
            if descriptor.memory_type == memory_type
                && physical_start >= descriptor.physical_start
                && requested_end <= descriptor_end
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

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
        let descriptor = unsafe {
            ptr::read_unaligned(self.base.add(offset).cast::<EfiMemoryDescriptor>())
        };
        let descriptor_bytes = descriptor.number_of_pages.checked_mul(4096)
            .ok_or(SovereignBootError::InvalidMemoryMap)?;
        if descriptor.number_of_pages == 0
            || descriptor.physical_start & 0xfff != 0
            || descriptor.physical_start >= MAX_PHYSICAL_ADDRESS
            || descriptor_bytes > MAX_PHYSICAL_ADDRESS - descriptor.physical_start
        {
            return Err(SovereignBootError::InvalidMemoryMap);
        }
        Ok(descriptor)
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


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_range_must_fit_one_descriptor_of_requested_type() {
        let descriptors = [
            EfiMemoryDescriptor::new(2, 0x1000, 0, 4, 0),
            EfiMemoryDescriptor::new(7, 0x5000, 0, 8, 0),
        ];
        let map = EfiMemoryMap::from_descriptors(&descriptors);
        assert_eq!(map.contains_range_with_type(0x2000, 0x2000, 2), Ok(true));
        assert_eq!(map.contains_range_with_type(0x1000, 0x1000, 7), Ok(false));
        assert_eq!(map.contains_range_with_type(0x4000, 0x2000, 2), Ok(false));
        assert_eq!(map.contains_range_with_type(0x1000, 0, 2), Ok(false));
    }

    #[test]
    fn descriptor_rejects_zero_pages_and_physical_overflow() {
        let zero_pages = [EfiMemoryDescriptor::new(2, 0x1000, 0, 0, 0)];
        let map = EfiMemoryMap::from_descriptors(&zero_pages);
        assert_eq!(map.descriptor(0), Err(SovereignBootError::InvalidMemoryMap));

        let overflow = [EfiMemoryDescriptor::new(
            2,
            MAX_PHYSICAL_ADDRESS - 4096,
            0,
            2,
            0,
        )];
        let map = EfiMemoryMap::from_descriptors(&overflow);
        assert_eq!(map.descriptor(0), Err(SovereignBootError::InvalidMemoryMap));
    }

    #[test]
    fn physical_range_rejects_overflow() {
        let descriptors = [EfiMemoryDescriptor::new(2, 0x1000, 0, 4, 0)];
        let map = EfiMemoryMap::from_descriptors(&descriptors);
        assert_eq!(
            map.contains_range_with_type(u64::MAX - 7, 16, 2),
            Err(SovereignBootError::InvalidMemoryMap),
        );
    }
}
