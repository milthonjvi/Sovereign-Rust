//! Minimal x86_64 CPU feature setup required by Sovereign Boot paging.

use core::arch::{asm, x86_64::__cpuid};

const CPUID_EXTENDED_MAX: u32 = 0x8000_0000;
const CPUID_EXTENDED_FEATURES: u32 = 0x8000_0001;
const EFER_MSR: u32 = 0xC000_0080;
const EFER_NXE: u64 = 1 << 11;
const CPUID_NX: u32 = 1 << 20;

/// Proof token produced only after EFER.NXE is confirmed active.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NxeEnabled { paging_levels: u8 }

impl NxeEnabled {
    pub const fn paging_levels(self) -> u8 { self.paging_levels }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CpuFeatureError {
    ExtendedCpuidUnavailable,
    NxUnsupported,
    NxeEnableFailed,
}

/// Enable EFER.NXE so page-table NX bits are architecturally active.
///
/// # Safety
/// Must execute at CPL0 on x86_64 with a valid MSR-capable CPU. The caller
/// must ensure firmware execution is not concurrently using incompatible
/// assumptions about EFER state.
pub unsafe fn enable_nxe() -> Result<NxeEnabled, CpuFeatureError> {
    // SAFETY: CPUID is available in long mode and does not mutate machine state.
    let maximum = unsafe { __cpuid(CPUID_EXTENDED_MAX) }.eax;
    if maximum < CPUID_EXTENDED_FEATURES {
        return Err(CpuFeatureError::ExtendedCpuidUnavailable);
    }
    // SAFETY: The extended feature leaf was confirmed to be available.
    let features = unsafe { __cpuid(CPUID_EXTENDED_FEATURES) };
    if features.edx & CPUID_NX == 0 {
        return Err(CpuFeatureError::NxUnsupported);
    }

    let low: u32;
    let high: u32;
    // SAFETY: NX support was confirmed; EFER is the architectural MSR.
    unsafe {
        asm!(
            "rdmsr",
            in("ecx") EFER_MSR,
            out("eax") low,
            out("edx") high,
            options(nomem, nostack, preserves_flags)
        );
    }
    let efer = ((high as u64) << 32) | low as u64;
    if efer & EFER_NXE == 0 {
        let updated = efer | EFER_NXE;
        // SAFETY: Preserve all EFER bits and set only the supported NXE bit.
        unsafe {
            asm!(
                "wrmsr",
                in("ecx") EFER_MSR,
                in("eax") updated as u32,
                in("edx") (updated >> 32) as u32,
                options(nomem, nostack, preserves_flags)
            );
        }
    }
    let verified_low: u32;
    let verified_high: u32;
    // SAFETY: Read back the architectural EFER MSR after the update.
    unsafe {
        asm!(
            "rdmsr",
            in("ecx") EFER_MSR,
            out("eax") verified_low,
            out("edx") verified_high,
            options(nomem, nostack, preserves_flags)
        );
    }
    let verified_efer = ((verified_high as u64) << 32) | verified_low as u64;
    if verified_efer & EFER_NXE == 0 {
        return Err(CpuFeatureError::NxeEnableFailed);
    }

    // CR0.WP is deferred until after ExitBootServices. Enabling it while
    // firmware is still running can expose firmware assumptions about its
    // supervisor mappings; the final transition enables it with our CR3.

    let cr4: u64;
    // SAFETY: Reading CR4 is valid at CPL 0 and does not mutate processor state.
    unsafe {
        asm!(
            "mov {}, cr4",
            out(reg) cr4,
            options(nomem, nostack, preserves_flags)
        );
    }
    let paging_levels = if cr4 & (1 << 12) != 0 { 5 } else { 4 };
    Ok(NxeEnabled { paging_levels })
}
