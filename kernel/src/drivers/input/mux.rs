//! Fair fan-in for independent input transport queues.
//!
//! Each bus/IRQ adapter owns its own SPSC producer. The input task owns all
//! corresponding consumers and polls this mux. Never share one SPSC producer
//! between PS/2 IRQ and USB polling contexts: that violates SPSC ownership.
use super::event::InputEvent;
use super::queue::SpscConsumer;

/// Stable slot assigned by the input subsystem when a source is attached.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct InputSourceId(pub u16);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourcedInputEvent {
    pub source: InputSourceId,
    pub event: InputEvent,
}

/// Round-robin fan-in over independent SPSC queues.
pub struct InputEventMux<'a, const SOURCES: usize, const N: usize> {
    consumers: [Option<SpscConsumer<'a, InputEvent, N>>; SOURCES],
    cursor: usize,
}

impl<'a, const SOURCES: usize, const N: usize> InputEventMux<'a, SOURCES, N> {
    /// Takes ownership of each unique consumer endpoint. Empty slots are allowed.
    pub const fn new(consumers: [Option<SpscConsumer<'a, InputEvent, N>>; SOURCES]) -> Self {
        Self { consumers, cursor: 0 }
    }

    /// Returns one event, scanning sources in round-robin order.
    pub fn pop(&mut self) -> Option<SourcedInputEvent> {
        if SOURCES == 0 { return None; }
        for offset in 0..SOURCES {
            let index = (self.cursor + offset) % SOURCES;
            if let Some(consumer) = self.consumers[index].as_mut() {
                if let Some(event) = consumer.pop() {
                    self.cursor = (index + 1) % SOURCES;
                    return Some(SourcedInputEvent {
                        source: InputSourceId(index as u16),
                        event,
                    });
                }
            }
        }
        None
    }

    pub const fn source_count(&self) -> usize {
        let mut index = 0;
        let mut count = 0;
        while index < SOURCES {
            if self.consumers[index].is_some() { count += 1; }
            index += 1;
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drivers::input::event::{InputDeviceId, KeyCode};
    use crate::drivers::input::queue::SpscQueue;

    #[test]
    fn independent_sources_are_merged_without_sharing_producers() {
        let mut ps2_queue = SpscQueue::<InputEvent, 4>::new();
        let mut usb_queue = SpscQueue::<InputEvent, 4>::new();
        let (mut ps2_producer, ps2_consumer) = ps2_queue.split();
        let (mut usb_producer, usb_consumer) = usb_queue.split();
        ps2_producer.push(InputEvent::Key { code: KeyCode::A, pressed: true, repeat: false }).unwrap();
        usb_producer.push(InputEvent::HidKey {
            device: InputDeviceId(9), usage_page: 0x07, usage: 0x04,
            pressed: true, modifiers: 0,
        }).unwrap();
        let mut mux = InputEventMux::new([Some(ps2_consumer), Some(usb_consumer)]);
        let first = mux.pop().unwrap();
        let second = mux.pop().unwrap();
        assert_ne!(first.source, second.source);
        assert!(mux.pop().is_none());
    }

    #[test]
    fn round_robin_prevents_busy_source_starvation() {
        let mut a = SpscQueue::<InputEvent, 4>::new();
        let mut b = SpscQueue::<InputEvent, 4>::new();
        let (mut ap, ac) = a.split();
        let (mut bp, bc) = b.split();
        let event = InputEvent::Key { code: KeyCode::A, pressed: true, repeat: false };
        ap.push(event).unwrap();
        ap.push(event).unwrap();
        bp.push(event).unwrap();
        let mut mux = InputEventMux::new([Some(ac), Some(bc)]);
        assert_eq!(mux.pop().unwrap().source, InputSourceId(0));
        assert_eq!(mux.pop().unwrap().source, InputSourceId(1));
        assert_eq!(mux.pop().unwrap().source, InputSourceId(0));
    }
}
