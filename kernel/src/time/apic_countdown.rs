//! x2APIC one-shot countdown queue backend.
//!
//! This backend is the fallback for x2APIC systems that lack the TSC-deadline
//! MSR feature. It uses an HPET-calibrated local-APIC countdown and the same
//! bounded timer service as the TSC-deadline implementation.

use core::marker::PhantomData;

use crate::{
    drivers::{
        api::DriverError,
        timer::{ApicDivide, Hpet, X2ApicTimer},
    },
};

use super::{
    apic_deadline::{BootstrapTimerInitError, TimerCancelReport, TimerProcessReport},
    clock::ClockSource,
    deadline::Deadline,
    interrupt::TimerInterruptError,
    service::{TimerService, TimerServiceError},
    timer_queue::{TimerEvent, TimerId},
};

/// Errors from the x2APIC countdown queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApicCountdownQueueError {
    Service(TimerServiceError),
    Hardware(DriverError),
}

/// Errors while initializing the BSP x2APIC countdown queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApicCountdownInitError {
    Interrupt(TimerInterruptError),
    Queue(ApicCountdownQueueError),
}

/// Per-CPU timer queue driven by the x2APIC initial-count timer.
pub struct ApicCountdownQueue<S: ClockSource, const N: usize> {
    service: TimerService<S, N>,
    timer: X2ApicTimer,
    _cpu_local: PhantomData<*mut ()>,
}

impl<S: ClockSource, const N: usize> ApicCountdownQueue<S, N> {
    /// Initialize the BSP countdown backend, calibrating it against HPET.
    ///
    /// The LVT is configured masked and the handler registered before
    /// calibration. The queue is created before the timer source is unmasked.
    ///
    /// # Safety
    /// BSP only, CPL 0, IF clear, after interrupt bootstrap selected x2APIC.
    /// The HPET must be enabled and stable; the source must be a valid
    /// monotonic clocksource. The vector must be reserved for this timer.
    pub unsafe fn initialize_bootstrap(
        vector: u8,
        divide: ApicDivide,
        hpet: &Hpet,
        source: S,
        initial_time_ns: u64,
        sample_ms: u32,
        max_spin_iterations: usize,
    ) -> Result<Self, ApicCountdownInitError> {
        // SAFETY: Forwarded from this method's BSP/CPL0/IF-clear contract.
        unsafe { super::interrupt::register_countdown_timer_interrupt(vector, divide) }
            .map_err(ApicCountdownInitError::Interrupt)?;

        // SAFETY: x2APIC was initialized and the countdown LVT is masked.
        let timer = match unsafe {
            X2ApicTimer::calibrate_with_hpet(
                vector, divide, hpet, sample_ms, max_spin_iterations,
            )
        } {
            Ok(timer) => timer,
            Err(error) => {
                // SAFETY: Same serialized bootstrap context.
                let _ = unsafe { super::interrupt::unregister_timer_interrupt() };
                return Err(ApicCountdownInitError::Queue(
                    ApicCountdownQueueError::Hardware(error),
                ));
            }
        };

        let service = match TimerService::new(source, initial_time_ns) {
            Ok(service) => service,
            Err(error) => {
                // SAFETY: Same serialized bootstrap context.
                let _ = unsafe { super::interrupt::unregister_timer_interrupt() };
                return Err(ApicCountdownInitError::Queue(
                    ApicCountdownQueueError::Service(error),
                ));
            }
        };

        let queue = Self { service, timer, _cpu_local: PhantomData };
        // SAFETY: Handler, calibrated timer and software queue are all ready.
        if let Err(error) = unsafe { super::interrupt::unmask_timer_interrupt() } {
            // SAFETY: IF is clear and no timer IRQ can be in flight.
            let _ = unsafe { super::interrupt::unregister_timer_interrupt() };
            return Err(ApicCountdownInitError::Interrupt(error));
        }
        Ok(queue)
    }

    #[inline]
    pub fn now_ns(&self) -> u64 {
        self.service.now_ns()
    }

    /// Schedule an absolute event and update the local countdown.
    ///
    /// # Safety
    /// Must be called on the CPU that owns this queue.
    pub unsafe fn schedule_at(
        &self,
        id: TimerId,
        deadline: Deadline,
        kind: u32,
        context: usize,
    ) -> Result<(), ApicCountdownQueueError> {
        self.service.schedule_at(id, deadline, kind, context)
            .map_err(ApicCountdownQueueError::Service)?;
        // SAFETY: Same-CPU requirement is part of this method's contract.
        if let Err(error) = unsafe { self.reprogram() } {
            let _ = self.service.cancel(id);
            return Err(ApicCountdownQueueError::Hardware(error));
        }
        Ok(())
    }

    /// Schedule a relative event.
    ///
    /// # Safety
    /// Must be called on the CPU that owns this queue.
    pub unsafe fn schedule_after(
        &self,
        id: TimerId,
        delay_ns: u64,
        kind: u32,
        context: usize,
    ) -> Result<Deadline, ApicCountdownQueueError> {
        let deadline = Deadline::after_ns(self.service.now_ns(), delay_ns);
        // SAFETY: Same-CPU requirement is part of this method's contract.
        unsafe { self.schedule_at(id, deadline, kind, context)? };
        Ok(deadline)
    }

    /// Cancel an event and report queue removal separately from hardware status.
    ///
    /// # Safety
    /// Must be called on the CPU that owns this queue.
    pub unsafe fn cancel(&self, id: TimerId) -> TimerCancelReport {
        let removed = self.service.cancel(id);
        // SAFETY: Same-CPU requirement is part of this method's contract.
        let reprogram_error = unsafe { self.reprogram() }.err();
        if reprogram_error.is_some() {
            super::interrupt::signal_timer_work_pending();
        }
        TimerCancelReport { removed, reprogram_error }
    }

    /// Drain expired events and update the next countdown.
    ///
    /// # Safety
    /// Must run on the owning CPU in deferred/task context, not in the hard IRQ.
    pub unsafe fn process_expired(
        &self,
        output: &mut [Option<TimerEvent>],
    ) -> TimerProcessReport {
        if output.is_empty() {
            super::interrupt::signal_timer_work_pending();
            return TimerProcessReport {
                expired: 0,
                reprogram_error: None,
                buffer_unavailable: true,
            };
        }
        let expired = self.service.drain_expired(output);
        // SAFETY: Same-CPU requirement is part of this method's contract.
        let reprogram_error = unsafe { self.reprogram() }.err();
        if reprogram_error.is_some() {
            super::interrupt::signal_timer_work_pending();
        }
        TimerProcessReport { expired, reprogram_error, buffer_unavailable: false }
    }

    /// Program the earliest deadline, splitting intervals beyond the APIC's
    /// 32-bit countdown range into successive one-shot interrupts.
    ///
    /// # Safety
    /// Must execute at CPL 0 on the CPU that owns this queue.
    pub unsafe fn reprogram(&self) -> Result<(), DriverError> {
        let Some(deadline) = self.service.next_deadline() else {
            // SAFETY: Timer is CPU-local and remains configured/unmasked.
            return unsafe { self.timer.disarm() };
        };
        if deadline.timestamp_ns() == u64::MAX {
            // SAFETY: Infinite deadlines do not require a hardware countdown.
            return unsafe { self.timer.disarm() };
        }

        let remaining_ns = deadline.timestamp_ns().saturating_sub(self.service.now_ns());
        // SAFETY: Timer is CPU-local and configured in one-shot mode.
        unsafe { self.timer.arm_after_ns(remaining_ns)?; }
        Ok(())
    }

    pub fn next_deadline(&self) -> Option<Deadline> {
        self.service.next_deadline()
    }

    pub fn pending(&self) -> usize {
        self.service.pending()
    }

    pub const fn frequency_hz(&self) -> u64 {
        self.timer.frequency_hz()
    }

    /// Stop the countdown, mask the local source, and unregister the handler.
    ///
    /// # Safety
    /// BSP only, CPL 0, IF clear, with no handler in flight and no remaining
    /// users of this queue.
    pub unsafe fn shutdown(&mut self) -> Result<(), TimerInterruptError> {
        // SAFETY: Caller guarantees the owning CPU and serialized teardown.
        unsafe { self.timer.stop() };
        // SAFETY: Same BSP teardown contract.
        unsafe { super::interrupt::unregister_timer_interrupt() }
    }
}
