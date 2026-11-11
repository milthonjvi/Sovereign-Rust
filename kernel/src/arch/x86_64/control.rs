//! Privileged CPU control and status register access.

#[inline]
/// Read CR0.
///
/// # Safety
/// Must be called at CPL 0.
pub unsafe fn read_cr0() -> u64 {
    let value: u64;
    // SAFETY: Caller guarantees ring 0.
    unsafe { core::arch::asm!("mov {}, cr0", out(reg) value, options(nomem, nostack, preserves_flags)); }
    value
}

#[inline]
/// Read CR2 (page-fault linear address).
///
/// # Safety
/// Must be called at CPL 0.
pub unsafe fn read_cr2() -> u64 {
    let value: u64;
    // SAFETY: Caller guarantees ring 0.
    unsafe { core::arch::asm!("mov {}, cr2", out(reg) value, options(nomem, nostack, preserves_flags)); }
    value
}

#[inline]
/// Read CR3 (current page-table root and PCID state).
///
/// # Safety
/// Must be called at CPL 0.
pub unsafe fn read_cr3() -> u64 {
    let value: u64;
    // SAFETY: Caller guarantees ring 0.
    unsafe { core::arch::asm!("mov {}, cr3", out(reg) value, options(nomem, nostack, preserves_flags)); }
    value
}

#[inline]
/// Read CR4.
///
/// # Safety
/// Must be called at CPL 0.
pub unsafe fn read_cr4() -> u64 {
    let value: u64;
    // SAFETY: Caller guarantees ring 0.
    unsafe { core::arch::asm!("mov {}, cr4", out(reg) value, options(nomem, nostack, preserves_flags)); }
    value
}

#[inline]
/// Read RFLAGS.
pub fn read_rflags() -> u64 {
    let value: u64;
    // SAFETY: PUSHFQ/POP do not access privileged state.
    unsafe { core::arch::asm!("pushfq", "pop {}", out(reg) value, options()); }
    value
}

#[inline]
/// Disable maskable interrupts.
///
/// # Safety
/// Caller must ensure interrupt state is restored at the correct boundary.
pub unsafe fn disable_interrupts() {
    // SAFETY: Caller owns interrupt-state discipline.
    unsafe { core::arch::asm!("cli", options(nomem, nostack)); }
}

#[inline]
/// Enable maskable interrupts.
///
/// # Safety
/// IDT and interrupt-controller state must already be valid.
pub unsafe fn enable_interrupts() {
    // SAFETY: Caller guarantees a valid interrupt environment.
    unsafe { core::arch::asm!("sti", options(nomem, nostack)); }
}

#[inline]
/// Halt until an enabled interrupt arrives.
///
/// # Safety
/// Must be called at CPL 0 with a valid interrupt strategy.
pub unsafe fn halt() {
    // SAFETY: Caller guarantees ring 0 and a valid wake-up source.
    unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
}

#[inline]
pub fn pause() {
    // SAFETY: PAUSE is an unprivileged spin-wait hint.
    unsafe { core::arch::asm!("pause", options(nomem, nostack, preserves_flags)); }
}

#[inline]
/// Write CR0.
///
/// # Safety
/// Must be called at CPL 0. The caller must preserve architectural CR0 invariants.
pub unsafe fn write_cr0(value: u64) {
    // SAFETY: Caller guarantees ring 0 and a valid CR0 value.
    unsafe { core::arch::asm!("mov cr0, {}", in(reg) value, options(nomem, nostack, preserves_flags)); }
}

#[inline]
/// Write CR3 (page-table root and optional PCID).
///
/// # Safety
/// Must execute at CPL 0. The root must be page-aligned and refer to a valid
/// page-table hierarchy; PCID/no-flush bits must obey the active CR4.PCIDE state.
pub unsafe fn write_cr3(value: u64) {
    // SAFETY: Caller guarantees ring 0 and a valid CR3 value.
    unsafe { core::arch::asm!("mov cr3, {}", in(reg) value, options(nostack, preserves_flags)); }
}

#[inline]
/// Invalidate one linear address in the local processor's TLB.
///
/// # Safety
/// Must execute at CPL 0. The address must be canonical for the active paging
/// mode. Other processors require their own invalidation or a TLB shootdown.
pub unsafe fn invalidate_page(address: u64) {
    // SAFETY: Caller guarantees CPL 0 and a canonical linear address.
    unsafe { core::arch::asm!("invlpg [{}]", in(reg) address, options(nostack, preserves_flags)); }
}

#[inline]
/// Flush non-global translations for the current address space on this CPU.
///
/// This preserves the current physical root and PCID. Global translations are
/// intentionally not invalidated; use a PGE-aware global flush when required.
///
/// # Safety
/// Must execute at CPL 0 with a valid active CR3 and correct PCID state.
pub unsafe fn flush_current_address_space() {
    // SAFETY: Caller guarantees ring 0 and a valid current page-table root.
    let current = unsafe { read_cr3() };
    // A zero no-flush bit requests a context flush when PCID is enabled.
    unsafe { write_cr3(current & !(1u64 << 63)) };
}

#[inline]
/// Write CR4.
///
/// # Safety
/// Must be called at CPL 0. The caller must enable only features supported by
/// the processor and must satisfy their associated OS-state requirements.
pub unsafe fn write_cr4(value: u64) {
    // SAFETY: Caller guarantees ring 0 and a valid CR4 value.
    unsafe { core::arch::asm!("mov cr4, {}", in(reg) value, options(nomem, nostack, preserves_flags)); }
}
