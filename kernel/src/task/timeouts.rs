//! Per-CPU task sleep deadlines and timeout-to-wakeup handoff.
//!
//! This layer connects task identifiers to the local APIC-backed deadline
//! queue. It does not perform context switches: expired IDs are returned to
//! the scheduler, which owns task-state transitions and ready-queue insertion.

use crate::time::{
    ClockSource, Deadline, TimerEvent, TimerId, TscDeadlineQueue, TscDeadlineQueueError,
};

/// Stable task identifier used by timeout registration.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TaskId(pub u64);

const TASK_SLEEP_EVENT: u32 = 0x5441_534B;

/// Per-CPU task timeout manager driven by the local APIC TSC-deadline timer.
pub struct TaskTimeouts<S: ClockSource, const N: usize> {
    timers: TscDeadlineQueue<S, N>,
}

impl<S: ClockSource, const N: usize> TaskTimeouts<S, N> {
    /// Construct task timeout management on the current CPU.
    ///
    /// # Safety
    /// Must run on the target CPU after configuring its local APIC timer LVT
    /// in TSC-deadline mode and registering its timer interrupt vector. The
    /// TSC frequency must be accurate for this CPU's TSC domain.
    pub unsafe fn new(
        source: S,
        initial_time_ns: u64,
        tsc_frequency_hz: u64,
    ) -> Result<Self, TscDeadlineQueueError> {
        // SAFETY: The caller guarantees the local APIC/TSC setup contract.
        let timers = unsafe {
            TscDeadlineQueue::new(source, initial_time_ns, tsc_frequency_hz)?
        };
        Ok(Self { timers })
    }

    /// Current monotonic time in nanoseconds.
    #[inline]
    pub fn now_ns(&self) -> u64 {
        self.timers.now_ns()
    }

    /// Register a task sleep until an absolute monotonic deadline.
    ///
    /// Register the timeout before changing the task to blocked state. The
    /// scheduler must serialize that transition against timeout processing.
    ///
    /// # Safety
    /// Must be called on the CPU that owns this timeout manager.
    pub unsafe fn sleep_until(
        &self,
        task: TaskId,
        deadline: Deadline,
    ) -> Result<(), TscDeadlineQueueError> {
        // SAFETY: Same-CPU requirement is part of this method's contract.
        unsafe {
            self.timers.schedule_at(
                TimerId(task.0),
                deadline,
                TASK_SLEEP_EVENT,
                0,
            )
        }
    }

    /// Register a relative task sleep and return its absolute deadline.
    ///
    /// # Safety
    /// Must be called on the CPU that owns this timeout manager.
    pub unsafe fn sleep_for(
        &self,
        task: TaskId,
        duration_ns: u64,
    ) -> Result<Deadline, TscDeadlineQueueError> {
        // SAFETY: Same-CPU requirement is part of this method's contract.
        unsafe { self.timers.schedule_after(TimerId(task.0), duration_ns, TASK_SLEEP_EVENT, 0) }
    }

    /// Cancel a pending sleep, e.g. when another event wakes the task.
    ///
    /// # Safety
    /// Must be called on the CPU that owns this timeout manager.
    pub unsafe fn cancel_sleep(
        &self,
        task: TaskId,
    ) -> Result<bool, crate::drivers::api::DriverError> {
        // SAFETY: Same-CPU requirement is part of this method's contract.
        Ok(unsafe { self.timers.cancel(TimerId(task.0)) }?.is_some())
    }

    /// Collect expired task IDs for insertion into the scheduler ready queue.
    ///
    /// The caller supplies scratch event storage and output task storage.
    /// At most min(events.len(), wakeups.len()) events are consumed. No task
    /// state is modified and no context switch is performed here.
    ///
    /// # Safety
    /// Must run on the CPU that owns this timeout manager, in deferred/task
    /// context rather than inside the hard interrupt handler.
    pub unsafe fn collect_wakeups(
        &self,
        events: &mut [Option<TimerEvent>],
        wakeups: &mut [Option<TaskId>],
    ) -> Result<usize, crate::drivers::api::DriverError> {
        let capacity = core::cmp::min(events.len(), wakeups.len());
        // SAFETY: Same-CPU and deferred-context requirements are part of the
        // caller contract for this method.
        let count = unsafe { self.timers.process_expired(&mut events[..capacity])? };
        for index in 0..count {
            if let Some(event) = events[index].take() {
                if event.kind == TASK_SLEEP_EVENT {
                    wakeups[index] = Some(TaskId(event.id.0));
                }
            }
        }
        Ok(count)
    }

    /// Next monotonic deadline for the current CPU's hardware timer.
    pub fn next_deadline(&self) -> Option<Deadline> {
        self.timers.next_deadline()
    }

    /// Number of pending task sleeps.
    pub fn pending(&self) -> usize {
        self.timers.pending()
    }
}
