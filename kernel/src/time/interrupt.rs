//! Timer interrupt registration and deferred-work signaling.
//!
//! The hard IRQ path performs one atomic store only. Expiration processing,
//! queue locking, and task wakeup handoff must run later in task/softirq context.
//! The common APIC dispatcher remains responsible for EOI.
//!
//! The notification flag is currently a bootstrap-CPU mechanism. Do not enable
//! this single global signal for multiple APIC timer CPUs until it is replaced
//! by per-CPU pending state and a corresponding deferred worker on each CPU.

use core::sync::atomic::{AtomicBool, Ordering};

use crate::interrupts::dispatch;

/// One timer interrupt has arrived and deferred expiration processing is due.
static TIMER_WORK_PENDING: AtomicBool = AtomicBool::new(false);

/// APIC interrupt callback. Never locks, allocates, logs, or runs callbacks.
extern "C" fn timer_interrupt_handler(_vector: u8) {
    TIMER_WORK_PENDING.store(true, Ordering::Release);
}

/// Errors while connecting the timer vector to the shared dispatcher.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimerInterruptError {
    InvalidVector,
    LocalApicUnavailable,
    AlreadyRegistered,
}

/// Register the timer callback while the hardware timer source is masked.
///
/// The caller must have installed the corresponding IDT gate and must keep
/// the device source masked until this call succeeds. The APIC entry path
/// sends EOI after this callback returns.
pub fn register_timer_interrupt(vector: u8) -> Result<(), TimerInterruptError> {
    if !crate::interrupts::apic::is_apic_vector(vector) {
        return Err(TimerInterruptError::InvalidVector);
    }
    if crate::interrupts::init::local_apic_mode()
        != crate::interrupts::init::LocalApicMode::X2Apic
    {
        return Err(TimerInterruptError::LocalApicUnavailable);
    }
    dispatch::register(vector, timer_interrupt_handler)
        .map_err(|_| TimerInterruptError::AlreadyRegistered)
}

/// Atomically consume the deferred-work signal.
///
/// A true result means the caller should drain expired timers and process
/// wakeups. Re-arm the hardware comparator from the queue's next deadline
/// after processing, then check this flag again to close the arrival race.
#[inline]
pub fn take_timer_work_pending() -> bool {
    TIMER_WORK_PENDING.swap(false, Ordering::AcqRel)
}

/// Query whether deferred timer processing is pending without consuming it.
#[inline]
pub fn timer_work_pending() -> bool {
    TIMER_WORK_PENDING.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_vector_registration_signals_deferred_work() {
        const VECTOR: u8 = 0xF0;
        assert!(!take_timer_work_pending());
        register_timer_interrupt(VECTOR).unwrap();
        assert!(dispatch::dispatch_vector(VECTOR));
        assert!(take_timer_work_pending());
        assert!(!take_timer_work_pending());
        assert_eq!(
            dispatch::unregister(VECTOR, timer_interrupt_handler),
            Ok(())
        );
    }

    #[test]
    fn rejects_non_apic_vectors() {
        assert_eq!(
            register_timer_interrupt(0x20),
            Err(TimerInterruptError::InvalidVector)
        );
        assert_eq!(
            register_timer_interrupt(0x80),
            Err(TimerInterruptError::InvalidVector)
        );
        assert_eq!(
            register_timer_interrupt(0xFF),
            Err(TimerInterruptError::InvalidVector)
        );
    }
}
