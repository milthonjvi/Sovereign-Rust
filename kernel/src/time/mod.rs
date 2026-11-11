//! Monotonic time and deadline primitives.
//!
//! Hardware timer drivers provide counters; this subsystem converts them into
//! kernel time. Hardware selection and interrupt programming remain explicit
//! platform-initialization responsibilities.

pub mod clock;
pub mod deadline;
pub mod timer_queue;

pub use clock::{ns_to_ticks_ceil, ticks_to_ns, ClockSource, MonotonicClock};
pub use deadline::{earlier, Deadline};
pub use timer_queue::{TimerEvent, TimerId, TimerQueue, TimerQueueError};
