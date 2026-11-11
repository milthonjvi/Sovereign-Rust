//! x2APIC local interrupt controller support.
//!
//! x2APIC registers are accessed through architectural MSRs, eliminating the
//! local-APIC MMIO mapping requirement and supporting 32-bit APIC IDs. This
//! module does not configure the I/O APIC or enable CPU interrupt delivery.

use core::marker::PhantomData;

use super::{cpuid::CpuInfo, msr};

pub const IA32_APIC_BASE: u32 = 0x1B;
pub const IA32_X2APIC_BASE: u32 = 0x800;
pub const SPURIOUS_VECTOR: u8 = 0xFF;

pub const REG_ID: u32 = 0x802;
pub const REG_VERSION: u32 = 0x803;
pub const REG_TPR: u32 = 0x808;
pub const REG_EOI: u32 = 0x80B;
pub const REG_SVR: u32 = 0x80F;
pub const REG_ESR: u32 = 0x828;
pub const REG_LVT_CMCI: u32 = 0x82F;
pub const REG_LVT_TIMER: u32 = 0x832;
pub const REG_TIMER_INITIAL_COUNT: u32 = 0x838;
pub const REG_TIMER_CURRENT_COUNT: u32 = 0x839;
pub const REG_TIMER_DIVIDE_CONFIGURATION: u32 = 0x83E;
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
const LVT_TIMER_PERIODIC: u32 = 1 << 17;
const LVT_TIMER_TSC_DEADLINE: u32 = 1 << 18;
const LVT_TIMER_VECTOR_MASK: u32 = 0xFF;
const SVR_SOFTWARE_ENABLE: u32 = 1 << 8;
const ICR_DELIVERY_PENDING: u64 = 1 << 12;
const ICR_DESTINATION_SHIFT: u32 = 32;
const ICR_WAIT_LIMIT: usize = 1_000_000;

#[inline]
const fn valid_tsc_deadline_vector(vector: u8) -> bool {
    vector >= 0x30 && vector <= 0xFE && vector != 0x80
}

#[inline]
const fn valid_timer_divide_encoding(encoding: u8) -> bool {
    matches!(encoding, 0b0000 | 0b0001 | 0b0010 | 0b0011 | 0b1000 | 0b1001 | 0b1010 | 0b1011)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum X2ApicError {
    FeatureUnavailable,
    InvalidPhysicalBase,
    InvalidVector,
    InvalidTimerConfiguration,
    DeliveryTimeout,
}

/// Validate vectors accepted by fixed and self-IPI operations.
#[inline]
pub const fn valid_ipi_vector(vector: u8) -> bool {
    vector >= 32 && vector != 0xFF
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
    _not_send_sync: PhantomData<*mut ()>,
}

impl X2Apic {
    /// Construct a handle after x2APIC mode has been enabled.
    ///
    /// # Safety
    /// Caller must ensure IA32_APIC_BASE has both global-enable and x2APIC bits
    /// set on this CPU. The handle must not be shared across CPUs.
    pub unsafe fn new() -> Self {
        Self { _private: (), _not_send_sync: PhantomData }
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
        self.write_register(REG_TIMER_INITIAL_COUNT, 0);
        if super::cpuid::cpuid(1, 0).ecx & (1 << 24) != 0 {
            // Clear a firmware-left TSC deadline only when the MSR exists.
            unsafe { msr::write(0x6E0, 0) };
        }
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

    /// Configure the local timer LVT for one-shot countdown mode, initially masked.
    ///
    /// The countdown uses the local APIC timer input clock and selected divider.
    /// Register the interrupt handler before unmasking, then arm a non-zero
    /// initial count.
    pub fn configure_one_shot_timer(
        &mut self,
        vector: u8,
        divide_encoding: u8,
    ) -> Result<(), X2ApicError> {
        if !valid_tsc_deadline_vector(vector) || !valid_timer_divide_encoding(divide_encoding) {
            return Err(X2ApicError::InvalidTimerConfiguration);
        }
        self.write_register(REG_LVT_TIMER, (vector as u32 | LVT_MASKED) as u64);
        self.write_register(REG_TIMER_INITIAL_COUNT, 0);
        self.write_register(REG_TIMER_DIVIDE_CONFIGURATION, divide_encoding as u64);
        Ok(())
    }

    /// Unmask a configured one-shot countdown timer.
    ///
    /// The initial count should be zero until the caller is ready to arm it.
    pub fn unmask_one_shot_timer(&mut self) -> Result<(), X2ApicError> {
        let value = self.read_register(REG_LVT_TIMER) as u32;
        let vector = (value & LVT_TIMER_VECTOR_MASK) as u8;
        if value & (LVT_TIMER_PERIODIC | LVT_TIMER_TSC_DEADLINE) != 0
            || value & LVT_MASKED == 0
            || !valid_tsc_deadline_vector(vector)
        {
            return Err(X2ApicError::InvalidTimerConfiguration);
        }
        self.write_register(REG_LVT_TIMER, (value & !LVT_MASKED) as u64);
        Ok(())
    }

    /// Load a countdown while its LVT source is still masked.
    ///
    /// This is intended for calibration: the counter runs without delivering
    /// an interrupt, then the caller stops it before normal activation.
    pub fn load_masked_one_shot_count(
        &mut self,
        initial_count: u32,
    ) -> Result<(), X2ApicError> {
        let lvt = self.read_register(REG_LVT_TIMER) as u32;
        if initial_count == 0
            || lvt & LVT_MASKED == 0
            || lvt & (LVT_TIMER_PERIODIC | LVT_TIMER_TSC_DEADLINE) != 0
            || !valid_tsc_deadline_vector((lvt & LVT_TIMER_VECTOR_MASK) as u8)
        {
            return Err(X2ApicError::InvalidTimerConfiguration);
        }
        self.write_register(REG_TIMER_INITIAL_COUNT, initial_count as u64);
        Ok(())
    }

    /// Arm or restart the configured one-shot countdown.
    ///
    /// # Safety
    /// Must execute on the owning CPU at CPL 0 after the one-shot LVT has been
    /// unmasked and the corresponding handler is registered.
    pub unsafe fn arm_one_shot_timer(&mut self, initial_count: u32) -> Result<(), X2ApicError> {
        if initial_count == 0 {
            return Err(X2ApicError::InvalidTimerConfiguration);
        }
        let lvt = self.read_register(REG_LVT_TIMER) as u32;
        if lvt & (LVT_MASKED | LVT_TIMER_PERIODIC | LVT_TIMER_TSC_DEADLINE) != 0
            || !valid_tsc_deadline_vector((lvt & LVT_TIMER_VECTOR_MASK) as u8)
        {
            return Err(X2ApicError::InvalidTimerConfiguration);
        }
        self.write_register(REG_TIMER_INITIAL_COUNT, initial_count as u64);
        Ok(())
    }

    /// Read the current one-shot countdown value.
    #[inline]
    pub fn timer_current_count(&self) -> u32 {
        self.read_register(REG_TIMER_CURRENT_COUNT) as u32
    }

    /// Disarm a one-shot countdown without masking its LVT source.
    ///
    /// Keeping the LVT unmasked allows a later queue insertion to arm the
    /// counter without repeating interrupt registration.
    pub fn disarm_one_shot_timer(&mut self) -> Result<(), X2ApicError> {
        let lvt = self.read_register(REG_LVT_TIMER) as u32;
        if lvt & (LVT_TIMER_PERIODIC | LVT_TIMER_TSC_DEADLINE) != 0
            || !valid_tsc_deadline_vector((lvt & LVT_TIMER_VECTOR_MASK) as u8)
        {
            return Err(X2ApicError::InvalidTimerConfiguration);
        }
        self.write_register(REG_TIMER_INITIAL_COUNT, 0);
        Ok(())
    }

    /// Stop the one-shot countdown and mask local timer delivery.
    pub fn stop_one_shot_timer(&mut self) {
        self.mask_timer();
        self.write_register(REG_TIMER_INITIAL_COUNT, 0);
    }

    /// Configure the local timer LVT for TSC-deadline mode, initially masked.
    ///
    /// The timer vector must already have an IDT gate installed and its
    /// dispatcher callback registered before the caller unmasks this source.
    pub fn configure_tsc_deadline_timer(&mut self, vector: u8) -> Result<(), X2ApicError> {
        // Keep the architectural syscall vector and spurious vector reserved.
        if !valid_tsc_deadline_vector(vector) {
            return Err(X2ApicError::InvalidVector);
        }
        if super::cpuid::cpuid(1, 0).ecx & (1 << 24) == 0 {
            return Err(X2ApicError::FeatureUnavailable);
        }
        self.write_register(REG_LVT_TIMER, (vector as u32 | LVT_TIMER_TSC_DEADLINE | LVT_MASKED) as u64);
        self.write_register(REG_TIMER_INITIAL_COUNT, 0);
        Ok(())
    }

    /// Unmask a previously configured TSC-deadline local timer.
    ///
    /// This does not arm a deadline. Program IA32_TSC_DEADLINE first, then
    /// unmask the LVT so no interrupt can be delivered from stale state.
    pub fn unmask_tsc_deadline_timer(&mut self) -> Result<(), X2ApicError> {
        let value = self.read_register(REG_LVT_TIMER) as u32;
        if value & LVT_TIMER_TSC_DEADLINE == 0
            || value & LVT_TIMER_PERIODIC != 0
            || value & LVT_MASKED == 0
            || !valid_tsc_deadline_vector((value & LVT_TIMER_VECTOR_MASK) as u8)
        {
            return Err(X2ApicError::InvalidTimerConfiguration);
        }
        self.write_register(REG_LVT_TIMER, (value & !LVT_MASKED) as u64);
        Ok(())
    }

    /// Mask the local timer without changing its vector or operating mode.
    pub fn mask_timer(&mut self) {
        let value = self.read_register(REG_LVT_TIMER);
        self.write_register(REG_LVT_TIMER, value | LVT_MASKED as u64);
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
        if !valid_ipi_vector(vector) {
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

#[cfg(test)]
mod tests {
    use super::valid_ipi_vector;

    #[test]
    fn reserves_exceptions_and_apic_spurious_vector() {
        assert!(!valid_ipi_vector(0));
        assert!(!valid_ipi_vector(31));
        assert!(valid_ipi_vector(32));
        assert!(valid_ipi_vector(0xFE));
        assert!(!valid_ipi_vector(0xFF));
    }
}

#[cfg(test)]
mod tsc_deadline_vector_tests {
    use super::valid_tsc_deadline_vector;

    #[test]
    fn accepts_only_installed_apic_timer_vector_range() {
        assert!(!valid_tsc_deadline_vector(0x2F));
        assert!(valid_tsc_deadline_vector(0x30));
        assert!(valid_tsc_deadline_vector(0x7F));
        assert!(!valid_tsc_deadline_vector(0x80));
        assert!(valid_tsc_deadline_vector(0x81));
        assert!(valid_tsc_deadline_vector(0xFE));
        assert!(!valid_tsc_deadline_vector(0xFF));
    }
}

#[cfg(test)]
mod timer_encoding_tests {
    use super::valid_timer_divide_encoding;

    #[test]
    fn accepts_only_architectural_apic_timer_divisors() {
        for encoding in [0, 1, 2, 3, 8, 9, 10, 11] {
            assert!(valid_timer_divide_encoding(encoding));
        }
        for encoding in [4, 5, 6, 7, 12, 13, 14, 15, 16, 0xFF] {
            assert!(!valid_timer_divide_encoding(encoding));
        }
    }
}
