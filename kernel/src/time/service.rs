//! Shared bounded timer service.
//!
//! The service combines a monotonic clock with a fixed-capacity event queue.
//! It deliberately does not execute callbacks: the interrupt handler should
//! only acknowledge the hardware source and request deferred processing.
//! Queue mutation and expiration draining are performed by task/softirq context.

use crate::sync::irq_lock::IrqSpinLock;

use super::{
    clock::{ClockSource, MonotonicClock},
    deadline::Deadline,
    timer_queue::{TimerEvent, TimerId, TimerQueue, TimerQueueError},
};

/// Errors surfaced by the generic timer service.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimerServiceError {
    /// The supplied clocksource has an invalid frequency or counter mask.
    InvalidClockSource,
    /// The fixed-capacity queue rejected an insertion.
    Queue(TimerQueueError),
}

/// Monotonic clock and bounded event queue for kernel timeout consumers.
///
/// The queue is protected against concurrent task-side users and local IRQ
/// re-entry. Callers must still avoid holding the queue lock while invoking
/// device code, waking tasks, or running arbitrary callbacks.
pub struct TimerService<S: ClockSource, const N: usize> {
    clock: MonotonicClock<S>,
    queue: IrqSpinLock<TimerQueue<N>>,
}

impl<S: ClockSource, const N: usize> TimerService<S, N> {
    /// Build a timer service from an initialized, stable hardware clocksource.
    pub fn new(source: S, initial_time_ns: u64) -> Result<Self, TimerServiceError> {
        let clock = MonotonicClock::new(source, initial_time_ns)
            .ok_or(TimerServiceError::InvalidClockSource)?;
        Ok(Self { clock, queue: IrqSpinLock::new(TimerQueue::new()) })
    }

    /// Read the shared monotonic nanosecond timeline.
    #[inline]
    pub fn now_ns(&self) -> u64 {
        self.clock.now_ns()
    }

    /// Schedule an event at an absolute monotonic deadline.
    pub fn schedule_at(
        &self,
        id: TimerId,
        deadline: Deadline,
        kind: u32,
        context: usize,
    ) -> Result<(), TimerServiceError> {
        self.queue.lock().insert(TimerEvent { id, deadline, kind, context })
            .map_err(TimerServiceError::Queue)
    }

    /// Schedule an event after a relative duration, saturating to INFINITE.
    pub fn schedule_after(
        &self,
        id: TimerId,
        delay_ns: u64,
        kind: u32,
        context: usize,
    ) -> Result<Deadline, TimerServiceError> {
        let deadline = Deadline::after_ns(self.now_ns(), delay_ns);
        self.schedule_at(id, deadline, kind, context)?;
        Ok(deadline)
    }

    /// Cancel a pending event by its unique ID.
    pub fn cancel(&self, id: TimerId) -> Option<TimerEvent> {
        self.queue.lock().cancel(id)
    }

    /// Return the next absolute deadline, if any.
    pub fn next_deadline(&self) -> Option<Deadline> {
        self.queue.lock().next_deadline()
    }

    /// Drain expired events into caller-owned storage.
    ///
    /// This method is intended for deferred/task context, not a hard IRQ:
    /// it takes the queue lock and may perform O(K log N) heap work. The
    /// hardware ISR should remain bounded and merely signal the deferred
    /// timer-processing path.
    pub fn drain_expired(&self, output: &mut [Option<TimerEvent>]) -> usize {
        let now = self.now_ns();
        self.queue.lock().drain_expired(now, output)
    }

    /// Number of events currently pending.
    pub fn pending(&self) -> usize {
        self.queue.lock().len()
    }

    /// Expose the calibrated source frequency for hardware comparator setup.
    pub const fn frequency_hz(&self) -> u64 {
        self.clock.frequency_hz()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicU64, Ordering};

    struct MockClock {
        ticks: AtomicU64,
        frequency_hz: u64,
    }

    impl ClockSource for MockClock {
        fn read_ticks(&self) -> u64 { self.ticks.load(Ordering::Relaxed) }
        fn frequency_hz(&self) -> u64 { self.frequency_hz }
        fn mask(&self) -> u64 { u64::MAX }
    }

    #[test]
    fn schedules_cancels_and_drains_expired_events() {
        let source = MockClock { ticks: AtomicU64::new(0), frequency_hz: 1_000_000_000 };
        let service = TimerService::<_, 4>::new(source, 0).unwrap();

        service.schedule_after(TimerId(1), 10, 7, 0xA).unwrap();
        service.schedule_after(TimerId(2), 20, 8, 0xB).unwrap();
        assert_eq!(service.pending(), 2);
        assert_eq!(service.cancel(TimerId(2)).unwrap().context, 0xB);

        service.clock.source().ticks.store(10, Ordering::Relaxed);
        let mut events = [None; 2];
        assert_eq!(service.drain_expired(&mut events), 1);
        assert_eq!(events[0].unwrap().id, TimerId(1));
        assert_eq!(service.pending(), 0);
    }

    #[test]
    fn rejects_invalid_clocksource() {
        let source = MockClock { ticks: AtomicU64::new(0), frequency_hz: 0 };
        assert_eq!(
            TimerService::<_, 2>::new(source, 0).err(),
            Some(TimerServiceError::InvalidClockSource)
        );
    }
}
