//! Fixed-capacity SPSC transmit queue for interrupt-driven UART output.
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};
const N: usize = 1024;
const MASK: usize = N - 1;

/// Static SPSC queue. Usable capacity is 1023 bytes.
pub struct UartTxBuffer {
    bytes: UnsafeCell<[u8; N]>,
    head: AtomicUsize,
    tail: AtomicUsize,
}
// SAFETY: Release/acquire indices enforce exclusive slot ownership for SPSC.
unsafe impl Sync for UartTxBuffer {}
impl UartTxBuffer {
    pub const fn new() -> Self {
        Self { bytes: UnsafeCell::new([0; N]), head: AtomicUsize::new(0), tail: AtomicUsize::new(0) }
    }
    /// Enqueues the largest prefix that fits.
    /// # Safety
    /// Exactly one serialized producer; the IRQ handler is the sole consumer.
    pub unsafe fn push_slice(&self, input: &[u8]) -> usize {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        let used = head.wrapping_sub(tail) & MASK;
        let count = core::cmp::min(input.len(), (N - 1) - used);
        for (offset, byte) in input[..count].iter().copied().enumerate() {
            // SAFETY: The producer exclusively owns unpublished head slots.
            unsafe { (*self.bytes.get())[(head + offset) & MASK] = byte; }
        }
        self.head.store((head + count) & MASK, Ordering::Release);
        count
    }
    /// Removes one byte for the IRQ consumer.
    /// # Safety
    /// Exactly one serialized consumer.
    #[inline]
    pub unsafe fn pop(&self) -> Option<u8> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail == head { return None; }
        // SAFETY: Acquire observes the published byte.
        let byte = unsafe { (*self.bytes.get())[tail] };
        self.tail.store((tail + 1) & MASK, Ordering::Release);
        Some(byte)
    }
    pub fn len(&self) -> usize {
        self.head.load(Ordering::Acquire).wrapping_sub(self.tail.load(Ordering::Acquire)) & MASK
    }
    pub const fn capacity(&self) -> usize { N - 1 }
    pub fn is_empty(&self) -> bool { self.len() == 0 }
}
impl Default for UartTxBuffer { fn default() -> Self { Self::new() } }

#[cfg(test)]
mod tests {
    use super::UartTxBuffer;
    #[test]
    fn queue_preserves_order_and_capacity() {
        let q = UartTxBuffer::new();
        assert_eq!(q.capacity(), 1023);
        assert_eq!(unsafe { q.push_slice(b"uart") }, 4);
        for byte in b"uart" { assert_eq!(unsafe { q.pop() }, Some(*byte)); }
        assert_eq!(unsafe { q.pop() }, None);
    }
    #[test]
    fn queue_accepts_only_available_prefix() {
        let q = UartTxBuffer::new();
        let data = [0xA5; 1100];
        assert_eq!(unsafe { q.push_slice(&data) }, 1023);
        assert_eq!(unsafe { q.push_slice(&[1, 2, 3]) }, 0);
    }
    #[test]
    fn queue_wraps_without_reordering() {
        let q = UartTxBuffer::new();
        assert_eq!(unsafe { q.push_slice(&[1; 700]) }, 700);
        for _ in 0..600 { assert_eq!(unsafe { q.pop() }, Some(1)); }
        assert_eq!(unsafe { q.push_slice(&[2; 500]) }, 500);
        for _ in 0..100 { assert_eq!(unsafe { q.pop() }, Some(1)); }
        for _ in 0..500 { assert_eq!(unsafe { q.pop() }, Some(2)); }
        assert!(q.is_empty());
    }
}
