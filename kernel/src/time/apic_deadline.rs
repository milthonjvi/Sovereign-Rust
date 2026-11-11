//! Per-CPU x86 TSC-deadline timer queue integration.
//!
//! This adapter connects the generic monotonic event queue to the local APIC
//! TSC-deadline MSR. It is deliberately CPU-local: construct and use one
//! instance on each CPU after its local APIC LVT and timer interrupt vector
//! have been configured. SMP-wide queue changes require an IPI/reprogram policy
//! and are not silently handled by this per-CPU adapter.

use crate::drivers::timer::{TscDeadline, IA32_TSC_DEADLINE};
use crate::drivers::api::DriverError;

use super::{
    clock::ClockSource,
    deadline::Deadline,
    service::{TimerService, TimerServiceError},
    timer_queue::{TimerEvent, TimerId},
    tsc_clock::{monotonic_deadline_to_tsc, TscClockSource},
};

/// Errors from the per-CPU deadline queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TscDeadlineQueueError {
    Service(TimerServiceError),
    Hardware(DriverError),
}

/// Per-CPU timer queue driven by the local APIC TSC-deadline facility.
pub struct TscDeadlineQueue<S: ClockSource, const N: usize> {
    service: TimerService<S, N>,
    timer: TscDeadline,
    tsc_frequency_hz: u64,
}

impl<S: ClockSource, const N: usize> TscDeadlineQueue<S, N> {
    /// Create the service and configure the local APIC deadline MSR controller.
    ///
    /// # Safety
    /// Must run on the target CPU at CPL 0, after that CPU's local APIC LVT
    /// timer is configured in TSC-deadline mode and its interrupt vector is
    /// registered. The TSC frequency must be accurate and the same TSC domain
    /// must be used for every operation on this instance.
    pub unsafe fn new(
        source: S,
        initial_time_ns: u64,
        tsc_frequency_hz: u64,
    ) -> Result<Self, TscDeadlineQueueError> {
        if tsc_frequency_hz == 0 {
            return Err(TscDeadlineQueueError::Service(TimerServiceError::InvalidClockSource));
        }
        // SAFETY: The constructor's caller contract guarantees CPL 0 and
        // TSC-deadline feature support on this local CPU.
        let timer = unsafe { TscDeadline::new() }
            .map_err(TscDeadlineQueueError::Hardware)?;
        let service = TimerService::new(source, initial_time_ns).map_err(TscDeadlineQueueError::Service)?;
        // SAFETY: Constructor is bound to this CPU and the timer handle was
        // created after validating TSC-deadline support.
        unsafe { timer.stop() };
        Ok(Self { service, timer, tsc_frequency_hz })
    }

    /// Current monotonic time in nanoseconds.
    #[inline]
    pub fn now_ns(&self) -> u64 {
        self.service.now_ns()
    }

    /// Schedule an absolute event and immediately reprogram this CPU's timer.
    ///
    /// # Safety
    /// Must be called on the CPU that owns this timer instance.
    pub unsafe fn schedule_at(
        &self,
        id: TimerId,
        deadline: Deadline,
        kind: u32,
        context: usize,
    ) -> Result<(), TscDeadlineQueueError> {
        self.service.schedule_at(id, deadline, kind, context)
            .map_err(TscDeadlineQueueError::Service)?;
        // SAFETY: Same-CPU requirement is part of this method's contract.
        if let Err(error) = unsafe { self.reprogram() } {
            let _ = self.service.cancel(id);
            return Err(TscDeadlineQueueError::Hardware(error));
        }
        Ok(())
    }

    /// Schedule a relative event and reprogram the local comparator.
    ///
    /// # Safety
    /// Must be called on the CPU that owns this timer instance.
    pub unsafe fn schedule_after(
        &self,
        id: TimerId,
        delay_ns: u64,
        kind: u32,
        context: usize,
    ) -> Result<Deadline, TscDeadlineQueueError> {
        let deadline = Deadline::after_ns(self.service.now_ns(), delay_ns);
        // SAFETY: Same-CPU requirement is part of this method's contract.
        unsafe { self.schedule_at(id, deadline, kind, context)? };
        Ok(deadline)
    }

    /// Cancel an event and reprogram the next earliest deadline.
    ///
    /// # Safety
    /// Must be called on the CPU that owns this timer instance.
    pub unsafe fn cancel(&self, id: TimerId) -> Result<Option<TimerEvent>, DriverError> {
        let removed = self.service.cancel(id);
        // SAFETY: Same-CPU requirement is part of this method's contract.
        unsafe { self.reprogram()? };
        Ok(removed)
    }

    /// Drain expired events and program the next queued deadline.
    ///
    /// Call from deferred/task context after consuming the timer IRQ signal.
    /// If the output buffer is smaller than the expired batch, the next
    /// reprogrammed deadline remains expired and causes another prompt IRQ.
    ///
    /// # Safety
    /// Must be called on the CPU that owns this timer instance.
    pub unsafe fn process_expired(
        &self,
        output: &mut [Option<TimerEvent>],
    ) -> Result<usize, DriverError> {
        let count = self.service.drain_expired(output);
        // SAFETY: Same-CPU requirement is part of this method's contract.
        unsafe { self.reprogram()? };
        Ok(count)
    }

    /// Reprogram the hardware from the current queue head.
    ///
    /// # Safety
    /// Must run at CPL 0 on the CPU that owns this local APIC timer.
    pub unsafe fn reprogram(&self) -> Result<(), DriverError> {
        let Some(deadline) = self.service.next_deadline() else {
            // SAFETY: Timer handle is bound to this CPU by construction.
            unsafe { self.timer.stop() };
            return Ok(());
        };

        if deadline.timestamp_ns() == u64::MAX {
            // Infinite deadlines must never be encoded as an armed TSC value.
            unsafe { self.timer.stop() };
            return Ok(());
        }

        let absolute_tsc = monotonic_deadline_to_tsc(
            deadline.timestamp_ns(),
            self.service.now_ns(),
            TscClockSource::read_tsc(),
            self.tsc_frequency_hz,
        ).ok_or(DriverError::InvalidConfiguration)?;

        // SAFETY: Absolute TSC value was converted from a monotonic deadline.
        unsafe { self.timer.arm(absolute_tsc) }
    }

    /// Access the underlying service for read-only diagnostics.
    pub fn pending(&self) -> usize {
        self.service.pending()
    }

    /// Read the configured TSC frequency.
    pub const fn tsc_frequency_hz(&self) -> u64 {
        self.tsc_frequency_hz
    }

    /// Return the IA32_TSC_DEADLINE MSR index used by this backend.
    pub const fn deadline_msr(&self) -> u32 {
        IA32_TSC_DEADLINE
    }
}
