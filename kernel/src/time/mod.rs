//! Monotonic time, deadline scheduling, and timer-service primitives.
//!
//! Hardware timer drivers provide counters and interrupt sources. This layer
//! owns clock conversion, bounded timeout queues, and deferred event delivery.

pub mod clock;
pub mod deadline;
pub mod timer_queue;
pub mod periodic;
pub mod hpet_clock;
pub mod tsc_clock;
pub mod service;
pub mod interrupt;
pub mod apic_deadline;

pub use clock::{ns_to_ticks_ceil, ticks_to_ns, ClockSource, MonotonicClock};
pub use deadline::{earlier, Deadline};
pub use timer_queue::{TimerEvent, TimerId, TimerQueue, TimerQueueError};
pub use periodic::{PeriodicExpiration, PeriodicMode, PeriodicTimer};
pub use hpet_clock::{hpet_frequency_hz, HpetClockSource};
pub use tsc_clock::{monotonic_deadline_to_tsc, TscClockSource};
pub use service::{TimerService, TimerServiceError};

pub use interrupt::{register_timer_interrupt, unmask_timer_interrupt, unregister_timer_interrupt, take_timer_work_pending, timer_work_pending, TimerInterruptError};

pub use apic_deadline::{TscDeadlineQueue, TscDeadlineQueueError};
