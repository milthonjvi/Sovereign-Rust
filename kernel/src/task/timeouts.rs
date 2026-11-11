//! Task sleep deadlines and timeout-to-wakeup handoff.
//!
//! This module does not perform a context switch. It converts expired timer
//! events into task IDs for the scheduler's ready-queue path. The scheduler
//! owns task state transitions and must enqueue returned IDs after this method
//! has completed.

use crate::time::{
    ClockSource, Deadline, TimerEvent, TimerId, TimerService, TimerServiceError,
};

/// Stable task identifier used by timeout registration.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TaskId(pub u64);

/// Task sleep/deadline manager backed by the kernel monotonic timer service.
pub struct TaskTimeouts<S: ClockSource, const N: usize> {
    timers: TimerService<S, N>,
}

impl<S: ClockSource, const N: usize> TaskTimeouts<S, N> {
    /// Construct the task timeout manager from the selected clocksource.
    pub fn new(source: S, initial_time_ns: u64) -> Result<Self, TimerServiceError> {
        Ok(Self { timers: TimerService::new(source, initial_time_ns)? })
    }

    /// Current monotonic time in nanoseconds.
    #[inline]
    pub fn now_ns(&self) -> u64 {
        self.timers.now_ns()
    }

    /// Register a task sleep until an absolute monotonic deadline.
    ///
    /// A task may have only one pending sleep. Register the timeout before
    /// changing the task to a blocked state, with the scheduler's own locking
    /// protocol preventing a wakeup from racing the state transition.
    pub fn sleep_until(&self, task: TaskId, deadline: Deadline) -> Result<(), TimerServiceError> {
        self.timers.schedule_at(TimerId(task.0), deadline, 0x5441_534B, 0)
    }

    /// Register a relative sleep. Returns the absolute wake deadline.
    pub fn sleep_for(&self, task: TaskId, duration_ns: u64) -> Result<Deadline, TimerServiceError> {
        self.timers.schedule_after(TimerId(task.0), duration_ns, 0x5441_534B, 0)
    }

    /// Cancel a pending sleep, for example when another event wakes the task.
    pub fn cancel_sleep(&self, task: TaskId) -> bool {
        self.timers.cancel(TimerId(task.0)).is_some()
    }

    /// Next deadline the hardware timer backend should program.
    pub fn next_deadline(&self) -> Option<Deadline> {
        self.timers.next_deadline()
    }

    /// Collect expired task IDs for the scheduler's ready queue.
    ///
    /// The caller supplies both scratch event storage and output task storage.
    /// At most min(events.len(), wakeups.len()) expirations are consumed. The
    /// method does not mutate scheduler state or perform context switches.
    pub fn collect_wakeups(
        &self,
        events: &mut [Option<TimerEvent>],
        wakeups: &mut [Option<TaskId>],
    ) -> usize {
        let capacity = core::cmp::min(events.len(), wakeups.len());
        let count = self.timers.drain_expired(&mut events[..capacity]);
        for index in 0..count {
            if let Some(event) = events[index].take() {
                wakeups[index] = Some(TaskId(event.id.0));
            }
        }
        count
    }

    /// Number of pending task sleeps.
    pub fn pending(&self) -> usize {
        self.timers.pending()
    }
}
