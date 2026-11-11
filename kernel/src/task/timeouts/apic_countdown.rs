//! x2APIC countdown and task timeout backend.

use crate::time::{
    ApicCountdownInitError, ApicCountdownQueue, ApicCountdownQueueError,
    ClockSource, Deadline, TimerEvent, TimerId,
};

use super::{
    task_cancel_report, task_wakeup_report, TaskCancelReport, TaskId, TaskWakeupReport,
    TASK_SLEEP_EVENT,
};

/// x2APIC countdown task timeout backend for CPUs without TSC-deadline
/// support. Its precision depends on HPET/TSC calibration and the APIC divider.
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

    /// Initialize the x2APIC countdown backend using an invariant TSC as
    /// the calibration reference instead of HPET.
    ///
    /// # Safety
    /// BSP only, CPL 0, IF clear, after x2APIC bootstrap. The supplied TSC
    /// frequency must be accurate and invariant for the current CPU.
    pub unsafe fn initialize_bootstrap_tsc(
        vector: u8,
        divide: crate::drivers::timer::ApicDivide,
        source: S,
        initial_time_ns: u64,
        tsc_frequency_hz: u64,
        sample_ms: u32,
        max_spin_iterations: usize,
    ) -> Result<Self, ApicCountdownInitError> {
        // SAFETY: Forwarded from the APIC/TSC bootstrap contract.
        let timers = unsafe {
            ApicCountdownQueue::initialize_bootstrap_tsc(
                vector, divide, source, initial_time_ns, tsc_frequency_hz,
                sample_ms, max_spin_iterations,
            )?
        };
        Ok(Self { timers })
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

impl<const N: usize>
    ApicCountdownTaskTimeouts<crate::time::TscClockSource, N>
{
    /// Discover invariant-TSC frequency and choose the available calibration
    /// reference: HPET when supplied, otherwise the TSC itself.
    ///
    /// # Safety
    /// BSP only, CPL 0, IF clear, after x2APIC bootstrap. If CPUID does not
    /// provide a complete TSC ratio, HPET must be supplied.
    pub unsafe fn initialize_bootstrap_detected(
        vector: u8,
        divide: crate::drivers::timer::ApicDivide,
        hpet: Option<&crate::drivers::timer::Hpet>,
        initial_time_ns: u64,
        sample_ms: u32,
        max_spin_iterations: usize,
    ) -> Result<Self, ApicCountdownInitError> {
        let source = crate::time::TscClockSource::detect_or_calibrate(
            hpet, sample_ms, max_spin_iterations,
        ).ok_or(ApicCountdownInitError::Queue(
            crate::time::ApicCountdownQueueError::Service(
                crate::time::TimerServiceError::InvalidClockSource,
            ),
        ))?;
        let frequency_hz = source.frequency_hz();
        if let Some(hpet) = hpet {
            // SAFETY: Forwarded from the x2APIC bootstrap contract.
            unsafe {
                Self::initialize_bootstrap(
                    vector, divide, hpet, source, initial_time_ns,
                    sample_ms, max_spin_iterations,
                )
            }
        } else {
            // SAFETY: The detected source guarantees invariant TSC frequency.
            unsafe {
                Self::initialize_bootstrap_tsc(
                    vector, divide, source, initial_time_ns, frequency_hz,
                    sample_ms, max_spin_iterations,
                )
            }
        }
    }
}

impl<'a, const N: usize>
    ApicCountdownTaskTimeouts<crate::time::HpetClockSource<'a>, N>
{
    /// Initialize the countdown backend using HPET for monotonic timekeeping
    /// and for local-APIC timer calibration.
    ///
    /// # Safety
    /// BSP only, CPL 0, IF clear, after x2APIC bootstrap. The HPET must be
    /// mapped, enabled, stable, and exclusively protected from reconfiguration.
    pub unsafe fn initialize_bootstrap_hpet(
        vector: u8,
        divide: crate::drivers::timer::ApicDivide,
        hpet: &'a crate::drivers::timer::Hpet,
        initial_time_ns: u64,
        sample_ms: u32,
        max_spin_iterations: usize,
    ) -> Result<Self, ApicCountdownInitError> {
        let source = crate::time::HpetClockSource::new(hpet).ok_or(
            ApicCountdownInitError::Queue(crate::time::ApicCountdownQueueError::Service(
                crate::time::TimerServiceError::InvalidClockSource,
            )),
        )?;
        // SAFETY: Forwarded from this method's BSP/x2APIC/HPET contract.
        unsafe {
            Self::initialize_bootstrap(
                vector, divide, hpet, source, initial_time_ns, sample_ms, max_spin_iterations,
            )
        }
    }
}

