//! Legacy PIT-driven timer service fallback.
//!
//! This backend is deliberately coarse: the periodic IRQ0 tick advances the
//! monotonic clock and requests deferred expiration processing. It is used
//! when a local-APIC TSC-deadline queue is unavailable; it never executes
//! callbacks or drains events from the hard IRQ handler.

use crate::{
    drivers::{
        timer::pit::{Pit, PitIrqError, take_periodic_work_pending, signal_periodic_work_pending},
    },
};

use super::{
    deadline::Deadline,
    pit_clock::PitClockSource,
    service::{TimerService, TimerServiceError},
    timer_queue::{TimerEvent, TimerId},
};

/// Errors while initializing or controlling the PIT-backed timer service.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PitTimerServiceError {
    Pit(PitIrqError),
    ClockSourceUnavailable,
    Service(TimerServiceError),
}

/// BSP timer service driven by periodic legacy PIC IRQ0 ticks.
pub struct PitTimerService<const N: usize> {
    pit: Pit,
    service: TimerService<PitClockSource, N>,
}

impl<const N: usize> PitTimerService<N> {
    /// Configure PIT IRQ0 and construct the monotonic timer service.
    ///
    /// # Safety
    /// Must run on the BSP at CPL 0 with IF clear after interrupt bootstrap
    /// initialized the legacy PIC. The caller must own PIT channel 0.
    pub unsafe fn initialize_bootstrap(
        requested_hz: u32,
        initial_time_ns: u64,
    ) -> Result<Self, PitTimerServiceError> {
        let mut pit = Pit::new();
        // SAFETY: Forwarded from this function's PIT port/PIC ownership contract.
        unsafe { pit.initialize_periodic_irq(requested_hz) }
            .map_err(PitTimerServiceError::Pit)?;

        let Some(source) = PitClockSource::new() else {
            // SAFETY: Initialization succeeded and IF remains clear.
            let _ = unsafe { pit.shutdown_periodic_irq() };
            return Err(PitTimerServiceError::ClockSourceUnavailable);
        };
        let service = match TimerService::new(source, initial_time_ns) {
            Ok(service) => service,
            Err(error) => {
                // SAFETY: Roll back the IRQ registration on bootstrap CPU.
                let _ = unsafe { pit.shutdown_periodic_irq() };
                return Err(PitTimerServiceError::Service(error));
            }
        };
        Ok(Self { pit, service })
    }

    /// Current monotonic time in nanoseconds.
    #[inline]
    pub fn now_ns(&self) -> u64 {
        self.service.now_ns()
    }

    /// Schedule an absolute event. The next PIT tick will process it if due.
    pub fn schedule_at(
        &self,
        id: TimerId,
        deadline: Deadline,
        kind: u32,
        context: usize,
    ) -> Result<(), TimerServiceError> {
        self.service.schedule_at(id, deadline, kind, context)
    }

    /// Schedule an event after a relative duration.
    pub fn schedule_after(
        &self,
        id: TimerId,
        delay_ns: u64,
        kind: u32,
        context: usize,
    ) -> Result<Deadline, TimerServiceError> {
        self.service.schedule_after(id, delay_ns, kind, context)
    }

    /// Cancel a queued event. No hardware reprogramming is needed for periodic PIT.
    pub fn cancel(&self, id: TimerId) -> Option<TimerEvent> {
        self.service.cancel(id)
    }

    /// Drain expired events after a PIT IRQ requested deferred work.
    ///
    /// Returns None when no tick notification is pending. An empty output
    /// buffer leaves the queue untouched and reasserts deferred work. If the
    /// buffer fills while more events are already expired, another deferred
    /// pass is requested immediately rather than waiting for another tick.
    pub fn process_pending(
        &self,
        output: &mut [Option<TimerEvent>],
    ) -> Option<usize> {
        if !take_periodic_work_pending() {
            return None;
        }
        if output.is_empty() {
            signal_periodic_work_pending();
            return Some(0);
        }

        let count = self.service.drain_expired(output);
        if self.service.next_deadline()
            .is_some_and(|deadline| deadline.is_expired(self.service.now_ns()))
        {
            signal_periodic_work_pending();
        }
        Some(count)
    }

    /// Number of events currently queued.
    pub fn pending(&self) -> usize {
        self.service.pending()
    }

    /// Disable IRQ0 delivery and unregister the PIT tick callback.
    ///
    /// # Safety
    /// Must run on the BSP at CPL 0 with IF clear and no PIT handler in flight.
    pub unsafe fn shutdown(&mut self) -> Result<(), PitTimerServiceError> {
        // SAFETY: The caller fulfills the PIT teardown contract.
        unsafe { self.pit.shutdown_periodic_irq() }
            .map_err(PitTimerServiceError::Pit)
    }
}
