//! Local APIC interrupt vector gates and EOI-owned dispatch.
//!
//! Vectors 0x30..=0xFE are available to local-APIC and I/O-APIC sources,
//! except 0x80 which is reserved for the future syscall ABI. Vector 0xFF is
//! the dedicated spurious-vector IRETQ gate installed by exception setup.

use super::{
    dispatch::{self, InterruptHandler},
    exceptions::ExceptionFrame,
    idt::{Idt, IdtEntry, IdtError, INTERRUPT_GATE},
};
use crate::arch::x86_64::x2apic::SPURIOUS_VECTOR;

pub const APIC_VECTOR_START: u8 = 0x30;
pub const APIC_VECTOR_END: u8 = 0xFE;
pub const SYSCALL_VECTOR: u8 = 0x80;
pub const APIC_VECTOR_COUNT: usize = 206;

core::arch::global_asm!(include_str!("../arch/x86_64/apic_irq.S"));

unsafe extern "C" {
    static sovereign_apic_stub_table: [u64; APIC_VECTOR_COUNT];
}

/// Errors returned while installing or registering local APIC vectors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApicVectorError {
    InvalidVector,
    InvalidGate(IdtError),
    LocalApicUnavailable,
    HandlerAlreadyRegistered,
    HandlerNotRegistered,
}

/// Whether a vector belongs to the external/local APIC dispatch range.
#[inline]
pub const fn is_apic_vector(vector: u8) -> bool {
    vector >= APIC_VECTOR_START
        && vector <= APIC_VECTOR_END
        && vector != SYSCALL_VECTOR
        && vector != SPURIOUS_VECTOR
}

/// Install all vector-specific local APIC gates into the bootstrap IDT.
///
/// # Safety
/// The current CS selector must identify the active 64-bit kernel code segment.
/// The IDT must remain writable during installation and stable after LIDT.
pub unsafe fn install_apic_gates(idt: &mut Idt) -> Result<(), ApicVectorError> {
    let selector = Idt::current_code_selector();
    for vector in APIC_VECTOR_START..=APIC_VECTOR_END {
        if !is_apic_vector(vector) {
            continue;
        }

        let index = if vector < SYSCALL_VECTOR {
            (vector - APIC_VECTOR_START) as usize
        } else {
            (vector - APIC_VECTOR_START - 1) as usize
        };
        // SAFETY: Assembly exports exactly APIC_VECTOR_COUNT stub pointers.
        let handler = unsafe { sovereign_apic_stub_table[index] };
        let entry = IdtEntry::new(handler, selector, 0, INTERRUPT_GATE, 0)
            .map_err(ApicVectorError::InvalidGate)?;
        idt.set_gate(vector, entry);
    }
    Ok(())
}

/// Register a handler for an APIC-delivered vector.
///
/// The hardware source must remain masked until registration succeeds.
///
/// # Safety
/// Call during serialized device initialization with the corresponding I/O
/// APIC or local-APIC source masked. The handler must remain valid for the
/// kernel lifetime, be bounded, nonblocking, and never unwind.
pub unsafe fn register_vector(
    vector: u8,
    handler: InterruptHandler,
) -> Result<(), ApicVectorError> {
    if !is_apic_vector(vector) {
        return Err(ApicVectorError::InvalidVector);
    }
    if super::init::local_apic_mode() != super::init::LocalApicMode::X2Apic {
        return Err(ApicVectorError::LocalApicUnavailable);
    }
    dispatch::register(vector, handler)
        .map_err(|_| ApicVectorError::HandlerAlreadyRegistered)
}

/// Remove a handler after masking its hardware source and draining in-flight
/// delivery.
///
/// # Safety
/// The source must be masked and no CPU may still be executing this callback.
pub unsafe fn unregister_vector(
    vector: u8,
    handler: InterruptHandler,
) -> Result<(), ApicVectorError> {
    if !is_apic_vector(vector) {
        return Err(ApicVectorError::InvalidVector);
    }
    dispatch::unregister(vector, handler)
        .map_err(|_| ApicVectorError::HandlerNotRegistered)
}

/// Common entry for local APIC and I/O APIC interrupts.
///
/// EOI is sent centrally after the device callback returns. The spurious
/// vector 0xFF never enters this function and is never acknowledged.
#[unsafe(no_mangle)]
pub extern "C" fn sovereign_apic_dispatch(frame: *const ExceptionFrame) {
    // SAFETY: Assembly passes the frame it pushed immediately before the call.
    let snapshot = unsafe { *frame };
    let Ok(vector) = u8::try_from(snapshot.vector) else {
        fatal_unhandled_apic();
    };
    if !is_apic_vector(vector) || !dispatch::dispatch_vector(vector) {
        fatal_unhandled_apic();
    }

    // SAFETY: This path handles a genuine APIC-delivered vector on the BSP.
    if unsafe { super::init::end_local_apic_irq() }.is_err() {
        fatal_unhandled_apic();
    }
}

fn fatal_unhandled_apic() -> ! {
    loop {
        // SAFETY: Returning without EOI would leave the local APIC in-service.
        unsafe { core::arch::asm!("cli", "hlt", options(nomem, nostack)); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_range_excludes_syscall_and_spurious_vectors() {
        assert!(!is_apic_vector(0x2F));
        assert!(is_apic_vector(0x30));
        assert!(is_apic_vector(0x7F));
        assert!(!is_apic_vector(0x80));
        assert!(is_apic_vector(0x81));
        assert!(is_apic_vector(0xFE));
        assert!(!is_apic_vector(0xFF));
    }

    #[test]
    fn assembly_vector_count_matches_reserved_range() {
        let mut count = 0;
        for vector in APIC_VECTOR_START..=APIC_VECTOR_END {
            if is_apic_vector(vector) {
                count += 1;
            }
        }
        assert_eq!(count, APIC_VECTOR_COUNT);
    }
}
