//! x86_64 Interrupt Descriptor Table (IDT).
//!
//! This module owns descriptor encoding and the privileged LIDT operation.
//! It deliberately does not provide entry stubs or enable CPU interrupts.
//! Install valid assembly entry points before loading the table.

use core::mem::size_of;

/// Number of entries in the architectural x86_64 IDT.
pub const IDT_ENTRIES: usize = 256;

/// Gate type for an interrupt gate (clears IF on entry).
pub const INTERRUPT_GATE: u8 = 0x0E;
/// Gate type for a trap gate (preserves IF on entry).
pub const TRAP_GATE: u8 = 0x0F;

/// Errors produced while constructing IDT entries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdtError {
    NullHandler,
    NullCodeSelector,
    InvalidPrivilegeLevel,
}

/// A 16-byte x86_64 IDT gate descriptor.
#[derive(Clone, Copy)]
#[repr(C, packed)]
pub struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    attributes: u8,
    offset_middle: u16,
    offset_high: u32,
    reserved: u32,
}

impl IdtEntry {
    const MISSING: Self = Self {
        offset_low: 0,
        selector: 0,
        ist: 0,
        attributes: 0,
        offset_middle: 0,
        offset_high: 0,
        reserved: 0,
    };

    /// Builds a present ring-0/ring-3 gate descriptor.
    ///
    /// `ist` selects an Interrupt Stack Table slot (0 disables IST, 1..=7
    /// select IST entries). DPL controls which privilege levels may invoke
    /// the gate using INT n.
    pub fn new(
        handler: u64,
        selector: u16,
        ist: u8,
        gate_type: u8,
        dpl: u8,
    ) -> Result<Self, IdtError> {
        if handler == 0 { return Err(IdtError::NullHandler); }
        if selector == 0 { return Err(IdtError::NullCodeSelector); }
        if ist > 7 || dpl > 3 || (gate_type != INTERRUPT_GATE && gate_type != TRAP_GATE) {
            return Err(IdtError::InvalidPrivilegeLevel);
        }

        let attributes = 0x80 | (dpl << 5) | gate_type;
        Ok(Self {
            offset_low: handler as u16,
            selector,
            ist,
            attributes,
            offset_middle: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            reserved: 0,
        })
    }

    /// Returns whether this descriptor is marked present.
    pub fn is_present(&self) -> bool {
        // Copy the packed field to a local; never create an unaligned reference.
        let attributes = self.attributes;
        attributes & 0x80 != 0
    }

    /// Reconstructs the 64-bit handler address.
    pub fn handler_address(&self) -> u64 {
        let low = self.offset_low as u64;
        let middle = self.offset_middle as u64;
        let high = self.offset_high as u64;
        low | (middle << 16) | (high << 32)
    }
}

/// Complete 256-entry IDT. Keep the loaded instance at a stable address.
#[repr(C, align(16))]
pub struct Idt {
    entries: [IdtEntry; IDT_ENTRIES],
}

impl Idt {
    /// Creates an IDT with every vector initially absent.
    pub const fn new() -> Self {
        Self { entries: [IdtEntry::MISSING; IDT_ENTRIES] }
    }

    /// Installs or replaces one vector descriptor.
    pub fn set_gate(&mut self, vector: u8, entry: IdtEntry) {
        self.entries[vector as usize] = entry;
    }

    /// Clears a vector descriptor.
    pub fn clear_gate(&mut self, vector: u8) {
        self.entries[vector as usize] = IdtEntry::MISSING;
    }

    /// Checks whether a vector has a present handler.
    pub fn has_gate(&self, vector: u8) -> bool {
        self.entries[vector as usize].is_present()
    }

    /// Counts present descriptors, useful during early initialization checks.
    pub fn present_count(&self) -> usize {
        self.entries.iter().filter(|entry| entry.is_present()).count()
    }

    /// Loads this IDT into the current CPU.
    ///
    /// # Safety
    /// Must execute at CPL 0. The IDT must remain mapped, readable, and at a
    /// stable address for as long as the CPU may use it. Every exception or
    /// interrupt vector that can occur must have a valid entry stub installed.
    /// Loading an incomplete table can turn a later fault into a double fault.
    pub unsafe fn load(&'static self) {
        let pointer = IdtPointer {
            limit: (size_of::<IdtEntry>() * IDT_ENTRIES - 1) as u16,
            base: self.entries.as_ptr() as u64,
        };
        // SAFETY: Caller guarantees CPL 0 and the IDT's lifetime/mapping.
        unsafe { core::arch::asm!("lidt [{}]", in(reg) &pointer, options(readonly, nostack, preserves_flags)); }
    }

    /// Reads the currently active CS selector for gate construction.
    ///
    /// This avoids assuming a selector value from a particular bootloader GDT.
    #[inline]
    pub fn current_code_selector() -> u16 {
        let selector: u16;
        // SAFETY: Reading CS is unprivileged and has no side effects.
        unsafe { core::arch::asm!("mov {0:x}, cs", out(reg) selector, options(nomem, nostack, preserves_flags)); }
        selector
    }
}

impl Default for Idt {
    fn default() -> Self { Self::new() }
}

#[repr(C, packed)]
struct IdtPointer {
    limit: u16,
    base: u64,
}

const _: [(); 16] = [(); size_of::<IdtEntry>()];
const _: [(); 10] = [(); size_of::<IdtPointer>()];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_decodes_handler_address() {
        let address = 0xFFFF_8000_1234_5678;
        let entry = IdtEntry::new(address, 0x08, 0, INTERRUPT_GATE, 0).unwrap();
        assert!(entry.is_present());
        assert_eq!(entry.handler_address(), address);
    }

    #[test]
    fn rejects_invalid_gate_inputs() {
        assert_eq!(IdtEntry::new(0, 8, 0, INTERRUPT_GATE, 0), Err(IdtError::NullHandler));
        assert_eq!(IdtEntry::new(1, 0, 0, INTERRUPT_GATE, 0), Err(IdtError::NullCodeSelector));
        assert_eq!(IdtEntry::new(1, 8, 0, INTERRUPT_GATE, 4), Err(IdtError::InvalidPrivilegeLevel));
        assert_eq!(IdtEntry::new(1, 8, 8, INTERRUPT_GATE, 0), Err(IdtError::InvalidPrivilegeLevel));
    }

    #[test]
    fn idt_starts_empty_and_accepts_gate() {
        let mut idt = Idt::new();
        assert_eq!(idt.present_count(), 0);
        let entry = IdtEntry::new(0xFFFF_8000_0000_1000, 8, 0, INTERRUPT_GATE, 0).unwrap();
        idt.set_gate(14, entry);
        assert!(idt.has_gate(14));
        assert_eq!(idt.present_count(), 1);
        idt.clear_gate(14);
        assert!(!idt.has_gate(14));
    }
}
