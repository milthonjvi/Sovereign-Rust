//! Legacy PIT-backed task timeout fallback.

use crate::time::{Deadline, TimerEvent, TimerId};

use super::{
    extract_task_wakeups, TaskCancelReport, TaskId, TaskWakeupReport, TASK_SLEEP_EVENT,
};

/// Legacy-PIT task timeout fallback for systems where the legacy PIC IRQ0
/// route remains active. Wakeup precision is bounded by the PIT tick rate.
pub struct PitTaskTimeouts<const N: usize> {
    timers: crate::time::PitTimerService<N>,
}

impl<const N: usize> PitTaskTimeouts<N> {
    /// Initialize the PIT IRQ0 fallback and task timeout queue.
    ///
    /// # Safety
    /// BSP only, CPL 0, IF clear, after legacy PIC initialization and with
    /// exclusive ownership of PIT channel 0.
    pub unsafe fn initialize_bootstrap(
        requested_hz: u32,
        initial_time_ns: u64,
    ) -> Result<Self, crate::time::PitTimerServiceError> {
        // SAFETY: Forwarded from the PIT/PIC bootstrap contract.
        let timers = unsafe {
            crate::time::PitTimerService::initialize_bootstrap(requested_hz, initial_time_ns)?
        };
        Ok(Self { timers })
    }

    #[inline]
    pub fn now_ns(&self) -> u64 {
        self.timers.now_ns()
    }

    /// Register an absolute task sleep deadline.
    pub fn sleep_until(
        &self,
        task: TaskId,
        deadline: Deadline,
    ) -> Result<(), crate::time::PitTimerServiceError> {
        self.timers.schedule_at(TimerId(task.0), deadline, TASK_SLEEP_EVENT, 0)
    }

    /// Register a relative task sleep and return its absolute deadline.
    pub fn sleep_for(
        &self,
        task: TaskId,
        duration_ns: u64,
    ) -> Result<Deadline, crate::time::PitTimerServiceError> {
        self.timers.schedule_after(TimerId(task.0), duration_ns, TASK_SLEEP_EVENT, 0)
    }

    /// Cancel a task sleep. The next PIT tick continues to drive other events.
    pub fn cancel_sleep(&self, task: TaskId) -> TaskCancelReport {
        TaskCancelReport {
            cancelled: self.timers.cancel(TimerId(task.0)).is_some(),
            timer_reprogram_error: None,
        }
    }

    /// Collect expired task IDs after a PIT tick requested deferred work.
    pub fn collect_pending_wakeups(
        &self,
        events: &mut [Option<TimerEvent>],
        wakeups: &mut [Option<TaskId>],
    ) -> Option<TaskWakeupReport> {
        let capacity = core::cmp::min(events.len(), wakeups.len());
        let drained = self.timers.process_pending(&mut events[..capacity])?;
        let count = extract_task_wakeups(&mut events[..drained], wakeups);
        Some(TaskWakeupReport {
            wakeups: count,
            timer_reprogram_error: None,
            buffer_unavailable: capacity == 0,
        })
    }

    pub fn next_deadline(&self) -> Option<Deadline> {
        self.timers.next_deadline()
    }

    pub fn pending(&self) -> usize {
        self.timers.pending()
    }

    /// Stop IRQ0 delivery and release the legacy PIT handler.
    ///
    /// # Safety
    /// BSP only, CPL 0, IF clear, with no PIT handler in flight.
    pub unsafe fn shutdown(
        &mut self,
    ) -> Result<(), crate::time::PitTimerServiceError> {
        // SAFETY: Forwarded from the PIT service teardown contract.
        unsafe { self.timers.shutdown() }
    }
}

