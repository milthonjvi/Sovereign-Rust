//! Legacy dual 8259A Programmable Interrupt Controller (PIC).
//!
//! This module provides controller programming and IRQ mask/EOI operations.
//! It does not install IDT entries or enable CPU interrupts. The caller must
//! keep interrupts disabled while remapping or changing controller state.

use crate::arch::x86_64::io;

const MASTER_COMMAND: u16 = 0x20;
const MASTER_DATA: u16 = 0x21;
const SLAVE_COMMAND: u16 = 0xA0;
const SLAVE_DATA: u16 = 0xA1;
const PIC_EOI: u8 = 0x20;
const ICW1_INIT: u8 = 0x10;
const ICW1_ICW4: u8 = 0x01;
const ICW4_8086: u8 = 0x01;
const ISR_READ: u8 = 0x0B;

/// Standard remapped vector offsets, leaving CPU exception vectors untouched.
pub const MASTER_VECTOR_OFFSET: u8 = 0x20;
pub const SLAVE_VECTOR_OFFSET: u8 = 0x28;

/// Errors returned by PIC operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PicError {
    InvalidVectorOffset,
    InvalidIrq,
    NotInitialized,
}


/// Check for a spurious legacy PIC IRQ without requiring a controller handle.
///
/// # Safety
/// The caller must own the legacy PIC command ports, have initialized the
/// dual PIC, and execute this from the serialized IRQ dispatch path with the
/// expected PIC remapping active. For spurious IRQ15 this sends the required
/// master-only EOI. A spurious IRQ7 requires no EOI.
pub unsafe fn acknowledge_spurious_irq(irq: u8) -> Result<bool, PicError> {
    match irq {
        7 => {
            // SAFETY: Guaranteed by the caller's PIC ownership contract.
            unsafe {
                io::out8(MASTER_COMMAND, ISR_READ);
                Ok(io::in8(MASTER_COMMAND) & (1 << 7) == 0)
            }
        }
        15 => {
            // SAFETY: Guaranteed by the caller's PIC ownership contract.
            unsafe {
                io::out8(SLAVE_COMMAND, ISR_READ);
                let spurious = io::in8(SLAVE_COMMAND) & (1 << 7) == 0;
                if spurious {
                    io::out8(MASTER_COMMAND, PIC_EOI);
                }
                Ok(spurious)
            }
        }
        0..=6 | 8..=14 => Ok(false),
        _ => Err(PicError::InvalidIrq),
    }
}

/// Cached mask state for the legacy master/slave PIC pair.
pub struct Pic8259 {
    master_mask: u8,
    slave_mask: u8,
    master_offset: u8,
    slave_offset: u8,
    initialized: bool,
}

impl Pic8259 {
    /// Creates an uninitialized controller handle.
    ///
    /// # Safety
    /// The caller must exclusively own I/O ports 0x20..=0x21 and 0xA0..=0xA1.
    pub const unsafe fn new() -> Self {
        Self {
            master_mask: 0xFF,
            slave_mask: 0xFF,
            master_offset: MASTER_VECTOR_OFFSET,
            slave_offset: SLAVE_VECTOR_OFFSET,
            initialized: false,
        }
    }

    /// Remaps both PICs and leaves every IRQ masked.
    ///
    /// # Safety
    /// Must run at CPL 0 with CPU interrupts disabled. The caller must ensure
    /// no other code accesses either PIC while this operation is in progress.
    pub unsafe fn initialize(&mut self, master_offset: u8, slave_offset: u8) -> Result<(), PicError> {
        if master_offset & 0x07 != 0
            || slave_offset & 0x07 != 0
            || master_offset < 0x20
            || slave_offset < 0x20
            || master_offset as u16 + 8 > 0x100
            || slave_offset as u16 + 8 > 0x100
            || (master_offset as u16) < (slave_offset as u16 + 8)
                && (slave_offset as u16) < (master_offset as u16 + 8)
        {
            return Err(PicError::InvalidVectorOffset);
        }

        // SAFETY: Caller guarantees exclusive PIC ownership and disabled IF.
        unsafe {
            io::out8(MASTER_COMMAND, ICW1_INIT | ICW1_ICW4);
            Self::io_wait();
            io::out8(SLAVE_COMMAND, ICW1_INIT | ICW1_ICW4);
            Self::io_wait();

            io::out8(MASTER_DATA, master_offset);
            Self::io_wait();
            io::out8(SLAVE_DATA, slave_offset);
            Self::io_wait();

            io::out8(MASTER_DATA, 1 << 2);
            Self::io_wait();
            io::out8(SLAVE_DATA, 2);
            Self::io_wait();

            io::out8(MASTER_DATA, ICW4_8086);
            Self::io_wait();
            io::out8(SLAVE_DATA, ICW4_8086);
            Self::io_wait();

            // Start fully masked. Restore prior masks only after explicit
            // unmask calls, preventing unexpected IRQ delivery during setup.
            io::out8(MASTER_DATA, 0xFF);
            io::out8(SLAVE_DATA, 0xFF);
        }

        self.master_mask = 0xFF;
        self.slave_mask = 0xFF;
        self.master_offset = master_offset;
        self.slave_offset = slave_offset;
        self.initialized = true;
        Ok(())
    }

    #[inline]
    fn io_wait() {
        // Port 0x80 is the conventional legacy delay port during PIC ICWs.
        // SAFETY: This is the standard x86 legacy I/O delay operation.
        unsafe { io::out8(0x80, 0) };
    }

    fn ensure_initialized(&self) -> Result<(), PicError> {
        if self.initialized { Ok(()) } else { Err(PicError::NotInitialized) }
    }

    /// Unmasks an IRQ line. Unmasking a slave IRQ also unmasks master IRQ2.
    pub fn unmask(&mut self, irq: u8) -> Result<(), PicError> {
        self.ensure_initialized()?;
        if irq >= 16 { return Err(PicError::InvalidIrq); }

        if irq < 8 {
            self.master_mask &= !(1 << irq);
        } else {
            self.slave_mask &= !(1 << (irq - 8));
            self.master_mask &= !(1 << 2);
        }

        // SAFETY: This handle owns both PICs and maintains the cached masks.
        unsafe {
            io::out8(MASTER_DATA, self.master_mask);
            io::out8(SLAVE_DATA, self.slave_mask);
        }
        Ok(())
    }

    /// Masks an IRQ line.
    pub fn mask(&mut self, irq: u8) -> Result<(), PicError> {
        self.ensure_initialized()?;
        if irq >= 16 { return Err(PicError::InvalidIrq); }

        if irq < 8 {
            self.master_mask |= 1 << irq;
        } else {
            self.slave_mask |= 1 << (irq - 8);
        }

        // SAFETY: This handle owns both PICs and maintains the cached masks.
        unsafe {
            io::out8(MASTER_DATA, self.master_mask);
            io::out8(SLAVE_DATA, self.slave_mask);
        }
        Ok(())
    }

    /// Sends end-of-interrupt to the appropriate controller(s).
    pub fn end_of_interrupt(&self, irq: u8) -> Result<(), PicError> {
        self.ensure_initialized()?;
        if irq >= 16 { return Err(PicError::InvalidIrq); }

        // SAFETY: EOI is written only to the PICs owned by this handle.
        unsafe {
            if irq >= 8 { io::out8(SLAVE_COMMAND, PIC_EOI); }
            io::out8(MASTER_COMMAND, PIC_EOI);
        }
        Ok(())
    }

    /// Returns the vector assigned to an IRQ line.
    pub fn vector_for_irq(&self, irq: u8) -> Result<u8, PicError> {
        self.ensure_initialized()?;
        match irq {
            0..=7 => Ok(self.master_offset + irq),
            8..=15 => Ok(self.slave_offset + irq - 8),
            _ => Err(PicError::InvalidIrq),
        }
    }

    /// Detects spurious IRQ7/IRQ15 using the PIC in-service registers.
    ///
    /// A spurious IRQ7 requires no EOI. A spurious IRQ15 requires an EOI to
    /// the master PIC only because the cascade request reached the master.
    /// Returns false for ordinary IRQ lines and genuine IRQ7/IRQ15 requests.
    pub fn acknowledge_spurious(&self, irq: u8) -> Result<bool, PicError> {
        self.ensure_initialized()?;
        match irq {
            7 => {
                // SAFETY: This handle owns the master PIC command port.
                unsafe {
                    io::out8(MASTER_COMMAND, ISR_READ);
                    Ok(io::in8(MASTER_COMMAND) & (1 << 7) == 0)
                }
            }
            15 => {
                // SAFETY: This handle owns both PIC command ports.
                unsafe {
                    io::out8(SLAVE_COMMAND, ISR_READ);
                    let spurious = io::in8(SLAVE_COMMAND) & (1 << 7) == 0;
                    if spurious {
                        // The master accepted the cascade IRQ; acknowledge it.
                        io::out8(MASTER_COMMAND, PIC_EOI);
                    }
                    Ok(spurious)
                }
            }
            0..=6 | 8..=14 => Ok(false),
            _ => Err(PicError::InvalidIrq),
        }
    }

    /// Reads the in-service register from both PICs.
    pub fn in_service(&self) -> Result<u16, PicError> {
        self.ensure_initialized()?;
        // SAFETY: OCW3 selects ISR reads; command ports belong to this handle.
        unsafe {
            io::out8(MASTER_COMMAND, ISR_READ);
            io::out8(SLAVE_COMMAND, ISR_READ);
            let master = io::in8(MASTER_COMMAND) as u16;
            let slave = io::in8(SLAVE_COMMAND) as u16;
            Ok((slave << 8) | master)
        }
    }

    pub const fn masks(&self) -> (u8, u8) {
        (self.master_mask, self.slave_mask)
    }

    pub const fn is_initialized(&self) -> bool {
        self.initialized
    }
}
