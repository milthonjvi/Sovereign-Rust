//! Allocation-free absolute deadline arithmetic for kernel timeout consumers.

/// An absolute monotonic deadline in nanoseconds.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Deadline(u64);
impl Deadline {
    pub const INFINITE: Self = Self(u64::MAX);
    pub const fn at_ns(timestamp_ns: u64) -> Self { Self(timestamp_ns) }
    pub const fn after_ns(now_ns: u64, duration_ns: u64) -> Self {
        Self(now_ns.saturating_add(duration_ns))
    }
    pub const fn timestamp_ns(self) -> u64 { self.0 }
    /// Infinite deadlines never expire, even if the clock saturates.
    pub const fn is_expired(self, now_ns: u64) -> bool {
        self.0 != u64::MAX && now_ns >= self.0
    }
    /// Infinite deadlines retain the maximum representable remaining duration.
    pub const fn remaining_ns(self, now_ns: u64) -> u64 {
        if self.0 == u64::MAX { u64::MAX } else { self.0.saturating_sub(now_ns) }
    }
}
/// Select the earlier of two deadlines.
pub const fn earlier(a: Deadline, b: Deadline) -> Deadline { if a.0 <= b.0 { a } else { b } }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn computes_remaining_and_expiration() {
        let d = Deadline::after_ns(100, 50);
        assert_eq!(d.timestamp_ns(), 150);
        assert_eq!(d.remaining_ns(120), 30);
        assert!(!d.is_expired(149));
        assert!(d.is_expired(150));
        assert_eq!(d.remaining_ns(200), 0);
    }
    #[test]
    fn saturates_deadline_overflow() {
        assert_eq!(Deadline::after_ns(u64::MAX - 2, 10), Deadline::INFINITE);
    }
    #[test]
    fn infinite_deadline_never_expires() {
        assert!(!Deadline::INFINITE.is_expired(u64::MAX));
        assert_eq!(Deadline::INFINITE.remaining_ns(u64::MAX), u64::MAX);
    }
    #[test]
    fn selects_earliest_deadline() {
        assert_eq!(earlier(Deadline::at_ns(10), Deadline::at_ns(20)), Deadline::at_ns(10));
    }
}
