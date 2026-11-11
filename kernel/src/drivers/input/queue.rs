//! Fixed-capacity event queue. Synchronization is owned by the caller.
pub struct EventQueue<T: Copy, const N: usize> { entries: [Option<T>; N], read: usize, write: usize, len: usize }
impl<T: Copy, const N: usize> EventQueue<T, N> {
 pub const fn new() -> Self { Self { entries: [None; N], read: 0, write: 0, len: 0 } }
 pub const fn capacity(&self) -> usize { N }
 pub const fn len(&self) -> usize { self.len }
 pub const fn is_empty(&self) -> bool { self.len == 0 }
 pub const fn is_full(&self) -> bool { self.len == N }
 pub fn push(&mut self, event: T) -> Result<(), T> {
  if N == 0 || self.len == N { return Err(event); }
  self.entries[self.write] = Some(event); self.write = (self.write + 1) % N; self.len += 1; Ok(())
 }
 pub fn pop(&mut self) -> Option<T> {
  if self.len == 0 { return None; }
  let event = self.entries[self.read].take(); self.read = (self.read + 1) % N; self.len -= 1; event
 }
}
impl<T: Copy, const N: usize> Default for EventQueue<T, N> { fn default() -> Self { Self::new() } }
#[cfg(test)]
mod tests {
 use super::EventQueue;
 #[test] fn fifo_wraparound_and_full() {
  let mut q = EventQueue::<u8, 2>::new(); assert_eq!(q.push(1), Ok(())); assert_eq!(q.push(2), Ok(()));
  assert_eq!(q.push(3), Err(3)); assert_eq!(q.pop(), Some(1)); assert_eq!(q.push(3), Ok(()));
  assert_eq!(q.pop(), Some(2)); assert_eq!(q.pop(), Some(3)); assert_eq!(q.pop(), None);
 }
 #[test] fn zero_capacity_rejects() { let mut q = EventQueue::<u8, 0>::new(); assert_eq!(q.push(1), Err(1)); }
}


/// Lock-free single-producer/single-consumer ring for interrupt-to-task events.
///
/// The producer and consumer handles are unique and cannot be cloned. The
/// producer publishes initialized slots with Release ordering; the consumer
/// observes them with Acquire ordering. A slot is not reused until the
/// consumer releases it. One slot is reserved to distinguish full from empty.
///
/// This is suitable for a single legacy-PIC input producer on the BSP and one
/// task consumer. Do not route IRQ1/IRQ12 to different CPUs or create multiple
/// producers without replacing this queue with an MPSC design.
pub struct SpscQueue<T: Copy, const N: usize> {
    slots: [core::cell::UnsafeCell<core::mem::MaybeUninit<T>>; N],
    head: core::sync::atomic::AtomicUsize,
    tail: core::sync::atomic::AtomicUsize,
}

// SAFETY: Only the unique producer writes a slot and only the unique consumer
// reads it. Release/Acquire indices transfer ownership of each slot.
unsafe impl<T: Copy + Send, const N: usize> Sync for SpscQueue<T, N> {}
unsafe impl<T: Copy + Send, const N: usize> Send for SpscQueue<T, N> {}

impl<T: Copy, const N: usize> SpscQueue<T, N> {
    /// Creates an empty queue. Usable capacity is N-1; N must be at least 2.
    pub const fn new() -> Self {
        Self {
            slots: [const {
                core::cell::UnsafeCell::new(core::mem::MaybeUninit::uninit())
            }; N],
            head: core::sync::atomic::AtomicUsize::new(0),
            tail: core::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Splits the queue into its unique producer and consumer endpoints.
    pub fn split(&self) -> (SpscProducer<'_, T, N>, SpscConsumer<'_, T, N>) {
        (
            SpscProducer { queue: self },
            SpscConsumer { queue: self },
        )
    }

    pub const fn capacity(&self) -> usize {
        if N == 0 { 0 } else { N - 1 }
    }
}

impl<T: Copy, const N: usize> Default for SpscQueue<T, N> {
    fn default() -> Self { Self::new() }
}

/// Unique SPSC producer endpoint. Keep it in the sole interrupt producer.
pub struct SpscProducer<'a, T: Copy, const N: usize> {
    queue: &'a SpscQueue<T, N>,
}

impl<T: Copy, const N: usize> SpscProducer<'_, T, N> {
    /// Enqueues without allocation, locks, or waiting. Returns the event if full.
    #[inline]
    pub fn push(&mut self, value: T) -> Result<(), T> {
        if N < 2 { return Err(value); }
        let head = self.queue.head.load(core::sync::atomic::Ordering::Relaxed);
        let next = if head + 1 == N { 0 } else { head + 1 };
        if next == self.queue.tail.load(core::sync::atomic::Ordering::Acquire) {
            return Err(value);
        }

        // SAFETY: The producer exclusively owns the slot until publishing head.
        unsafe {
            (*self.queue.slots[head].get()).write(value);
        }
        self.queue.head.store(next, core::sync::atomic::Ordering::Release);
        Ok(())
    }
}

/// Unique SPSC consumer endpoint. Keep it in the sole task consumer.
pub struct SpscConsumer<'a, T: Copy, const N: usize> {
    queue: &'a SpscQueue<T, N>,
}

impl<T: Copy, const N: usize> SpscConsumer<'_, T, N> {
    /// Dequeues one event without allocation, locks, or waiting.
    #[inline]
    pub fn pop(&mut self) -> Option<T> {
        if N < 2 { return None; }
        let tail = self.queue.tail.load(core::sync::atomic::Ordering::Relaxed);
        if tail == self.queue.head.load(core::sync::atomic::Ordering::Acquire) {
            return None;
        }

        // SAFETY: Acquire observed the producer's publication. The producer
        // cannot reuse this slot until the updated tail is released below.
        let value = unsafe { (*self.queue.slots[tail].get()).assume_init_read() };
        let next = if tail + 1 == N { 0 } else { tail + 1 };
        self.queue.tail.store(next, core::sync::atomic::Ordering::Release);
        Some(value)
    }
}

#[cfg(test)]
mod spsc_tests {
    use super::SpscQueue;

    #[test]
    fn spsc_fifo_full_and_wraparound() {
        let queue = SpscQueue::<u8, 3>::new();
        let (mut producer, mut consumer) = queue.split();
        assert_eq!(producer.push(10), Ok(()));
        assert_eq!(producer.push(20), Ok(()));
        assert_eq!(producer.push(30), Err(30));
        assert_eq!(consumer.pop(), Some(10));
        assert_eq!(producer.push(30), Ok(()));
        assert_eq!(consumer.pop(), Some(20));
        assert_eq!(consumer.pop(), Some(30));
        assert_eq!(consumer.pop(), None);
    }

    #[test]
    fn spsc_rejects_zero_and_one_slot_queues() {
        let zero = SpscQueue::<u8, 0>::new();
        let (mut zp, mut zc) = zero.split();
        assert_eq!(zp.push(1), Err(1));
        assert_eq!(zc.pop(), None);

        let one = SpscQueue::<u8, 1>::new();
        let (mut op, mut oc) = one.split();
        assert_eq!(op.push(1), Err(1));
        assert_eq!(oc.pop(), None);
    }
}
