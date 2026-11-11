//! Per-CPU x86 TSC-deadline timer queue integration.
//!
//! This adapter connects the generic monotonic event queue to the local APIC
//! TSC-deadline MSR. It is deliberately CPU-local: construct and use one
//! instance on each CPU after its local APIC LVT and timer interrupt vector
//! have been configured. SMP-wide queue changes require an IPI/reprogram policy
//! and are not silently handled by this per-CPU adapter.

use crate::drivers::timer::{TscDeadline, IA32_TSC_DEADLINE};
use crate::drivers::api::DriverError;
use core::marker::PhantomData;

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

/// Errors while bringing the BSP timer interrupt and deadline queue online.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootstrapTimerInitError {
    Interrupt(super::interrupt::TimerInterruptError),
    Queue(TscDeadlineQueueError),
}

/// Outcome of draining expired timers and attempting to re-arm the comparator.
///
/// Expired events remain delivered in the caller's output buffer even when
/// hardware reprogramming fails. The caller must process those events and
/// arrange a deferred retry; the queue also reasserts its pending-work signal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimerProcessReport {
    pub expired: usize,
    pub reprogram_error: Option<DriverError>,
    pub buffer_unavailable: bool,
}

/// Outcome of cancelling a queued timer and updating the comparator.
///
/// `removed` is authoritative even if comparator reprogramming fails: the
/// event has already been removed from the software queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimerCancelReport {
    pub removed: Option<TimerEvent>,
    pub reprogram_error: Option<DriverError>,
}

/// Per-CPU timer queue driven by the local APIC TSC-deadline facility.
pub struct TscDeadlineQueue<S: ClockSource, const N: usize> {
    service: TimerService<S, N>,
    timer: TscDeadline,
    tsc_frequency_hz: u64,
    // Prevent moving or sharing this local-APIC handle across CPU contexts.
    _cpu_local: PhantomData<*mut ()>,
}

impl<S: ClockSource, const N: usize> TscDeadlineQueue<S, N> {
    /// Bring up the BSP TSC-deadline path in a safe, ordered sequence.
    ///
    /// Configures the local APIC LVT masked, registers the hard-IRQ signal
    /// handler, constructs the software queue and MSR controller, then unmasks
    /// the LVT. On any failure after registration, the interrupt is torn down.
    ///
    /// # Safety
    /// Must run on the bootstrap CPU at CPL 0 with IF clear, after
    /// initialize_bootstrap() selected x2APIC. The supplied clocksource and
    /// TSC frequency must be valid and stable for this CPU.
    pub unsafe fn initialize_bootstrap(
        vector: u8,
        source: S,
        initial_time_ns: u64,
        tsc_frequency_hz: u64,
    ) -> Result<Self, BootstrapTimerInitError> {
        // SAFETY: Forwarded from this function's BSP/CPL0/IF-clear contract.
        unsafe { super::interrupt::register_timer_interrupt(vector) }
            .map_err(BootstrapTimerInitError::Interrupt)?;

        // SAFETY: Registration has configured the LVT masked and published the
        // handler, satisfying TscDeadlineQueue::new's hardware preconditions.
        let queue = match unsafe { Self::new(source, initial_time_ns, tsc_frequency_hz) } {
            Ok(queue) => queue,
            Err(error) => {
                // SAFETY: Same serialized bootstrap initialization context.
                let _ = unsafe { super::interrupt::unregister_timer_interrupt() };
                return Err(BootstrapTimerInitError::Queue(error));
            }
        };

        // SAFETY: The queue and deadline MSR controller are now ready.
        if let Err(error) = unsafe { super::interrupt::unmask_timer_interrupt() } {
            // SAFETY: No timer interrupt can be in flight while IF is clear.
            let _ = unsafe { super::interrupt::unregister_timer_interrupt() };
            return Err(BootstrapTimerInitError::Interrupt(error));
        }
        Ok(queue)
    }

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
        let service = TimerService::new(source, initial_time_ns)
            .map_err(TscDeadlineQueueError::Service)?;
        // SAFETY: The constructor's caller contract guarantees CPL 0 and
        // TSC-deadline feature support on this local CPU.
        let timer = unsafe { TscDeadline::new() }
            .map_err(TscDeadlineQueueError::Hardware)?;
        // SAFETY: Constructor is bound to this CPU and the timer handle was
        // created after validating TSC-deadline support.
        unsafe { timer.stop() };
        Ok(Self { service, timer, tsc_frequency_hz, _cpu_local: PhantomData })
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
    pub unsafe fn cancel(&self, id: TimerId) -> TimerCancelReport {
        let removed = self.service.cancel(id);
        // SAFETY: Same-CPU requirement is part of this method's contract.
        let reprogram_error = unsafe { self.reprogram() }.err();
        if reprogram_error.is_some() {
            super::interrupt::signal_timer_work_pending();
        }
        TimerCancelReport { removed, reprogram_error }
    }

    /// Drain expired events and program the next queued deadline.
    ///
    /// Call from deferred/task context after consuming the timer IRQ signal.
    /// If the output buffer is smaller than the expired batch, the next
    /// reprogrammed deadline remains expired and causes another prompt IRQ.
    /// An empty buffer is not drained or re-armed; deferred work is signaled
    /// again so the caller can retry with usable storage.
    ///
    /// # Safety
    /// Must be called on the CPU that owns this timer instance.
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
            // Preserve a retry request: the current batch is already in
            // caller-owned storage, while remaining deadlines still need
            // another attempt to arm the local comparator.
            super::interrupt::signal_timer_work_pending();
        }
        TimerProcessReport { expired, reprogram_error, buffer_unavailable: false }
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

        // Bracket the monotonic clock read with TSC samples. Using the
        // midpoint aligns the two clock domains and avoids programming a
        // deadline early by the clock-read latency.
        let tsc_before = TscClockSource::read_tsc();
        let now_ns = self.service.now_ns();
        let tsc_after = TscClockSource::read_tsc();
        let now_tsc = tsc_before.wrapping_add(tsc_after.wrapping_sub(tsc_before) / 2);
        let absolute_tsc = monotonic_deadline_to_tsc(
            deadline.timestamp_ns(),
            now_ns,
            now_tsc,
            self.tsc_frequency_hz,
        ).ok_or(DriverError::InvalidConfiguration)?;

        // SAFETY: Absolute TSC value was converted from a monotonic deadline.
        unsafe { self.timer.arm(absolute_tsc) }
    }

    /// Next absolute deadline currently queued for this CPU.
    pub fn next_deadline(&self) -> Option<Deadline> {
        self.service.next_deadline()
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
