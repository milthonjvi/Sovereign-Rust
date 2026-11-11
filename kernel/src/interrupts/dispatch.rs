//! Lock-free vector-to-handler dispatch table for interrupt entry stubs.
//!
//! Assembly entry stubs can call `dispatch_vector(vector)` after saving the
//! interrupted register state. Handler registration is performed during early
//! initialization, before the corresponding interrupt source is unmasked.

use core::sync::atomic::{AtomicUsize, Ordering};

/// Interrupt callback ABI used by the low-level entry stubs.
///
/// Handlers execute in interrupt context: they must be bounded, nonblocking,
/// must not unwind, and must not assume that task scheduling is available.
pub type InterruptHandler = extern "C" fn(vector: u8);

/// Number of architectural interrupt vectors.
pub const VECTOR_COUNT: usize = 256;

/// A vector is unhandled when its table entry is zero.
static HANDLERS: [AtomicUsize; VECTOR_COUNT] =
    [const { AtomicUsize::new(0) }; VECTOR_COUNT];

/// Errors returned by handler registration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandlerError {
    /// The vector already has a registered handler.
    AlreadyRegistered,
    /// The vector does not have the expected registered handler.
    NotRegistered,
}

/// Registers a handler for one vector.
///
/// Registration should happen with the corresponding hardware source masked.
/// The function pointer is published with Release ordering before the source
/// can be unmasked.
pub fn register(vector: u8, handler: InterruptHandler) -> Result<(), HandlerError> {
    let slot = &HANDLERS[vector as usize];
    let address = handler as usize;
    match slot.compare_exchange(0, address, Ordering::Release, Ordering::Relaxed) {
        Ok(_) => Ok(()),
        Err(_) => Err(HandlerError::AlreadyRegistered),
    }
}

/// Removes a handler only if it still matches the supplied function.
///
/// The caller must mask the interrupt source and ensure no CPU is currently
/// executing the handler before unregistering it.
pub fn unregister(vector: u8, handler: InterruptHandler) -> Result<(), HandlerError> {
    let slot = &HANDLERS[vector as usize];
    let address = handler as usize;
    match slot.compare_exchange(address, 0, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => Ok(()),
        Err(_) => Err(HandlerError::NotRegistered),
    }
}

/// Dispatches one vector to its registered callback.
///
/// Returns `true` when a handler was present. This function deliberately does
/// not send PIC/APIC EOI: the entry/interrupt-controller layer owns that policy.
#[inline]
pub fn dispatch_vector(vector: u8) -> bool {
    let address = HANDLERS[vector as usize].load(Ordering::Acquire);
    if address == 0 {
        return false;
    }

    // SAFETY: Only register() publishes a nonzero value, and it stores a
    // function pointer with this exact ABI. The handler lifetime is static.
    let handler: InterruptHandler = unsafe { core::mem::transmute(address) };
    handler(vector);
    true
}

/// Reports whether a vector currently has a registered callback.
#[inline]
pub fn is_registered(vector: u8) -> bool {
    HANDLERS[vector as usize].load(Ordering::Acquire) != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::AtomicUsize;

    static LAST_VECTOR: AtomicUsize = AtomicUsize::new(usize::MAX);

    extern "C" fn test_handler(vector: u8) {
        LAST_VECTOR.store(vector as usize, Ordering::Relaxed);
    }

    #[test]
    fn registration_dispatch_and_removal_follow_publication_order() {
        const VECTOR: u8 = 0xF1;
        LAST_VECTOR.store(usize::MAX, Ordering::Relaxed);

        assert!(!is_registered(VECTOR));
        assert_eq!(register(VECTOR, test_handler), Ok(()));
        assert!(is_registered(VECTOR));
        assert!(dispatch_vector(VECTOR));
        assert_eq!(LAST_VECTOR.load(Ordering::Relaxed), VECTOR as usize);
        assert_eq!(unregister(VECTOR, test_handler), Ok(()));
        assert!(!is_registered(VECTOR));
        assert!(!dispatch_vector(VECTOR));
    }

    #[test]
    fn duplicate_registration_is_rejected() {
        const VECTOR: u8 = 0xF2;
        assert_eq!(register(VECTOR, test_handler), Ok(()));
        assert_eq!(register(VECTOR, test_handler), Err(HandlerError::AlreadyRegistered));
        assert_eq!(unregister(VECTOR, test_handler), Ok(()));
    }
}
