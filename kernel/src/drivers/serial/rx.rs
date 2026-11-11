//! Lock-free single-producer/single-consumer UART receive queue.
//!
//! The producer is normally the UART interrupt handler and the consumer is
//! normal kernel code. The producer must be serialized: nested/concurrent IRQ
//! producers are not supported. The queue never allocates or blocks.
//!
//! Payload bytes use UnsafeCell rather than per-byte atomics. Release/acquire
//! publication on the indices provides the synchronization needed for SPSC.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};

#[repr(align(64))]
struct CachePadded<T>(T);

/// Fixed-capacity byte queue for one serialized producer and one consumer.
///
/// One slot is deliberately left unused to distinguish full from empty, so
/// usable capacity is `N - 1`. `N` must be a power of two and at least 2.
pub struct UartRxBuffer<const N: usize> {
    bytes: UnsafeCell<[u8; N]>,
    head: CachePadded<AtomicUsize>,
    tail: CachePadded<AtomicUsize>,
    dropped: CachePadded<AtomicUsize>,
}

// SAFETY: The API is strictly SPSC. The producer exclusively writes the slot
// at head and publishes it with Release; the consumer acquires head before
// reading and releases tail before the producer may reuse that slot.
unsafe impl<const N: usize> Sync for UartRxBuffer<N> {}

impl<const N: usize> UartRxBuffer<N> {
    /// Creates an empty queue. Const construction allows static kernel queues.
    ///
    /// # Panics
    /// Panics if N is not a power of two or is smaller than 2.
    pub const fn new() -> Self {
        assert!(N >= 2 && N.is_power_of_two());
        Self {
            bytes: UnsafeCell::new([0; N]),
            head: CachePadded(AtomicUsize::new(0)),
            tail: CachePadded(AtomicUsize::new(0)),
            dropped: CachePadded(AtomicUsize::new(0)),
        }
    }

    /// Adds one byte from the serialized producer context.
    ///
    /// Returns false when full; the dropped-byte counter is incremented.
    ///
    /// # Safety
    /// Exactly one producer may call push for this queue. Calls must not be
    /// nested or concurrent; normally only one UART IRQ handler owns producer
    /// access.
    #[inline]
    pub unsafe fn push(&self, byte: u8) -> bool {
        let head = self.head.0.load(Ordering::Relaxed);
        let next = (head + 1) & (N - 1);
        let tail = self.tail.0.load(Ordering::Acquire);
        if next == tail {
            self.dropped.0.fetch_add(1, Ordering::Relaxed);
            return false;
        }

        // SAFETY: SPSC ownership gives the producer exclusive access to the
        // unpublished head slot. The Release store publishes the initialized byte.
        unsafe { (*self.bytes.get())[head] = byte; }
        self.head.0.store(next, Ordering::Release);
        true
    }

    /// Removes one byte from the single consumer context.
    ///
    /// # Safety
    /// Exactly one consumer may call pop for this queue. Calls must not be
    /// concurrent; normally one kernel task owns consumer access.
    #[inline]
    pub unsafe fn pop(&self) -> Option<u8> {
        let tail = self.tail.0.load(Ordering::Relaxed);
        let head = self.head.0.load(Ordering::Acquire);
        if tail == head {
            return None;
        }

        // SAFETY: Acquire observed the producer's published head. The producer
        // cannot reuse this slot until the consumer releases the updated tail.
        let byte = unsafe { (*self.bytes.get())[tail] };
        self.tail.0.store((tail + 1) & (N - 1), Ordering::Release);
        Some(byte)
    }

    /// Number of bytes currently queued (an instantaneous snapshot).
    #[inline]
    pub fn len(&self) -> usize {
        let head = self.head.0.load(Ordering::Acquire);
        let tail = self.tail.0.load(Ordering::Acquire);
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
        self.dropped.0.load(Ordering::Relaxed)
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
        assert_eq!(unsafe { queue.pop() }, None);
    }

    #[test]
    fn preserves_fifo_order() {
        let queue = UartRxBuffer::<8>::new();
        assert!(unsafe { queue.push(0x41) });
        assert!(unsafe { queue.push(0x42) });
        assert!(unsafe { queue.push(0x43) });
        assert_eq!(queue.len(), 3);
        assert_eq!(unsafe { queue.pop() }, Some(0x41));
        assert_eq!(unsafe { queue.pop() }, Some(0x42));
        assert_eq!(unsafe { queue.pop() }, Some(0x43));
        assert_eq!(unsafe { queue.pop() }, None);
    }

    #[test]
    fn rejects_new_bytes_when_full_and_counts_drops() {
        let queue = UartRxBuffer::<4>::new();
        assert!(unsafe { queue.push(1) });
        assert!(unsafe { queue.push(2) });
        assert!(unsafe { queue.push(3) });
        assert!(queue.is_full());
        assert!(!unsafe { queue.push(4) });
        assert_eq!(queue.dropped_bytes(), 1);
        assert_eq!(unsafe { queue.pop() }, Some(1));
    }

    #[test]
    fn wraps_indices_without_reordering() {
        let queue = UartRxBuffer::<4>::new();
        assert!(unsafe { queue.push(1) });
        assert!(unsafe { queue.push(2) });
        assert_eq!(unsafe { queue.pop() }, Some(1));
        assert!(unsafe { queue.push(3) });
        assert!(unsafe { queue.push(4) });
        assert_eq!(unsafe { queue.pop() }, Some(2));
        assert_eq!(unsafe { queue.pop() }, Some(3));
        assert_eq!(unsafe { queue.pop() }, Some(4));
        assert!(queue.is_empty());
    }

    #[test]
    fn can_be_const_initialized_for_static_kernel_storage() {
        static QUEUE: UartRxBuffer<16> = UartRxBuffer::new();
        assert!(QUEUE.is_empty());
        assert!(unsafe { QUEUE.push(0x5A) });
        assert_eq!(unsafe { QUEUE.pop() }, Some(0x5A));
    }
}
