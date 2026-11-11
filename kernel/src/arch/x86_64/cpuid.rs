//! CPUID discovery and feature reporting.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CpuidResult {
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
}

/// Execute CPUID for a leaf and subleaf.
///
/// CPUID is guaranteed on x86_64. This wrapper does not alter control state.
#[inline]
pub fn cpuid(leaf: u32, subleaf: u32) -> CpuidResult {
    let mut eax = leaf;
    let mut ecx = subleaf;
    let ebx: u32;
    let edx: u32;

    // SAFETY: CPUID is available in long mode and has no memory side effects.
    unsafe {
        core::arch::asm!(
            "cpuid",
            inout("eax") eax,
            lateout("ebx") ebx,
            inout("ecx") ecx,
            lateout("edx") edx,
            options(nomem, nostack)
        );
    }

    CpuidResult { eax, ebx, ecx, edx }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CpuFeatures {
    pub apic: bool,
    pub x2apic: bool,
    pub sse2: bool,
    pub xsave: bool,
    pub avx: bool,
    pub syscall: bool,
    pub nx: bool,
    pub page_1g: bool,
    pub rdrand: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CpuInfo {
    pub vendor: [u8; 12],
    pub max_basic_leaf: u32,
    pub max_extended_leaf: u32,
    pub features: CpuFeatures,
}

impl CpuInfo {
    /// Read the processor vendor and the feature bits Sovereign currently uses.
    pub fn detect() -> Self {
        let basic = cpuid(0, 0);
        let mut vendor = [0u8; 12];
        vendor[0..4].copy_from_slice(&basic.ebx.to_le_bytes());
        vendor[4..8].copy_from_slice(&basic.edx.to_le_bytes());
        vendor[8..12].copy_from_slice(&basic.ecx.to_le_bytes());

        let leaf1 = if basic.eax >= 1 { cpuid(1, 0) } else {
            CpuidResult { eax: 0, ebx: 0, ecx: 0, edx: 0 }
        };
        let extended = cpuid(0x8000_0000, 0).eax;
        let ext1 = if extended >= 0x8000_0001 { cpuid(0x8000_0001, 0) } else {
            CpuidResult { eax: 0, ebx: 0, ecx: 0, edx: 0 }
        };

        Self {
            vendor,
            max_basic_leaf: basic.eax,
            max_extended_leaf: extended,
            features: CpuFeatures {
                apic: leaf1.edx & (1 << 9) != 0,
                x2apic: leaf1.ecx & (1 << 21) != 0,
                sse2: leaf1.edx & (1 << 26) != 0,
                xsave: leaf1.ecx & (1 << 26) != 0,
                avx: leaf1.ecx & (1 << 28) != 0,
                syscall: ext1.edx & (1 << 11) != 0,
                nx: ext1.edx & (1 << 20) != 0,
                page_1g: ext1.edx & (1 << 26) != 0,
                rdrand: leaf1.ecx & (1 << 30) != 0,
            },
        }
    }

    pub fn vendor_str(&self) -> &str {
        // CPUID vendor IDs are fixed-width ASCII byte sequences.
        core::str::from_utf8(&self.vendor).unwrap_or("unknown")
    }
}
