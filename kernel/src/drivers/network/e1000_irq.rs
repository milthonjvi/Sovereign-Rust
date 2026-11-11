//! E1000 interrupt bridge for the kernel's lock-free APIC vector dispatcher.
//!
//! The PCI/platform layer must configure and mask the device's MSI/MSI-X route
//! before installing this bridge. The callback only records a pending vector;
//! packet processing and descriptor reclamation remain in bounded driver
//! context, never in the hard-IRQ callback.

use core::sync::atomic::{AtomicU64, Ordering};

use super::e1000::{REG_IMC, REG_IMS};
use super::e1000_hw::E1000Registers;

pub const E1000_ICR_TXDW: u32 = 1 << 0;
pub const E1000_ICR_LSC: u32 = 1 << 2;
pub const E1000_ICR_RXO: u32 = 1 << 6;
pub const E1000_ICR_RXT0: u32 = 1 << 7;

/// Causes that request deferred processing instead of doing work in IRQ context.
pub const E1000_DEFAULT_INTERRUPT_MASK: u32 =
    E1000_ICR_TXDW | E1000_ICR_LSC | E1000_ICR_RXO | E1000_ICR_RXT0;

static PENDING_VECTORS: [AtomicU64; 4] = [
    AtomicU64::new(0), AtomicU64::new(0),
    AtomicU64::new(0), AtomicU64::new(0),
];

/// One registered E1000 interrupt vector. The vector is shared globally with
/// the dispatcher, so use a unique APIC vector for each independently handled NIC.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct E1000Interrupt {
    vector: u8,
    mask: u32,
    enabled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum E1000InterruptError<E> {
    InvalidMask,
    Register(E),
    VectorRegistration,
    VectorRemoval,
}

impl E1000Interrupt {
    /// Register a deferred-work callback while the device's MSI/MSI-X source
    /// remains masked. The platform must already have routed that source to
    /// `vector`; this function does not modify PCI MSI configuration.
    ///
    /// # Safety
    /// The vector must be a valid, exclusively assigned APIC vector and the
    /// corresponding device interrupt source must remain masked until `enable`.
    pub unsafe fn install(vector: u8, mask: u32)
        -> Result<Self, E1000InterruptError<core::convert::Infallible>>
    {
        if mask == 0 || mask & !E1000_DEFAULT_INTERRUPT_MASK != 0 {
            return Err(E1000InterruptError::InvalidMask);
        }
        // SAFETY: forwarded from the caller's vector-ownership contract.
        unsafe {
            crate::interrupts::apic::register_vector(vector, e1000_interrupt_handler)
        }.map_err(|_| E1000InterruptError::VectorRegistration)?;
        Ok(Self { vector, mask, enabled: false })
    }

    /// Enable selected E1000 causes after the platform has completed MSI routing.
    pub fn enable<R: E1000Registers>(&mut self, registers: &mut R)
        -> Result<(), E1000InterruptError<R::Error>>
    {
        registers.write32(REG_IMC, u32::MAX).map_err(E1000InterruptError::Register)?;
        registers.write32(REG_IMS, self.mask).map_err(E1000InterruptError::Register)?;
        self.enabled = true;
        Ok(())
    }

    /// Mask device causes before removing the vector callback.
    pub fn disable<R: E1000Registers>(&mut self, registers: &mut R)
        -> Result<(), E1000InterruptError<R::Error>>
    {
        registers.write32(REG_IMC, u32::MAX).map_err(E1000InterruptError::Register)?;
        self.enabled = false;
        Ok(())
    }

    /// Consume the coalesced notification. The caller should then inspect the
    /// E1000 cause register and service RX/TX queues in deferred context.
    pub fn take_pending(&self) -> bool {
        let word = self.vector as usize / 64;
        let bit = 1u64 << (self.vector as usize % 64);
        PENDING_VECTORS[word].fetch_and(!bit, Ordering::AcqRel) & bit != 0
    }

    /// Remove the callback after the device is masked and in-flight IRQs drained.
    ///
    /// # Safety
    /// No CPU may still be executing the registered callback, and the device
    /// interrupt source must already be masked.
    pub unsafe fn uninstall(self)
        -> Result<(), E1000InterruptError<core::convert::Infallible>>
    {
        // SAFETY: caller guarantees source masking and drained in-flight delivery.
        unsafe {
            crate::interrupts::apic::unregister_vector(self.vector, e1000_interrupt_handler)
        }.map_err(|_| E1000InterruptError::VectorRemoval)
    }

    pub const fn vector(&self) -> u8 { self.vector }
    pub const fn is_enabled(&self) -> bool { self.enabled }
}

extern "C" fn e1000_interrupt_handler(vector: u8) {
    let word = vector as usize / 64;
    let bit = 1u64 << (vector as usize % 64);
    PENDING_VECTORS[word].fetch_or(bit, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interrupt_callback_only_sets_deferred_pending_bit() {
        let vector = 0xB4;
        let word = vector as usize / 64;
        let bit = 1u64 << (vector as usize % 64);
        PENDING_VECTORS[word].fetch_and(!bit, Ordering::Relaxed);
        e1000_interrupt_handler(vector);
        assert!(PENDING_VECTORS[word].load(Ordering::Acquire) & bit != 0);
        assert!(PENDING_VECTORS[word].fetch_and(!bit, Ordering::AcqRel) & bit != 0);
    }

    #[test]
    fn interrupt_mask_rejects_unknown_causes() {
        assert_eq!(E1000_DEFAULT_INTERRUPT_MASK,
            E1000_ICR_TXDW | E1000_ICR_LSC | E1000_ICR_RXO | E1000_ICR_RXT0);
        assert_eq!(E1000_DEFAULT_INTERRUPT_MASK & (1 << 31), 0);
    }
}
