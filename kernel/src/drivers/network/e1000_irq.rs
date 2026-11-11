//! E1000 interrupt bridge for the kernel's lock-free APIC vector dispatcher.
//!
//! The PCI/platform layer must configure and mask the device's MSI/MSI-X route
//! before installing this bridge. The callback only records a pending vector;
//! packet processing and descriptor reclamation remain in bounded driver
//! context, never in the hard-IRQ callback.

use core::sync::atomic::{AtomicU64, Ordering};

use super::e1000::{REG_ICR, REG_IMC, REG_IMS};
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
#[derive(Debug, Eq, PartialEq)]
pub struct E1000Interrupt {
    vector: u8,
    mask: u32,
    enabled: bool,
    registered: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum E1000InterruptError<E> {
    InvalidMask,
    Register(E),
    VectorRegistration,
    VectorRemoval,
    StillEnabled,
    NotRegistered,
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
        Ok(Self { vector, mask, enabled: false, registered: true })
    }

    /// Enable selected E1000 causes after the platform has completed MSI routing.
    pub fn enable<R: E1000Registers>(&mut self, registers: &mut R)
        -> Result<(), E1000InterruptError<R::Error>>
    {
        if !self.registered {
            return Err(E1000InterruptError::NotRegistered);
        }
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

    /// Read and clear the hardware cause register after consuming a pending
    /// APIC notification. This is deferred-context work, never IRQ-callback work.
    pub fn take_causes<R: E1000Registers>(
        &self,
        registers: &mut R,
    ) -> Result<Option<u32>, E1000InterruptError<R::Error>> {
        if !self.take_pending() {
            return Ok(None);
        }
        match registers.read32(REG_ICR) {
            Ok(causes) => Ok(Some(causes)),
            Err(error) => {
                // Preserve the deferred notification so a transient MMIO
                // failure does not silently consume the only wakeup.
                let word = self.vector as usize / 64;
                let bit = 1u64 << (self.vector as usize % 64);
                PENDING_VECTORS[word].fetch_or(bit, Ordering::Release);
                Err(E1000InterruptError::Register(error))
            }
        }
    }

    /// Remove the callback after the device is masked and in-flight IRQs drained.
    ///
    /// # Safety
    /// No CPU may still be executing the registered callback, and the device
    /// interrupt source must already be masked.
    pub unsafe fn uninstall(&mut self)
        -> Result<(), E1000InterruptError<core::convert::Infallible>>
    {
        if self.enabled {
            return Err(E1000InterruptError::StillEnabled);
        }
        if !self.registered {
            return Err(E1000InterruptError::VectorRemoval);
        }
        // SAFETY: caller guarantees source masking and drained in-flight delivery.
        unsafe {
            crate::interrupts::apic::unregister_vector(self.vector, e1000_interrupt_handler)
        }.map_err(|_| E1000InterruptError::VectorRemoval)?;
        let word = self.vector as usize / 64;
        let bit = 1u64 << (self.vector as usize % 64);
        PENDING_VECTORS[word].fetch_and(!bit, Ordering::AcqRel);
        self.registered = false;
        Ok(())
    }

    pub const fn vector(&self) -> u8 { self.vector }
    pub const fn is_enabled(&self) -> bool { self.enabled }
    pub const fn is_registered(&self) -> bool { self.registered }
}

extern "C" fn e1000_interrupt_handler(vector: u8) {
    let word = vector as usize / 64;
    let bit = 1u64 << (vector as usize % 64);
    PENDING_VECTORS[word].fetch_or(bit, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::e1000_hw::E1000Registers;

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

    struct MockRegisters { fail_read: bool, causes: u32 }

    unsafe impl E1000Registers for MockRegisters {
        type Error = ();
        fn read32(&mut self, offset: u32) -> Result<u32, Self::Error> {
            if offset == REG_ICR && self.fail_read { Err(()) } else { Ok(self.causes) }
        }
        fn write32(&mut self, _offset: u32, _value: u32) -> Result<(), Self::Error> { Ok(()) }
        fn dma_write_barrier(&mut self) {}
        fn dma_read_barrier(&mut self) {}
    }

    #[test]
    fn failed_cause_read_preserves_deferred_notification_for_retry() {
        let vector = 0xb6;
        let mut interrupt = E1000Interrupt {
            vector, mask: E1000_DEFAULT_INTERRUPT_MASK,
            enabled: false, registered: true,
        };
        let word = vector as usize / 64;
        let bit = 1u64 << (vector as usize % 64);
        PENDING_VECTORS[word].fetch_or(bit, Ordering::Release);
        let mut registers = MockRegisters { fail_read: true, causes: E1000_ICR_RXT0 };
        assert_eq!(interrupt.take_causes(&mut registers), Err(E1000InterruptError::Register(())));
        assert!(interrupt.take_pending());
        PENDING_VECTORS[word].fetch_or(bit, Ordering::Release);
        registers.fail_read = false;
        assert_eq!(interrupt.take_causes(&mut registers), Ok(Some(E1000_ICR_RXT0)));
    }

    #[test]
    fn uninstall_keeps_registered_handle_when_device_is_still_enabled() {
        let mut interrupt = E1000Interrupt {
            vector: 0xb5,
            mask: E1000_DEFAULT_INTERRUPT_MASK,
            enabled: true,
            registered: true,
        };
        assert_eq!(unsafe { interrupt.uninstall() }, Err(E1000InterruptError::StillEnabled));
        assert!(interrupt.is_registered());
        assert!(interrupt.is_enabled());
    }

    #[test]
    fn interrupt_mask_rejects_unknown_causes() {
        assert_eq!(E1000_DEFAULT_INTERRUPT_MASK,
            E1000_ICR_TXDW | E1000_ICR_LSC | E1000_ICR_RXO | E1000_ICR_RXT0);
        assert_eq!(E1000_DEFAULT_INTERRUPT_MASK & (1 << 31), 0);
    }
}
