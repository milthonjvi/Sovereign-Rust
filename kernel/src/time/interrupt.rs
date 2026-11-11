//! Timer interrupt registration and deferred-work signaling.
//!
//! The hard IRQ path performs one atomic store only. Expiration processing,
//! queue locking, and task wakeup handoff must run later in task/softirq context.
//! The common APIC dispatcher remains responsible for EOI.

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
