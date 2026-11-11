//! Legacy PIC IRQ gate installation and dispatch bridge.

use super::dispatch::dispatch_vector;
use super::exceptions::ExceptionFrame;
use super::idt::{Idt, IdtEntry, IdtError, INTERRUPT_GATE};

core::arch::global_asm!(include_str!("../arch/x86_64/irq.S"));

unsafe extern "C" {
    static sovereign_irq_stub_table: [u64; 16];
}

/// Installs the 16 legacy PIC IRQ gates at the requested vector offset.
///
/// The offset is fixed at 0x20 because the entry stubs encode vectors 0x20..=0x2F.
///
/// # Safety
/// The active CS selector must name a valid 64-bit code segment. The vector
/// range must not overlap another installed subsystem. Keep the IDT mapped
/// and stable after loading it.
pub unsafe fn install_irq_gates(
    idt: &mut Idt,
    vector_offset: u8,
) -> Result<(), IrqGateError> {
    // These assembly stubs encode vectors 0x20..=0x2F directly.
    if vector_offset != 0x20 {
        return Err(IrqGateError::InvalidVectorOffset);
    }

    let selector = Idt::current_code_selector();
    for irq in 0..16 {
        // SAFETY: The assembly object exports exactly 16 stub addresses.
        let handler = unsafe { sovereign_irq_stub_table[irq] };
        let entry = IdtEntry::new(handler, selector, 0, INTERRUPT_GATE, 0)
            .map_err(IrqGateError::InvalidGate)?;
        idt.set_gate(vector_offset + irq as u8, entry);
    }
    Ok(())
}

/// Errors returned while installing IRQ gates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IrqGateError {
    InvalidVectorOffset,
    InvalidGate(IdtError),
}

/// Common assembly entry for remapped legacy PIC IRQs.
///
/// The registered callback is responsible for acknowledging its interrupt
/// controller before returning. For the 8259 PIC this means sending EOI,
/// including to the slave and master for IRQs 8-15.
#[unsafe(no_mangle)]
pub extern "C" fn sovereign_irq_dispatch(frame: *const ExceptionFrame) {
    // SAFETY: Assembly passes a pointer to the register frame it just pushed.
    let snapshot = unsafe { *frame };
    let Ok(vector) = u8::try_from(snapshot.vector) else {
        fatal_unhandled_irq();
    };

    if !dispatch_vector(vector) {
        fatal_unhandled_irq();
    }
}

fn fatal_unhandled_irq() -> ! {
    loop {
        // SAFETY: An unhandled IRQ cannot safely return without controller EOI.
        unsafe { core::arch::asm!("cli", "hlt", options(nomem, nostack)); }
    }
}
