//! x2APIC local interrupt controller support.
//!
//! x2APIC registers are accessed through architectural MSRs, eliminating the
//! local-APIC MMIO mapping requirement and supporting 32-bit APIC IDs. This
//! module does not configure the I/O APIC or enable CPU interrupt delivery.

use super::{cpuid::CpuInfo, msr};

pub const IA32_APIC_BASE: u32 = 0x1B;
pub const IA32_X2APIC_BASE: u32 = 0x800;

pub const REG_ID: u32 = 0x802;
pub const REG_VERSION: u32 = 0x803;
pub const REG_TPR: u32 = 0x808;
pub const REG_EOI: u32 = 0x80B;
pub const REG_SVR: u32 = 0x80F;
pub const REG_ESR: u32 = 0x828;
pub const REG_LVT_CMCI: u32 = 0x82F;
pub const REG_LVT_TIMER: u32 = 0x832;
pub const REG_LVT_THERMAL: u32 = 0x833;
pub const REG_LVT_PERFORMANCE: u32 = 0x834;
pub const REG_LVT_LINT0: u32 = 0x835;
pub const REG_LVT_LINT1: u32 = 0x836;
pub const REG_LVT_ERROR: u32 = 0x837;
pub const REG_ICR: u32 = 0x830;
pub const REG_SELF_IPI: u32 = 0x83F;

const APIC_BASE_X2APIC: u64 = 1 << 10;
const APIC_BASE_GLOBAL_ENABLE: u64 = 1 << 11;
const APIC_BASE_ADDRESS_MASK: u64 = 0x000F_FFFF_FFFF_F000;
const LVT_MASKED: u32 = 1 << 16;
const SVR_SOFTWARE_ENABLE: u32 = 1 << 8;
const ICR_DELIVERY_PENDING: u64 = 1 << 12;
const ICR_FIXED_DELIVERY: u64 = 0;
const ICR_LEVEL_ASSERT: u64 = 1 << 14;
const ICR_TRIGGER_LEVEL: u64 = 1 << 15;
const ICR_DESTINATION_SHIFT: u32 = 32;
const ICR_WAIT_LIMIT: usize = 1_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum X2ApicError {
    FeatureUnavailable,
    InvalidPhysicalBase,
    InvalidVector,
    InvalidDestination,
    DeliveryTimeout,
}

/// Enable x2APIC mode on the current logical processor.
///
/// The APIC base physical address and unrelated MSR bits are preserved. The
/// global APIC enable bit is set before the x2APIC mode bit.
///
/// # Safety
/// Must execute at CPL 0 on a CPU supporting APIC, x2APIC and RDMSR/WRMSR.
/// This changes the local interrupt-controller mode for the current CPU.
pub unsafe fn enable() -> Result<u64, X2ApicError> {
    let features = CpuInfo::detect().features;
    if !features.apic || !features.x2apic {
        return Err(X2ApicError::FeatureUnavailable);
    }

    // SAFETY: CPUID confirms APIC support; caller guarantees CPL 0.
    let mut base = unsafe { msr::read(IA32_APIC_BASE) };
    let physical = base & APIC_BASE_ADDRESS_MASK;
    if physical == 0 || physical & 0xFFF != 0 {
        return Err(X2ApicError::InvalidPhysicalBase);
    }

    if base & APIC_BASE_GLOBAL_ENABLE == 0 {
        base |= APIC_BASE_GLOBAL_ENABLE;
        // SAFETY: Only the architectural global-enable bit is changed.
        unsafe { msr::write(IA32_APIC_BASE, base) };
    }

    if base & APIC_BASE_X2APIC == 0 {
        base |= APIC_BASE_X2APIC;
        // SAFETY: x2APIC is advertised by CPUID and APIC is globally enabled.
        unsafe { msr::write(IA32_APIC_BASE, base) };
    }

    Ok(physical)
}

/// Handle for the current CPU's x2APIC MSR register bank.
///
/// The type is intentionally not Sync: all access must be serialized on the
/// owning logical processor. It is cheap to construct after enable().
pub struct X2Apic {
    _private: (),
}

impl X2Apic {
    /// Construct a handle after x2APIC mode has been enabled.
    ///
    /// # Safety
    /// Caller must ensure IA32_APIC_BASE has both global-enable and x2APIC bits
    /// set on this CPU. The handle must not be shared across CPUs.
    pub unsafe fn new() -> Self {
        Self { _private: () }
    }

    #[inline]
    fn read_register(&self, register: u32) -> u64 {
        // SAFETY: All register constants are valid x2APIC MSRs.
        unsafe { msr::read(register) }
    }

    #[inline]
    fn write_register(&mut self, register: u32, value: u64) {
        // SAFETY: All register constants are valid writable x2APIC MSRs.
        unsafe { msr::write(register, value) }
    }

    pub fn id(&self) -> u32 {
        self.read_register(REG_ID) as u32
    }

    pub fn version(&self) -> u32 {
        self.read_register(REG_VERSION) as u32
    }

    pub fn max_lvt_entry(&self) -> u8 {
        ((self.version() >> 16) & 0xFF) as u8
    }

    /// Configure the local APIC and keep every implemented LVT source masked.
    pub fn initialize_masked(&mut self, spurious_vector: u8) -> Result<(), X2ApicError> {
        if spurious_vector < 32 {
            return Err(X2ApicError::InvalidVector);
        }

        self.write_register(REG_TPR, 0);
        self.write_register(REG_ESR, 0);
        let _ = self.read_register(REG_ESR);

        let max_lvt = self.max_lvt_entry();
        self.write_register(REG_LVT_TIMER, LVT_MASKED as u64);
        if max_lvt >= 1 { self.write_register(REG_LVT_THERMAL, LVT_MASKED as u64); }
        if max_lvt >= 2 { self.write_register(REG_LVT_PERFORMANCE, LVT_MASKED as u64); }
        if max_lvt >= 3 { self.write_register(REG_LVT_LINT0, LVT_MASKED as u64); }
        if max_lvt >= 4 { self.write_register(REG_LVT_LINT1, LVT_MASKED as u64); }
        if max_lvt >= 5 { self.write_register(REG_LVT_ERROR, LVT_MASKED as u64); }
        if max_lvt >= 6 { self.write_register(REG_LVT_CMCI, LVT_MASKED as u64); }

        let svr = self.read_register(REG_SVR);
        self.write_register(
            REG_SVR,
            (svr & !0x1FF) | spurious_vector as u64 | SVR_SOFTWARE_ENABLE as u64,
        );
        Ok(())
    }

    /// Mask every implemented local-vector source without disabling x2APIC.
    pub fn mask_local_vectors(&mut self) {
        let max_lvt = self.max_lvt_entry();
        self.mask_register(REG_LVT_TIMER);
        if max_lvt >= 1 { self.mask_register(REG_LVT_THERMAL); }
        if max_lvt >= 2 { self.mask_register(REG_LVT_PERFORMANCE); }
        if max_lvt >= 3 { self.mask_register(REG_LVT_LINT0); }
        if max_lvt >= 4 { self.mask_register(REG_LVT_LINT1); }
        if max_lvt >= 5 { self.mask_register(REG_LVT_ERROR); }
        if max_lvt >= 6 { self.mask_register(REG_LVT_CMCI); }
    }

    #[inline]
    fn mask_register(&mut self, register: u32) {
        let value = self.read_register(register);
        self.write_register(register, value | LVT_MASKED as u64);
    }

    /// Acknowledge the current local APIC interrupt.
    #[inline]
    pub fn end_of_interrupt(&mut self) {
        self.write_register(REG_EOI, 0);
    }

    /// Send a fixed IPI to a destination APIC ID.
    ///
    /// The delivery-status bit is polled with a hard upper bound. The vector
    /// must be a normal interrupt vector; 0xFF remains reserved for spurious
    /// interrupts.
    pub fn send_fixed_ipi(
        &mut self,
        destination_apic_id: u32,
        vector: u8,
    ) -> Result<(), X2ApicError> {
        if vector < 32 || vector == 0xFF {
            return Err(X2ApicError::InvalidVector);
        }

        let mut pending = false;
        for _ in 0..ICR_WAIT_LIMIT {
            if self.read_register(REG_ICR) & ICR_DELIVERY_PENDING == 0 {
                pending = true;
                break;
            }
            core::hint::spin_loop();
        }
        if !pending {
            return Err(X2ApicError::DeliveryTimeout);
        }

        let command = ((destination_apic_id as u64) << ICR_DESTINATION_SHIFT)
            | ICR_FIXED_DELIVERY
            | vector as u64;
        self.write_register(REG_ICR, command);

        for _ in 0..ICR_WAIT_LIMIT {
            if self.read_register(REG_ICR) & ICR_DELIVERY_PENDING == 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(X2ApicError::DeliveryTimeout)
    }

    /// Send a fixed IPI to the current logical processor.
    pub fn send_self_ipi(&mut self, vector: u8) -> Result<(), X2ApicError> {
        if vector < 32 || vector == 0xFF {
            return Err(X2ApicError::InvalidVector);
        }
        self.write_register(REG_SELF_IPI, vector as u64);
        Ok(())
    }

    /// Report whether the local APIC software-enable bit is set.
    pub fn is_software_enabled(&self) -> bool {
        self.read_register(REG_SVR) as u32 & SVR_SOFTWARE_ENABLE != 0
    }
}
