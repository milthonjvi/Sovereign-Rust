//! Fixed-capacity, allocation-free timer queue.
//!
//! The queue is a binary min-heap ordered by (deadline, insertion sequence).
//! It performs O(log N) insertion/removal and O(1) next-deadline queries.
//! Synchronization is deliberately external: callers must protect a shared
//! queue with the kernel's appropriate lock and define IRQ/task ownership.

use super::deadline::Deadline;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct TimerId(pub u64);

/// Opaque event delivered to the timeout consumer when it expires.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimerEvent {
    pub id: TimerId,
    pub deadline: Deadline,
    /// Consumer-defined event type.
    pub kind: u32,
    /// Consumer-owned opaque value; never dereferenced by this queue.
    pub context: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimerQueueError { Full, DuplicateId, SequenceExhausted }

#[derive(Clone, Copy)]
struct Node { event: TimerEvent, sequence: u64 }

/// Bounded timer queue. No heap allocation, callbacks, or hidden locking.
pub struct TimerQueue<const N: usize> {
    heap: [Option<Node>; N],
    len: usize,
    next_sequence: u64,
}

impl<const N: usize> TimerQueue<N> {
    pub const fn new() -> Self {
        Self { heap: [None; N], len: 0, next_sequence: 0 }
    }
    pub const fn len(&self) -> usize { self.len }
    pub const fn capacity(&self) -> usize { N }
    pub const fn is_empty(&self) -> bool { self.len == 0 }
    pub const fn is_full(&self) -> bool { self.len == N }

    fn less(a: Node, b: Node) -> bool {
        let ad = a.event.deadline.timestamp_ns();
        let bd = b.event.deadline.timestamp_ns();
        ad < bd || (ad == bd && a.sequence < b.sequence)
    }
    fn swap(&mut self, a: usize, b: usize) { self.heap.swap(a, b); }
    fn sift_up(&mut self, mut index: usize) {
        while index > 0 {
            let parent = (index - 1) / 2;
            let child_node = self.heap[index].unwrap();
            let parent_node = self.heap[parent].unwrap();
            if !Self::less(child_node, parent_node) { break; }
            self.swap(index, parent);
            index = parent;
        }
    }
    fn sift_down(&mut self, mut index: usize) {
        loop {
            let left = index * 2 + 1;
            if left >= self.len { break; }
            let right = left + 1;
            let mut smallest = left;
            if right < self.len && Self::less(self.heap[right].unwrap(), self.heap[left].unwrap()) {
                smallest = right;
            }
            if !Self::less(self.heap[smallest].unwrap(), self.heap[index].unwrap()) { break; }
            self.swap(index, smallest);
            index = smallest;
        }
    }

    /// Insert an event. IDs must be unique among currently queued events.
    pub fn insert(&mut self, event: TimerEvent) -> Result<(), TimerQueueError> {
        if self.len == N { return Err(TimerQueueError::Full); }
        if self.contains(event.id) { return Err(TimerQueueError::DuplicateId); }
        if self.next_sequence == u64::MAX { return Err(TimerQueueError::SequenceExhausted); }
        let node = Node { event, sequence: self.next_sequence };
        self.next_sequence += 1;
        self.heap[self.len] = Some(node);
        self.len += 1;
        self.sift_up(self.len - 1);
        Ok(())
    }

    pub fn contains(&self, id: TimerId) -> bool {
        self.heap[..self.len].iter().any(|slot| slot.map(|node| node.event.id == id).unwrap_or(false))
    }

    pub fn next_deadline(&self) -> Option<Deadline> {
        if self.len == 0 { None } else { Some(self.heap[0].unwrap().event.deadline) }
    }

    /// Remove a queued event by ID. O(N) lookup plus O(log N) heap repair.
    pub fn cancel(&mut self, id: TimerId) -> Option<TimerEvent> {
        let index = self.heap[..self.len].iter().position(|slot|
            slot.map(|node| node.event.id == id).unwrap_or(false))?;
        let removed = self.heap[index].take().unwrap().event;
        self.len -= 1;
        if index != self.len {
            self.heap[index] = self.heap[self.len].take();
            if index > 0 && Self::less(self.heap[index].unwrap(), self.heap[(index - 1) / 2].unwrap()) {
                self.sift_up(index);
            } else {
                self.sift_down(index);
            }
        }
        Some(removed)
    }

    /// Pop the earliest event only if its deadline has expired.
    pub fn pop_expired(&mut self, now_ns: u64) -> Option<TimerEvent> {
        let event = self.heap.get(0).and_then(|slot| *slot)?.event;
        if !event.deadline.is_expired(now_ns) { return None; }
        self.cancel(event.id)
    }

    /// Remove every event. Returns the number discarded.
    pub fn clear(&mut self) -> usize {
        let old_len = self.len;
        for slot in self.heap[..self.len].iter_mut() { *slot = None; }
        self.len = 0;
        old_len
    }
}

impl<const N: usize> Default for TimerQueue<N> {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(id: u64, deadline: u64) -> TimerEvent {
        TimerEvent { id: TimerId(id), deadline: Deadline::at_ns(deadline), kind: 1, context: id as usize }
    }
    #[test]
    fn orders_events_and_preserves_fifo_for_equal_deadlines() {
        let mut q = TimerQueue::<4>::new();
        q.insert(event(1, 30)).unwrap();
        q.insert(event(2, 10)).unwrap();
        q.insert(event(3, 10)).unwrap();
        assert_eq!(q.next_deadline(), Some(Deadline::at_ns(10)));
        assert_eq!(q.pop_expired(9), None);
        assert_eq!(q.pop_expired(10).unwrap().id, TimerId(2));
        assert_eq!(q.pop_expired(10).unwrap().id, TimerId(3));
        assert_eq!(q.pop_expired(30).unwrap().id, TimerId(1));
        assert!(q.is_empty());
    }
    #[test]
    fn cancellation_repairs_heap() {
        let mut q = TimerQueue::<5>::new();
        q.insert(event(1, 50)).unwrap();
        q.insert(event(2, 10)).unwrap();
        q.insert(event(3, 30)).unwrap();
        assert_eq!(q.cancel(TimerId(2)).unwrap().deadline, Deadline::at_ns(10));
        assert_eq!(q.pop_expired(30).unwrap().id, TimerId(3));
        assert_eq!(q.pop_expired(50).unwrap().id, TimerId(1));
    }
    #[test]
    fn rejects_capacity_and_duplicate_ids() {
        let mut q = TimerQueue::<1>::new();
        q.insert(event(1, 1)).unwrap();
        assert_eq!(q.insert(event(1, 2)), Err(TimerQueueError::DuplicateId));
        assert_eq!(q.insert(event(2, 2)), Err(TimerQueueError::Full));
    }
    #[test]
    fn clear_resets_queue() {
        let mut q = TimerQueue::<3>::new();
        q.insert(event(1, 1)).unwrap();
        q.insert(event(2, 2)).unwrap();
        assert_eq!(q.clear(), 2);
        assert!(q.is_empty());
        assert_eq!(q.next_deadline(), None);
    }
}
