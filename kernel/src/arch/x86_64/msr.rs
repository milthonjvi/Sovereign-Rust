//! Model-specific register access.

#[inline]
/// Read a model-specific register.
///
/// # Safety
/// The MSR must be supported by the current CPU and the caller must run at CPL 0.
pub unsafe fn read(index: u32) -> u64 {
    let low: u32;
    let high: u32;
    // SAFETY: MSR validity and privilege are caller obligations.
    unsafe {
        core::arch::asm!(
            "rdmsr",
            in("ecx") index,
            out("eax") low,
            out("edx") high,
            options(nomem, nostack)
        );
    }
    ((high as u64) << 32) | low as u64
}

#[inline]
/// Write a model-specific register.
///
/// # Safety
/// The MSR must be writable and supported; the caller must run at CPL 0 and
/// provide a value satisfying the register's architectural constraints.
pub unsafe fn write(index: u32, value: u64) {
    let low = value as u32;
    let high = (value >> 32) as u32;
    // SAFETY: MSR validity, value, and privilege are caller obligations.
    unsafe {
        core::arch::asm!(
            "wrmsr",
            in("ecx") index,
            in("eax") low,
            in("edx") high,
            options(nomem, nostack)
        );
    }
}
