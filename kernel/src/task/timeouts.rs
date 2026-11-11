//! Per-CPU task sleep deadlines and timeout-to-wakeup handoff.
//!
//! This layer connects task identifiers to the local APIC-backed deadline
//! queue. It does not perform context switches: expired IDs are returned to
//! the scheduler, which owns task-state transitions and ready-queue insertion.

use crate::time::{
    ApicCountdownInitError, ApicCountdownQueue, BootstrapTimerInitError, ClockSource, Deadline, TimerEvent, TimerId, TimerCancelReport, TimerProcessReport, TscDeadlineQueue, TscDeadlineQueueError,
};

/// Stable task identifier used by timeout registration.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TaskId(pub u64);

const TASK_SLEEP_EVENT: u32 = 0x5441_534B;

/// Result of collecting task wakeups.
///
/// Wakeup IDs are valid even when `timer_reprogram_error` is present. In that
/// case the timer layer has reasserted deferred work so the caller can retry
/// comparator programming without losing this batch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaskWakeupReport {
    pub wakeups: usize,
    pub timer_reprogram_error: Option<crate::drivers::api::DriverError>,
    pub buffer_unavailable: bool,
}

/// Outcome of cancelling a task sleep.
///
/// The `cancelled` field reports the software-queue result even if updating
/// the hardware comparator failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaskCancelReport {
    pub cancelled: bool,
    pub timer_reprogram_error: Option<crate::drivers::api::DriverError>,
}

/// Per-CPU task timeout manager driven by the local APIC TSC-deadline timer.
pub struct TaskTimeouts<S: ClockSource, const N: usize> {
    timers: TscDeadlineQueue<S, N>,
}

impl<S: ClockSource, const N: usize> TaskTimeouts<S, N> {
    /// Initialize the BSP hardware timer and task timeout manager end-to-end.
    ///
    /// The APIC LVT is configured masked, its handler is registered, the
    /// deadline queue is constructed, and only then is the timer unmasked.
    ///
    /// # Safety
    /// BSP only, CPL 0, IF clear, after interrupt bootstrap selected x2APIC.
    /// The source and TSC frequency must be stable for the current CPU.
    pub unsafe fn initialize_bootstrap(
        vector: u8,
        source: S,
        initial_time_ns: u64,
        tsc_frequency_hz: u64,
    ) -> Result<Self, BootstrapTimerInitError> {
        // SAFETY: Forwarded from this method's bootstrap initialization contract.
        let timers = unsafe {
            TscDeadlineQueue::initialize_bootstrap(
                vector, source, initial_time_ns, tsc_frequency_hz,
            )?
        };
        Ok(Self { timers })
    }

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
    pub unsafe fn cancel_sleep(&self, task: TaskId) -> TaskCancelReport {
        // SAFETY: Same-CPU requirement is part of this method's contract.
        let report = unsafe { self.timers.cancel(TimerId(task.0)) };
        task_cancel_report(report)
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
    ) -> TaskWakeupReport {
        let capacity = core::cmp::min(events.len(), wakeups.len());
        // SAFETY: Same-CPU and deferred-context requirements are part of the
        // caller contract for this method.
        let processed = unsafe { self.timers.process_expired(&mut events[..capacity]) };
        task_wakeup_report(&mut events[..processed.expired], wakeups, processed)
    }


    /// Process task wakeups only when the registered timer IRQ signaled work.
    ///
    /// The atomic notification is consumed before draining. An IRQ arriving
    /// during or after the drain leaves the flag set for the next deferred
    /// pass, so notifications are not lost.
    ///
    /// # Safety
    /// Must run on the CPU that owns this timeout manager, in deferred/task
    /// context rather than inside the hard interrupt handler.
    pub unsafe fn collect_pending_wakeups(
        &self,
        events: &mut [Option<TimerEvent>],
        wakeups: &mut [Option<TaskId>],
    ) -> Option<TaskWakeupReport> {
        if !crate::time::take_timer_work_pending() {
            return None;
        }
        // SAFETY: Same-CPU and deferred-context requirements are inherited.
        Some(unsafe { self.collect_wakeups(events, wakeups) })
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

/// Errors while discovering a TSC clocksource and initializing its deadline backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DetectedTaskTimeoutInitError {
    ClockSourceUnavailable,
    Timer(BootstrapTimerInitError),
}

impl<const N: usize> TaskTimeouts<crate::time::TscClockSource, N> {
    /// Discover an invariant TSC frequency (using HPET calibration if needed)
    /// and initialize the BSP TSC-deadline task timeout backend.
    ///
    /// # Safety
    /// BSP only, CPL 0, IF clear, after x2APIC bootstrap. If CPUID leaf 0x15
    /// lacks a complete ratio, an enabled HPET must be supplied for calibration.
    pub unsafe fn initialize_bootstrap_detected(
        vector: u8,
        hpet: Option<&crate::drivers::timer::Hpet>,
        initial_time_ns: u64,
        sample_ms: u32,
        max_spin_iterations: usize,
    ) -> Result<Self, DetectedTaskTimeoutInitError> {
        let source = crate::time::TscClockSource::detect_or_calibrate(
            hpet, sample_ms, max_spin_iterations,
        ).ok_or(DetectedTaskTimeoutInitError::ClockSourceUnavailable)?;
        let frequency_hz = source.frequency_hz();
        // SAFETY: Forwarded from this method's BSP/x2APIC bootstrap contract.
        unsafe {
            Self::initialize_bootstrap(vector, source, initial_time_ns, frequency_hz)
        }.map_err(DetectedTaskTimeoutInitError::Timer)
    }
}

/// x2APIC countdown task timeout backend for CPUs without TSC-deadline
/// support. Its precision depends on HPET calibration and the APIC divider.
pub struct ApicCountdownTaskTimeouts<S: ClockSource, const N: usize> {
    timers: ApicCountdownQueue<S, N>,
}

impl<S: ClockSource, const N: usize> ApicCountdownTaskTimeouts<S, N> {
    /// Initialize the calibrated x2APIC countdown timer and task queue.
    ///
    /// # Safety
    /// BSP only, CPL 0, IF clear, after x2APIC and interrupt bootstrap. HPET
    /// must be enabled and the vector reserved for this timer.
    pub unsafe fn initialize_bootstrap(
        vector: u8,
        divide: crate::drivers::timer::ApicDivide,
        hpet: &crate::drivers::timer::Hpet,
        source: S,
        initial_time_ns: u64,
        sample_ms: u32,
        max_spin_iterations: usize,
    ) -> Result<Self, ApicCountdownInitError> {
        // SAFETY: Forwarded from the APIC countdown bootstrap contract.
        let timers = unsafe {
            ApicCountdownQueue::initialize_bootstrap(
                vector, divide, hpet, source, initial_time_ns, sample_ms, max_spin_iterations,
            )?
        };
        Ok(Self { timers })
    }

    #[inline]
    pub fn now_ns(&self) -> u64 {
        self.timers.now_ns()
    }

    /// Register an absolute task sleep deadline.
    ///
    /// # Safety
    /// Must be called on the CPU that owns this timeout manager.
    pub unsafe fn sleep_until(
        &self,
        task: TaskId,
        deadline: Deadline,
    ) -> Result<(), crate::time::ApicCountdownQueueError> {
        // SAFETY: Same-CPU requirement is part of this method's contract.
        unsafe { self.timers.schedule_at(TimerId(task.0), deadline, TASK_SLEEP_EVENT, 0) }
    }

    /// Register a relative task sleep and return its absolute deadline.
    ///
    /// # Safety
    /// Must be called on the CPU that owns this timeout manager.
    pub unsafe fn sleep_for(
        &self,
        task: TaskId,
        duration_ns: u64,
    ) -> Result<Deadline, crate::time::ApicCountdownQueueError> {
        // SAFETY: Same-CPU requirement is part of this method's contract.
        unsafe { self.timers.schedule_after(TimerId(task.0), duration_ns, TASK_SLEEP_EVENT, 0) }
    }

    /// Cancel a pending task sleep.
    ///
    /// # Safety
    /// Must be called on the CPU that owns this timeout manager.
    pub unsafe fn cancel_sleep(&self, task: TaskId) -> TaskCancelReport {
        // SAFETY: Same-CPU requirement is part of this method's contract.
        task_cancel_report(unsafe { self.timers.cancel(TimerId(task.0)) })
    }

    /// Collect expired task IDs after the local APIC timer IRQ.
    ///
    /// # Safety
    /// Must run on the owning CPU in deferred/task context, not in the hard IRQ.
    pub unsafe fn collect_pending_wakeups(
        &self,
        events: &mut [Option<TimerEvent>],
        wakeups: &mut [Option<TaskId>],
    ) -> Option<TaskWakeupReport> {
        if !crate::time::take_timer_work_pending() {
            return None;
        }
        let capacity = core::cmp::min(events.len(), wakeups.len());
        // SAFETY: Same-CPU and deferred-context requirements are inherited.
        let processed = unsafe { self.timers.process_expired(&mut events[..capacity]) };
        Some(task_wakeup_report(&mut events[..processed.expired], wakeups, processed))
    }

    pub fn next_deadline(&self) -> Option<Deadline> {
        self.timers.next_deadline()
    }

    pub fn pending(&self) -> usize {
        self.timers.pending()
    }

    /// Stop the local timer and unregister its IRQ handler.
    ///
    /// # Safety
    /// BSP only, CPL 0, IF clear, no handler in flight, and no remaining users.
    pub unsafe fn shutdown(&mut self) -> Result<(), crate::time::TimerInterruptError> {
        // SAFETY: Forwarded from the APIC countdown teardown contract.
        unsafe { self.timers.shutdown() }
    }
}

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
    pub fn cancel_sleep(&self, task: TaskId) -> bool {
        self.timers.cancel(TimerId(task.0)).is_some()
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

/// Convert the timer backend's cancellation result into task semantics.
fn task_cancel_report(report: TimerCancelReport) -> TaskCancelReport {
    TaskCancelReport {
        cancelled: report.removed.is_some(),
        timer_reprogram_error: report.reprogram_error,
    }
}

/// Convert a drained timer batch into task wakeups without discarding
/// the hardware reprogramming status.
fn task_wakeup_report(
    events: &mut [Option<TimerEvent>],
    wakeups: &mut [Option<TaskId>],
    processed: TimerProcessReport,
) -> TaskWakeupReport {
    let count = extract_task_wakeups(events, wakeups);
    TaskWakeupReport {
        wakeups: count,
        timer_reprogram_error: processed.reprogram_error,
        buffer_unavailable: processed.buffer_unavailable,
    }
}

/// Compact task-sleep events into the caller's wakeup buffer.
///
/// Non-task timer events are consumed but never exposed as task IDs. The
/// returned count is the number of actual task wakeups, not all events drained.
fn extract_task_wakeups(
    events: &mut [Option<TimerEvent>],
    wakeups: &mut [Option<TaskId>],
) -> usize {
    wakeups.fill(None);
    let mut written = 0;
    for slot in events.iter_mut() {
        let Some(event) = slot.take() else { continue };
        if event.kind == TASK_SLEEP_EVENT && written < wakeups.len() {
            wakeups[written] = Some(TaskId(event.id.0));
            written += 1;
        }
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: u64, kind: u32) -> Option<TimerEvent> {
        Some(TimerEvent { id: TimerId(id), deadline: Deadline::at_ns(10), kind, context: 0 })
    }

    #[test]
    fn wakeups_are_compacted_and_non_task_events_are_filtered() {
        let mut events = [event(1, TASK_SLEEP_EVENT), event(2, 7), event(3, TASK_SLEEP_EVENT)];
        let mut wakeups = [Some(TaskId(99)); 3];
        assert_eq!(extract_task_wakeups(&mut events, &mut wakeups), 2);
        assert_eq!(wakeups, [Some(TaskId(1)), Some(TaskId(3)), None]);
        assert!(events.iter().all(Option::is_none));
    }

    #[test]
    fn cancellation_result_survives_comparator_failure() {
        let report = task_cancel_report(TimerCancelReport {
            removed: event(9, TASK_SLEEP_EVENT),
            reprogram_error: Some(crate::drivers::api::DriverError::HardwareFault),
        });
        assert!(report.cancelled);
        assert_eq!(
            report.timer_reprogram_error,
            Some(crate::drivers::api::DriverError::HardwareFault)
        );

        let missing = task_cancel_report(TimerCancelReport {
            removed: None,
            reprogram_error: None,
        });
        assert!(!missing.cancelled);
        assert_eq!(missing.timer_reprogram_error, None);
    }

    #[test]
    fn wakeups_are_delivered_even_when_timer_reprogramming_fails() {
        let mut events = [event(42, TASK_SLEEP_EVENT)];
        let mut wakeups = [None; 1];
        let report = task_wakeup_report(
            &mut events,
            &mut wakeups,
            TimerProcessReport {
                expired: 1,
                reprogram_error: Some(crate::drivers::api::DriverError::HardwareFault),
                buffer_unavailable: false,
            },
        );

        assert_eq!(report.wakeups, 1);
        assert_eq!(wakeups, [Some(TaskId(42))]);
        assert_eq!(
            report.timer_reprogram_error,
            Some(crate::drivers::api::DriverError::HardwareFault)
        );
    }

    #[test]
    fn empty_storage_is_reported_without_consuming_events() {
        let mut wakeups = [Some(TaskId(7))];
        let report = task_wakeup_report(
            &mut [],
            &mut wakeups,
            TimerProcessReport {
                expired: 0,
                reprogram_error: None,
                buffer_unavailable: true,
            },
        );

        assert_eq!(report.wakeups, 0);
        assert!(report.buffer_unavailable);
        assert_eq!(wakeups, [None]);
    }

    #[test]
    fn wakeup_output_is_cleared_when_no_task_events_exist() {
        let mut events = [event(2, 7)];
        let mut wakeups = [Some(TaskId(99)); 2];
        assert_eq!(extract_task_wakeups(&mut events, &mut wakeups), 0);
        assert_eq!(wakeups, [None, None]);
    }
}
