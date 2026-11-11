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
