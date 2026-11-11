//! Lock-free single-producer/single-consumer UART receive queue.
//!
//! The producer is normally the UART interrupt handler and the consumer is
//! normal kernel code. The producer must be serialized: nested/concurrent IRQ
//! producers are not supported. The queue never allocates or blocks.

use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

/// Fixed-capacity byte queue for one serialized producer and one consumer.
///
/// One slot is deliberately left unused to distinguish full from empty, so
/// usable capacity is `N - 1`. `N` must be a power of two and at least 2.
pub struct UartRxBuffer<const N: usize> {
    bytes: [AtomicU8; N],
    head: AtomicUsize,
    tail: AtomicUsize,
    dropped: AtomicUsize,
}

impl<const N: usize> UartRxBuffer<N> {
    /// Creates an empty queue.
    ///
    /// # Panics
    /// Panics if N is not a power of two or is smaller than 2.
    pub fn new() -> Self {
        assert!(N >= 2 && N.is_power_of_two());
        Self {
            bytes: core::array::from_fn(|_| AtomicU8::new(0)),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            dropped: AtomicUsize::new(0),
        }
    }

    /// Adds one byte from the producer context.
    ///
    /// Returns false when full; the dropped-byte counter is incremented.
    #[inline]
    pub fn push(&self, byte: u8) -> bool {
        let head = self.head.load(Ordering::Relaxed);
        let next = (head + 1) & (N - 1);
        let tail = self.tail.load(Ordering::Acquire);
        if next == tail {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }

        self.bytes[head].store(byte, Ordering::Relaxed);
        self.head.store(next, Ordering::Release);
        true
    }

    /// Removes one byte from the consumer context.
    #[inline]
    pub fn pop(&self) -> Option<u8> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail == head {
            return None;
        }

        let byte = self.bytes[tail].load(Ordering::Relaxed);
        self.tail.store((tail + 1) & (N - 1), Ordering::Release);
        Some(byte)
    }

    /// Number of bytes currently queued (an instantaneous snapshot).
    #[inline]
    pub fn len(&self) -> usize {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);
        head.wrapping_sub(tail) & (N - 1)
    }

    #[inline]
    pub const fn capacity(&self) -> usize {
        N - 1
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn is_full(&self) -> bool {
        self.len() == self.capacity()
    }

    /// Total number of bytes rejected because the queue was full.
    #[inline]
    pub fn dropped_bytes(&self) -> usize {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl<const N: usize> Default for UartRxBuffer<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::UartRxBuffer;

    #[test]
    fn starts_empty_and_reports_capacity() {
        let queue = UartRxBuffer::<8>::new();
        assert!(queue.is_empty());
        assert!(!queue.is_full());
        assert_eq!(queue.len(), 0);
        assert_eq!(queue.capacity(), 7);
        assert_eq!(queue.pop(), None);
    }

    #[test]
    fn preserves_fifo_order() {
        let queue = UartRxBuffer::<8>::new();
        assert!(queue.push(0x41));
        assert!(queue.push(0x42));
        assert!(queue.push(0x43));
        assert_eq!(queue.len(), 3);
        assert_eq!(queue.pop(), Some(0x41));
        assert_eq!(queue.pop(), Some(0x42));
        assert_eq!(queue.pop(), Some(0x43));
        assert_eq!(queue.pop(), None);
    }

    #[test]
    fn rejects_new_bytes_when_full_and_counts_drops() {
        let queue = UartRxBuffer::<4>::new();
        assert!(queue.push(1));
        assert!(queue.push(2));
        assert!(queue.push(3));
        assert!(queue.is_full());
        assert!(!queue.push(4));
        assert_eq!(queue.dropped_bytes(), 1);
        assert_eq!(queue.pop(), Some(1));
    }

    #[test]
    fn wraps_indices_without_reordering() {
        let queue = UartRxBuffer::<4>::new();
        assert!(queue.push(1));
        assert!(queue.push(2));
        assert_eq!(queue.pop(), Some(1));
        assert!(queue.push(3));
        assert!(queue.push(4));
        assert_eq!(queue.pop(), Some(2));
        assert_eq!(queue.pop(), Some(3));
        assert_eq!(queue.pop(), Some(4));
        assert!(queue.is_empty());
    }
}
