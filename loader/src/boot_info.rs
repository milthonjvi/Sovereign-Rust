//! Stable, versioned handoff contract between Sovereign Boot and the kernel.
//!
//! The structure contains physical addresses rather than Rust references because
//! the kernel may enter with a different virtual-address space. All pointed-to
//! buffers must remain resident after ExitBootServices.

/// Eight-byte identifier: "SOVBOOT\0".
pub const BOOT_INFO_MAGIC: [u8; 8] = *b"SOVBOOT\0";

/// Initial handoff ABI version.
pub const BOOT_INFO_VERSION: u32 = 1;

/// The memory map uses the firmware EFI_MEMORY_DESCRIPTOR layout.
pub const MEMORY_MAP_FORMAT_UEFI: u32 = 1;
pub const MEMORY_DESCRIPTOR_VERSION_UEFI: u32 = 1;

/// No framebuffer information is currently provided.
pub const BOOT_INFO_FLAG_FRAMEBUFFER_VALID: u64 = 1 << 0;

/// A validated UEFI ACPI RSDP physical address is provided when available.
pub const BOOT_INFO_FLAG_ACPI_VALID: u64 = 1 << 1;

/// Versioned boot handoff passed in the first argument register (RDI).
///
/// This is an ABI structure, not an ownership-bearing Rust type. The kernel
/// must validate every field and range before dereferencing any physical
/// address. Reserved fields must be zero in version 1.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SovereignBootInfo {
    pub magic: [u8; 8],
    pub version: u32,
    pub structure_size: u32,
    pub flags: u64,

    /// Physical base address of the UEFI memory-map buffer.
    pub memory_map_physical: u64,
    /// Number of valid bytes in the memory-map buffer.
    pub memory_map_size: u64,
    /// Stride in bytes between firmware memory descriptors.
    pub memory_descriptor_size: u64,
    /// UEFI memory descriptor revision.
    pub memory_descriptor_version: u32,
    pub memory_map_format: u32,

    /// Physical base of the loaded kernel image, including reserved padding.
    pub kernel_physical_base: u64,
    /// Virtual base expected by the kernel image.
    pub kernel_virtual_base: u64,
    /// Span of the loaded kernel image in bytes.
    pub kernel_image_size: u64,
    /// Virtual entry address to which control will be transferred.
    pub kernel_entry: u64,

    /// Physical address of the ACPI RSDP, when discovered.
    pub acpi_rsdp_physical: u64,
    /// Physical address of the framebuffer, when supplied.
    pub framebuffer_physical: u64,
    pub framebuffer_size: u64,
    pub framebuffer_width: u32,
    pub framebuffer_height: u32,
    pub framebuffer_pixels_per_scanline: u32,
    pub framebuffer_pixel_format: u32,

    /// Reserved for ABI-compatible extension. Must be zero in version 1.
    pub reserved: [u64; 4],
}

impl SovereignBootInfo {
    /// Validate fields that can be checked without dereferencing physical memory.
    pub const fn is_well_formed(&self) -> bool {
        self.magic[0] == BOOT_INFO_MAGIC[0]
            && self.magic[1] == BOOT_INFO_MAGIC[1]
            && self.magic[2] == BOOT_INFO_MAGIC[2]
            && self.magic[3] == BOOT_INFO_MAGIC[3]
            && self.magic[4] == BOOT_INFO_MAGIC[4]
            && self.magic[5] == BOOT_INFO_MAGIC[5]
            && self.magic[6] == BOOT_INFO_MAGIC[6]
            && self.magic[7] == BOOT_INFO_MAGIC[7]
            && self.version == BOOT_INFO_VERSION
            && self.structure_size as usize == core::mem::size_of::<Self>()
            && self.flags & !(BOOT_INFO_FLAG_FRAMEBUFFER_VALID | BOOT_INFO_FLAG_ACPI_VALID) == 0
            && self.memory_map_format == MEMORY_MAP_FORMAT_UEFI
            && self.memory_descriptor_size >= 40
            && self.memory_descriptor_version == MEMORY_DESCRIPTOR_VERSION_UEFI
            && self.memory_map_size != 0
            && self.memory_map_size % self.memory_descriptor_size == 0
            && self.memory_map_physical != 0
            && self.memory_map_physical & 0xfff == 0
            && self.memory_map_physical.checked_add(self.memory_map_size).is_some()
            && self.kernel_physical_base != 0
            && self.kernel_physical_base & 0xfff == 0
            && self.kernel_virtual_base != 0
            && self.kernel_virtual_base & 0xfff == 0
            && self.kernel_image_size != 0
            && self.kernel_image_size & 0xfff == 0
            && self.kernel_physical_base.checked_add(self.kernel_image_size).is_some()
            && self.kernel_virtual_base.checked_add(self.kernel_image_size).is_some()
            && self.kernel_entry >= self.kernel_virtual_base
            && self.kernel_entry < self.kernel_virtual_base + self.kernel_image_size
            && is_canonical_x86_64(self.kernel_virtual_base)
            && is_canonical_x86_64(self.kernel_entry)
            && is_canonical_x86_64(self.kernel_virtual_base + self.kernel_image_size - 1)
            && (self.flags & BOOT_INFO_FLAG_ACPI_VALID != 0 || self.acpi_rsdp_physical == 0)
            && (self.flags & BOOT_INFO_FLAG_FRAMEBUFFER_VALID != 0
                || (self.framebuffer_physical == 0 && self.framebuffer_size == 0
                    && self.framebuffer_width == 0 && self.framebuffer_height == 0
                    && self.framebuffer_pixels_per_scanline == 0 && self.framebuffer_pixel_format == 0))
            && (self.flags & BOOT_INFO_FLAG_ACPI_VALID == 0
                || (self.acpi_rsdp_physical != 0
                    && self.acpi_rsdp_physical & !0x000F_FFFF_FFFF_FFFF == 0))
            && (self.flags & BOOT_INFO_FLAG_FRAMEBUFFER_VALID == 0
                || (self.framebuffer_physical != 0 && self.framebuffer_size != 0
                    && self.framebuffer_physical.checked_add(self.framebuffer_size).is_some()
                    && self.framebuffer_width != 0 && self.framebuffer_height != 0
                    && self.framebuffer_pixels_per_scanline >= self.framebuffer_width))
            && self.reserved[0] == 0
            && self.reserved[1] == 0
            && self.reserved[2] == 0
            && self.reserved[3] == 0
    }
}

const fn is_canonical_x86_64(address: u64) -> bool {
    let upper = address >> 48;
    let sign = (address >> 47) & 1;
    (sign == 0 && upper == 0) || (sign == 1 && upper == 0xffff)
}

// ABI v1 layout is fixed at 160 bytes with 8-byte alignment.
const _: [(); 160] = [(); core::mem::size_of::<SovereignBootInfo>()];
const _: [(); 8] = [(); core::mem::align_of::<SovereignBootInfo>()];
