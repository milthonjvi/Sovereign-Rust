//! x86_64 CPU exception entry and fatal dispatch.

use core::sync::atomic::{AtomicU64, Ordering};

use super::idt::{Idt, IdtEntry, IdtError, INTERRUPT_GATE};

core::arch::global_asm!(include_str!("../arch/x86_64/exceptions.S"));

unsafe extern "C" {
    static sovereign_exception_stub_table: [u64; 256];
}

/// Register snapshot and processor frame captured on exception entry.
///
/// For exceptions that push an error code, `error_code` is supplied by the
/// processor. Other vectors receive a synthetic zero from their entry stub.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(C)]
pub struct ExceptionFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rbp: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub vector: u64,
    pub error_code: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
}

/// Last fatal exception captured by the bootstrap CPU.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExceptionRecord {
    pub vector: u64,
    pub error_code: u64,
    pub instruction_pointer: u64,
}

static LAST_VECTOR: AtomicU64 = AtomicU64::new(u64::MAX);
static LAST_ERROR: AtomicU64 = AtomicU64::new(0);
static LAST_RIP: AtomicU64 = AtomicU64::new(0);

/// Installs a valid fatal fallback gate for every architectural vector.\n/// PIC IRQ gates may subsequently replace vectors 0x20..=0x2F.
///
/// # Safety
/// The caller must ensure the current CS selector names a valid 64-bit code
/// segment in the active GDT. The IDT must remain mapped and stable after load.
pub unsafe fn install_exception_gates(idt: &mut Idt) -> Result<(), IdtError> {
    let selector = Idt::current_code_selector();
    for vector in 0..256 {
        // SAFETY: The assembly object exports exactly 256 pointers in this table.
        let handler = unsafe { sovereign_exception_stub_table[vector] };
        // Dedicated IST stacks prevent catastrophic faults from reusing a
        // potentially corrupted or exhausted kernel stack.
        let ist = match vector {
            2 => 2,  // NMI
            8 => 1,  // Double fault
            18 => 3, // Machine check
            _ => 0,
        };
        let entry = IdtEntry::new(handler, selector, ist, INTERRUPT_GATE, 0)?;
        idt.set_gate(vector as u8, entry);
    }
    Ok(())
}

/// Returns the most recently captured fatal exception, if any.
pub fn last_exception() -> Option<ExceptionRecord> {
    let vector = LAST_VECTOR.load(Ordering::Acquire);
    if vector == u64::MAX {
        return None;
    }
    Some(ExceptionRecord {
        vector,
        error_code: LAST_ERROR.load(Ordering::Relaxed),
        instruction_pointer: LAST_RIP.load(Ordering::Relaxed),
    })
}

/// Common assembly entry. This function never returns.
#[unsafe(no_mangle)]
pub extern "C" fn sovereign_exception_dispatch(frame: *const ExceptionFrame) -> ! {
    // SAFETY: The entry stub passes a pointer to the register frame it just
    // pushed on the current kernel stack; dispatch is non-returning.
    let snapshot = unsafe { *frame };
    LAST_ERROR.store(snapshot.error_code, Ordering::Relaxed);
    LAST_RIP.store(snapshot.rip, Ordering::Relaxed);
    LAST_VECTOR.store(snapshot.vector, Ordering::Release);

    // Until a diagnostic console and recovery policy exist, stop deterministically.
    loop {
        // SAFETY: Exception dispatch is executing in ring 0 and must not return.
        unsafe { core::arch::asm!("cli", "hlt", options(nomem, nostack)); }
    }
}
