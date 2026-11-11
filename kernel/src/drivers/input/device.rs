//! Fixed-capacity input-device identity registry with non-reused boot-lifetime IDs.
use super::event::InputDeviceId;
pub const MAX_INPUT_DEVICES: usize = 64;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputDeviceRegistryError { CapacityExceeded, IdSpaceExhausted }
pub struct InputDeviceRegistry<const N: usize = MAX_INPUT_DEVICES> {
    slots: [Option<InputDeviceId>; N], next_id: u16, exhausted: bool,
}
impl<const N: usize> InputDeviceRegistry<N> {
    pub const fn new() -> Self { Self { slots: [None; N], next_id: 1, exhausted: false } }
    pub fn register(&mut self) -> Result<InputDeviceId, InputDeviceRegistryError> {
        let slot = self.slots.iter().position(Option::is_none)
            .ok_or(InputDeviceRegistryError::CapacityExceeded)?;
        if self.exhausted { return Err(InputDeviceRegistryError::IdSpaceExhausted); }
        let id = InputDeviceId(self.next_id);
        if self.next_id == u16::MAX { self.exhausted = true; } else { self.next_id += 1; }
        self.slots[slot] = Some(id);
        Ok(id)
    }
    pub fn unregister(&mut self, id: InputDeviceId) -> bool {
        if let Some(slot) = self.slots.iter_mut().find(|slot| **slot == Some(id)) {
            *slot = None; true
        } else { false }
    }
    pub fn contains(&self, id: InputDeviceId) -> bool { self.slots.iter().any(|slot| *slot == Some(id)) }
    pub fn active_count(&self) -> usize { self.slots.iter().filter(|slot| slot.is_some()).count() }
    pub const fn capacity(&self) -> usize { N }
}
impl<const N: usize> Default for InputDeviceRegistry<N> { fn default() -> Self { Self::new() } }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retired_slots_do_not_reuse_ids() {
        let mut r = InputDeviceRegistry::<2>::new();
        let a = r.register().unwrap(); let b = r.register().unwrap();
        assert_eq!(a, InputDeviceId(1)); assert_eq!(b, InputDeviceId(2));
        assert_eq!(r.register(), Err(InputDeviceRegistryError::CapacityExceeded));
        assert!(r.unregister(a)); let c = r.register().unwrap();
        assert_eq!(c, InputDeviceId(3)); assert!(!r.contains(a));
        assert!(r.contains(b)); assert!(r.contains(c)); assert_eq!(r.active_count(), 2);
    }
}
