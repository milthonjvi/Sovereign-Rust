//! Early bootstrap-processor initialization.

use super::control::{read_cr0, read_cr4, write_cr0, write_cr4};
use super::cpuid::CpuInfo;

const CR0_MP: u64 = 1 << 1;
const CR0_EM: u64 = 1 << 2;
const CR0_TS: u64 = 1 << 3;
const CR0_NE: u64 = 1 << 5;

const CR4_OSFXSR: u64 = 1 << 9;
const CR4_OSXMMEXCPT: u64 = 1 << 10;

/// Enable the architectural baseline required for normal x86_64 kernel code.
///
/// This does not enable AVX/XSAVE, APIC routing, paging changes, or interrupts.
/// Those require their own initialization and state-management contracts.
///
/// # Safety
/// Must execute on the bootstrap processor at CPL 0, before enabling interrupts
/// or starting code that relies on floating-point/SIMD context management.
pub unsafe fn initialize_baseline() -> CpuInfo {
    let info = CpuInfo::detect();

    // SSE2 is mandatory in long mode. Treat a contradictory CPUID report as a
    // fatal virtual-CPU/platform configuration rather than using invalid state.
    if !info.features.sse2 {
        loop {
            // SAFETY: This function is called at CPL 0 with interrupts disabled.
            unsafe { core::arch::asm!("cli", "hlt", options(nomem, nostack)); }
        }
    }

    // CR0: enable native x87 error reporting and make the FPU available.
    // SAFETY: Caller guarantees ring 0 and early single-CPU initialization.
    let mut cr0 = unsafe { read_cr0() };
    cr0 |= CR0_MP | CR0_NE;
    cr0 &= !(CR0_EM | CR0_TS);
    // SAFETY: The modified bits are architecturally defined.
    unsafe { write_cr0(cr0) };

    // CR4: enable OS support for FXSAVE/FXRSTOR and SIMD exceptions.
    // XSAVE/AVX remain disabled until the kernel implements extended-state
    // save/restore for every task and CPU.
    // SAFETY: Caller guarantees ring 0 and early single-CPU initialization.
    let mut cr4 = unsafe { read_cr4() };
    cr4 |= CR4_OSFXSR | CR4_OSXMMEXCPT;
    // SAFETY: SSE2 is reported by CPUID before these bits are enabled.
    unsafe { write_cr4(cr4) };

    info
}
