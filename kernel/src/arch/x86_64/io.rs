//! x86 port-mapped I/O primitives.
//!
//! These functions are intentionally unsafe: callers must own the device,
//! know the port width, and serialize concurrent access where required.

#[inline]
/// Read one byte from an I/O port.
///
/// # Safety
/// The port must be valid for byte reads and exclusively managed by the caller.
pub unsafe fn in8(port: u16) -> u8 {
    let value: u8;
    // SAFETY: Port validity and ownership are caller obligations.
    unsafe { core::arch::asm!("in al, dx", in("dx") port, out("al") value, options(nomem, nostack, preserves_flags)); }
    value
}

#[inline]
/// Read a 16-bit word from an I/O port.
///
/// # Safety
/// The port must be valid for 16-bit reads and exclusively managed by the caller.
pub unsafe fn in16(port: u16) -> u16 {
    let value: u16;
    // SAFETY: Port validity and ownership are caller obligations.
    unsafe { core::arch::asm!("in ax, dx", in("dx") port, out("ax") value, options(nomem, nostack, preserves_flags)); }
    value
}

#[inline]
/// Read a 32-bit value from an I/O port.
///
/// # Safety
/// The port must be valid for 32-bit reads and exclusively managed by the caller.
pub unsafe fn in32(port: u16) -> u32 {
    let value: u32;
    // SAFETY: Port validity and ownership are caller obligations.
    unsafe { core::arch::asm!("in eax, dx", in("dx") port, out("eax") value, options(nomem, nostack, preserves_flags)); }
    value
}

#[inline]
/// Write one byte to an I/O port.
///
/// # Safety
/// The port must accept byte writes and be exclusively managed by the caller.
pub unsafe fn out8(port: u16, value: u8) {
    // SAFETY: Port validity and ownership are caller obligations.
    unsafe { core::arch::asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags)); }
}

#[inline]
/// Write a 16-bit word to an I/O port.
///
/// # Safety
/// The port must accept 16-bit writes and be exclusively managed by the caller.
pub unsafe fn out16(port: u16, value: u16) {
    // SAFETY: Port validity and ownership are caller obligations.
    unsafe { core::arch::asm!("out dx, ax", in("dx") port, in("ax") value, options(nomem, nostack, preserves_flags)); }
}

#[inline]
/// Write a 32-bit value to an I/O port.
///
/// # Safety
/// The port must accept 32-bit writes and be exclusively managed by the caller.
pub unsafe fn out32(port: u16, value: u32) {
    // SAFETY: Port validity and ownership are caller obligations.
    unsafe { core::arch::asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack, preserves_flags)); }
}
