//! xAPIC local interrupt controller support.
//!
//! This module deliberately supports memory-mapped xAPIC mode only. It does
//! not allocate/map physical pages: the caller must establish an uncached,
//! writable MMIO mapping before constructing `LocalApic`. x2APIC mode is
//! detected and rejected rather than silently misprogrammed.

use core::ptr::{NonNull, read_volatile, write_volatile};

use super::{cpuid::CpuInfo, msr};

pub const IA32_APIC_BASE: u32 = 0x1B;
pub const APIC_BASE_BSP: u64 = 1 << 8;
pub const APIC_BASE_X2APIC: u64 = 1 << 10;
pub const APIC_BASE_GLOBAL_ENABLE: u64 = 1 << 11;
pub const APIC_BASE_ADDRESS_MASK: u64 = 0x000F_FFFF_FFFF_F000;

pub const REG_ID: usize = 0x020;
pub const REG_VERSION: usize = 0x030;
pub const REG_TPR: usize = 0x080;
pub const REG_EOI: usize = 0x0B0;
pub const REG_SVR: usize = 0x0F0;
pub const REG_ESR: usize = 0x280;
pub const REG_LVT_CMCI: usize = 0x2F0;
pub const REG_LVT_TIMER: usize = 0x320;
pub const REG_LVT_THERMAL: usize = 0x330;
pub const REG_LVT_PERFORMANCE: usize = 0x340;
pub const REG_LVT_LINT0: usize = 0x350;
pub const REG_LVT_LINT1: usize = 0x360;
pub const REG_LVT_ERROR: usize = 0x370;

pub const SPURIOUS_VECTOR: u8 = 0xFF;
const LVT_MASKED: u32 = 1 << 16;
const SVR_SOFTWARE_ENABLE: u32 = 1 << 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApicError {
    FeatureUnavailable,
    X2ApicModeActive,
    InvalidPhysicalBase,
    InvalidMapping,
    InvalidVector,
}

/// Read and enable the architectural xAPIC base MSR.
///
/// This leaves x2APIC disabled and preserves the firmware-selected physical
/// base and all unrelated defined bits.
///
/// # Safety
/// Must execute at CPL 0 on a CPU supporting the APIC MSR. Changing this MSR
/// affects the current logical processor and assumes the APIC MMIO mapping is
/// managed separately by the caller.
pub unsafe fn enable_xapic() -> Result<u64, ApicError> {
    if !CpuInfo::detect().features.apic {
        return Err(ApicError::FeatureUnavailable);
    }

    // SAFETY: CPUID reports the architectural APIC feature; caller is ring 0.
    let current = unsafe { msr::read(IA32_APIC_BASE) };
    if current & APIC_BASE_X2APIC != 0 {
        return Err(ApicError::X2ApicModeActive);
    }
    let physical_base = current & APIC_BASE_ADDRESS_MASK;
    if physical_base == 0 || physical_base & 0xFFF != 0 {
        return Err(ApicError::InvalidPhysicalBase);
    }
    let enabled = current | APIC_BASE_GLOBAL_ENABLE;
    if enabled != current {
        // SAFETY: Preserve the firmware base and only set global APIC enable.
        unsafe { msr::write(IA32_APIC_BASE, enabled) };
    }
    Ok(physical_base)
}

/// A handle to the current CPU's mapped xAPIC register page.
///
/// # Safety
/// The pointer must refer to the full 4-KiB xAPIC MMIO page, mapped writable
/// and uncached (or with the architecturally required UC-equivalent attributes).
/// The mapping must remain valid and access must be serialized per CPU.
pub struct LocalApic {
    base: NonNull<u8>,
}

impl LocalApic {
    /// Construct a handle from an already established MMIO mapping.
    ///
    /// # Safety
    /// The caller must uphold the mapping and cacheability contract on the type.
    pub unsafe fn new(base: NonNull<u8>) -> Self {
        Self { base }
    }

    #[inline]
    fn read(&self, offset: usize) -> u32 {
        // SAFETY: Constructor contract guarantees the register page is mapped.
        unsafe { read_volatile(self.base.as_ptr().add(offset).cast::<u32>()) }
    }

    #[inline]
    fn write(&mut self, offset: usize, value: u32) {
        // SAFETY: Constructor contract guarantees writable MMIO; &mut serializes.
        unsafe { write_volatile(self.base.as_ptr().add(offset).cast::<u32>(), value) }
    }

    pub fn id(&self) -> u8 {
        (self.read(REG_ID) >> 24) as u8
    }

    pub fn version(&self) -> u32 {
        self.read(REG_VERSION)
    }

    pub fn max_lvt_entry(&self) -> u8 {
        ((self.version() >> 16) & 0xFF) as u8
    }

    /// Configure the current CPU's local APIC while keeping all LVT sources
    /// masked. The spurious vector must be reserved in the IDT beforehand.
    pub fn initialize_masked(&mut self, spurious_vector: u8) -> Result<(), ApicError> {
        if spurious_vector < 32 {
            return Err(ApicError::InvalidVector);
        }

        self.write(REG_TPR, 0);
        self.write(REG_ESR, 0);
        let _ = self.read(REG_ESR);

        // Mask legacy and optional LVT entries according to the APIC version.
        self.write(REG_LVT_TIMER, LVT_MASKED);
        self.write(REG_LVT_THERMAL, LVT_MASKED);
        self.write(REG_LVT_PERFORMANCE, LVT_MASKED);
        self.write(REG_LVT_LINT0, LVT_MASKED);
        self.write(REG_LVT_LINT1, LVT_MASKED);
        self.write(REG_LVT_ERROR, LVT_MASKED);
        if self.max_lvt_entry() >= 6 {
            self.write(REG_LVT_CMCI, LVT_MASKED);
        }

        self.write(REG_SVR, (spurious_vector as u32) | SVR_SOFTWARE_ENABLE);
        Ok(())
    }

    /// Signal end-of-interrupt for the current local APIC interrupt.
    pub fn end_of_interrupt(&mut self) {
        self.write(REG_EOI, 0);
    }

    /// Mask all implemented local-vector sources without disabling the APIC.
    pub fn mask_local_vectors(&mut self) {
        self.write(REG_LVT_TIMER, self.read(REG_LVT_TIMER) | LVT_MASKED);
        self.write(REG_LVT_THERMAL, self.read(REG_LVT_THERMAL) | LVT_MASKED);
        self.write(REG_LVT_PERFORMANCE, self.read(REG_LVT_PERFORMANCE) | LVT_MASKED);
        self.write(REG_LVT_LINT0, self.read(REG_LVT_LINT0) | LVT_MASKED);
        self.write(REG_LVT_LINT1, self.read(REG_LVT_LINT1) | LVT_MASKED);
        self.write(REG_LVT_ERROR, self.read(REG_LVT_ERROR) | LVT_MASKED);
        if self.max_lvt_entry() >= 6 {
            self.write(REG_LVT_CMCI, self.read(REG_LVT_CMCI) | LVT_MASKED);
        }
    }

    pub fn is_software_enabled(&self) -> bool {
        self.read(REG_SVR) & SVR_SOFTWARE_ENABLE != 0
    }
}
