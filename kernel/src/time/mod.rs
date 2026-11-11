//! Monotonic time, clocksource selection, deadlines, and timer queues.
//!
//! Hardware timer drivers provide counters; this subsystem converts them into
//! kernel time and scheduler deadlines.
