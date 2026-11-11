//! Periodic timer policy independent of hardware and interrupt delivery.

use super::deadline::Deadline;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeriodicMode { FixedRate, FixedDelay }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeriodicExpiration {
    pub scheduled: Deadline,
    pub periods_elapsed: u64,
    pub periods_missed: u64,
    pub next: Deadline,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeriodicTimer { next: Deadline, period_ns: u64, mode: PeriodicMode }

impl PeriodicTimer {
    pub const fn new(first: Deadline, period_ns: u64, mode: PeriodicMode) -> Option<Self> {
        if period_ns == 0 { return None; }
        Some(Self { next: first, period_ns, mode })
    }
    pub const fn next_deadline(&self) -> Deadline { self.next }
    pub const fn period_ns(&self) -> u64 { self.period_ns }
    pub const fn mode(&self) -> PeriodicMode { self.mode }

    /// Service an expired timer and calculate its next absolute deadline.
    /// Fixed-rate mode skips missed intervals instead of causing callback storms.
    pub fn expire(&mut self, now_ns: u64) -> Option<PeriodicExpiration> {
        if !self.next.is_expired(now_ns) { return None; }
        let scheduled = self.next;
        let (periods_elapsed, next_ns) = match self.mode {
            PeriodicMode::FixedRate => {
                let lateness = now_ns.saturating_sub(scheduled.timestamp_ns());
                let periods = lateness / self.period_ns + 1;
                let next = (scheduled.timestamp_ns() as u128)
                    .saturating_add((periods as u128).saturating_mul(self.period_ns as u128));
                (periods, next.min(u64::MAX as u128) as u64)
            }
            PeriodicMode::FixedDelay => (1, now_ns.saturating_add(self.period_ns)),
        };
        self.next = Deadline::at_ns(next_ns);
        Some(PeriodicExpiration { scheduled, periods_elapsed,
            periods_missed: periods_elapsed.saturating_sub(1), next: self.next })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_rate_preserves_cadence_and_reports_missed_periods() {
        let mut timer = PeriodicTimer::new(Deadline::at_ns(100), 10, PeriodicMode::FixedRate).unwrap();
        assert_eq!(timer.expire(99), None);
        let fired = timer.expire(135).unwrap();
        assert_eq!(fired.scheduled, Deadline::at_ns(100));
        assert_eq!(fired.periods_elapsed, 4);
        assert_eq!(fired.periods_missed, 3);
        assert_eq!(fired.next, Deadline::at_ns(140));
    }
    #[test]
    fn fixed_delay_restarts_from_service_time() {
        let mut timer = PeriodicTimer::new(Deadline::at_ns(100), 10, PeriodicMode::FixedDelay).unwrap();
        let fired = timer.expire(135).unwrap();
        assert_eq!(fired.periods_elapsed, 1);
        assert_eq!(fired.periods_missed, 0);
        assert_eq!(fired.next, Deadline::at_ns(145));
    }
    #[test]
    fn rejects_zero_period_and_saturates_overflow() {
        assert!(PeriodicTimer::new(Deadline::at_ns(0), 0, PeriodicMode::FixedRate).is_none());
        let mut timer = PeriodicTimer::new(Deadline::at_ns(u64::MAX - 2), 10, PeriodicMode::FixedRate).unwrap();
        assert_eq!(timer.expire(u64::MAX).unwrap().next, Deadline::INFINITE);
    }
}
