//! Timer interrupt registration and deferred-work signaling.
//!
//! The hard IRQ path performs one atomic store only. Expiration processing,
//! queue locking, and task wakeup handoff must run later in task/softirq context.
//! The common APIC dispatcher remains responsible for EOI.
//!
//! The notification flag is currently a bootstrap-CPU mechanism. Do not enable
//! this single global signal for multiple APIC timer CPUs until it is replaced
//! by per-CPU pending state and a corresponding deferred worker on each CPU.

use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use crate::{
    arch::x86_64::{msr, x2apic::X2Apic},
    interrupts::dispatch,
};

static TIMER_WORK_PENDING: AtomicBool = AtomicBool::new(false);
const NO_TIMER_VECTOR: u8 = 0;
static TIMER_VECTOR: AtomicU8 = AtomicU8::new(NO_TIMER_VECTOR);

/// Hard-IRQ callback: signal deferred processing only.
extern "C" fn timer_interrupt_handler(_vector: u8) {
    TIMER_WORK_PENDING.store(true, Ordering::Release);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimerInterruptError {
    InvalidVector,
    LocalApicUnavailable,
    NotInitialized,
    AlreadyRegistered,
    NotRegistered,
    HardwareUnavailable,
}

/// Configure the BSP x2APIC timer LVT in TSC-deadline mode and register its
/// deferred-work handler. The LVT remains masked until explicitly unmasked.
///
/// # Safety
/// BSP only, CPL 0, IF clear. The caller must initialize the matching
/// TscDeadlineQueue before calling unmask_timer_interrupt().
pub unsafe fn register_timer_interrupt(vector: u8) -> Result<(), TimerInterruptError> {
    if !crate::interrupts::apic::is_apic_vector(vector) {
        return Err(TimerInterruptError::InvalidVector);
    }
    if !crate::interrupts::init::is_initialized() {
        return Err(TimerInterruptError::NotInitialized);
    }
    if crate::interrupts::init::local_apic_mode()
        != crate::interrupts::init::LocalApicMode::X2Apic
    {
        return Err(TimerInterruptError::LocalApicUnavailable);
    }
    if TIMER_VECTOR.load(Ordering::Acquire) != NO_TIMER_VECTOR {
        return Err(TimerInterruptError::AlreadyRegistered);
    }

    // SAFETY: Bootstrap initialized x2APIC on this CPU; caller guarantees CPL 0/IF clear.
    let mut local_apic = unsafe { X2Apic::new() };
    local_apic.configure_tsc_deadline_timer(vector)
        .map_err(|_| TimerInterruptError::HardwareUnavailable)?;

    dispatch::register(vector, timer_interrupt_handler)
        .map_err(|_| TimerInterruptError::AlreadyRegistered)?;
    TIMER_WORK_PENDING.store(false, Ordering::Release);
    TIMER_VECTOR.store(vector, Ordering::Release);
    Ok(())
}

/// Unmask the registered timer after its deadline queue is initialized.
///
/// # Safety
/// BSP only, CPL 0, IF clear; the matching queue must be ready.
pub unsafe fn unmask_timer_interrupt() -> Result<(), TimerInterruptError> {
    let vector = TIMER_VECTOR.load(Ordering::Acquire);
    if vector == NO_TIMER_VECTOR || !dispatch::is_registered(vector) {
        return Err(TimerInterruptError::NotRegistered);
    }
    // SAFETY: Same BSP/x2APIC context as registration.
    let mut local_apic = unsafe { X2Apic::new() };
    local_apic.unmask_tsc_deadline_timer()
        .map_err(|_| TimerInterruptError::HardwareUnavailable)
}

/// Mask the timer, disarm its TSC comparator, and unregister its handler.
///
/// # Safety
/// BSP only, CPL 0, IF clear, with no handler in flight and no remaining
/// users of the associated timer queue.
pub unsafe fn unregister_timer_interrupt() -> Result<(), TimerInterruptError> {
    let vector = TIMER_VECTOR.load(Ordering::Acquire);
    if vector == NO_TIMER_VECTOR || !dispatch::is_registered(vector) {
        return Err(TimerInterruptError::NotRegistered);
    }

    // SAFETY: Caller guarantees serialized teardown on the owning CPU.
    let mut local_apic = unsafe { X2Apic::new() };
    local_apic.mask_timer();
    // SAFETY: x2APIC initialization and TSC-deadline support were validated.
    unsafe { msr::write(crate::drivers::timer::IA32_TSC_DEADLINE, 0) };
    dispatch::unregister(vector, timer_interrupt_handler)
        .map_err(|_| TimerInterruptError::NotRegistered)?;
    TIMER_WORK_PENDING.store(false, Ordering::Release);
    TIMER_VECTOR.store(NO_TIMER_VECTOR, Ordering::Release);
    Ok(())
}

/// Atomically consume the deferred-work signal.
#[inline]
pub fn take_timer_work_pending() -> bool {
    TIMER_WORK_PENDING.swap(false, Ordering::AcqRel)
}

/// Query whether deferred timer processing is pending.
#[inline]
pub fn timer_work_pending() -> bool {
    TIMER_WORK_PENDING.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_vectors_outside_the_apic_timer_range() {
        assert_eq!(
            unsafe { register_timer_interrupt(0x20) },
            Err(TimerInterruptError::InvalidVector)
        );
        assert_eq!(
            unsafe { register_timer_interrupt(0x80) },
            Err(TimerInterruptError::InvalidVector)
        );
        assert_eq!(
            unsafe { register_timer_interrupt(0xFF) },
            Err(TimerInterruptError::InvalidVector)
        );
    }

    #[test]
    fn pending_signal_is_consumed_once() {
        TIMER_WORK_PENDING.store(true, Ordering::Release);
        assert!(take_timer_work_pending());
        assert!(!take_timer_work_pending());
    }
}
