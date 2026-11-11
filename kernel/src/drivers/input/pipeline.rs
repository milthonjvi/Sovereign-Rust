//! Allocation-free PS/2 byte-to-event pipeline.
//!
//! Owns the unique SPSC producer endpoint and both protocol decoders. Invoke
//! from one serialized interrupt producer; consume through the paired endpoint.
//! Hardware port reads and PIC EOI remain the IRQ adapter's responsibility.

use super::event::InputEvent;
use super::keyboard::set1::Set1Decoder;
use super::mouse::packet::Ps2MouseDecoder;
use super::queue::{SpscConsumer, SpscProducer, SpscQueue};

pub const STATUS_AUXILIARY: u8 = 1 << 5;
pub const STATUS_TIMEOUT: u8 = 1 << 6;
pub const STATUS_PARITY: u8 = 1 << 7;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InputPipelineStats {
    pub keyboard_bytes: u64,
    pub mouse_bytes: u64,
    pub controller_errors: u64,
    pub dropped_events: u64,
}

/// Single-producer PS/2 decoder and event publisher.
pub struct Ps2InputPipeline<'a, const N: usize> {
    producer: SpscProducer<'a, InputEvent, N>,
    keyboard: Set1Decoder,
    mouse: Ps2MouseDecoder,
    stats: InputPipelineStats,
}

impl<'a, const N: usize> Ps2InputPipeline<'a, N> {
    /// Creates the IRQ-side pipeline and paired task-side consumer.
    pub fn new(queue: &'a mut SpscQueue<InputEvent, N>, wheel_enabled: bool)
        -> (Self, SpscConsumer<'a, InputEvent, N>)
    {
        let (producer, consumer) = queue.split();
        (
            Self {
                producer,
                keyboard: Set1Decoder::new(),
                mouse: Ps2MouseDecoder::new(wheel_enabled),
                stats: InputPipelineStats::default(),
            },
            consumer,
        )
    }

    /// Process one byte read from i8042 data port 0x60. Error bytes are dropped.
    /// This routine never allocates, blocks, or performs port I/O.
    #[inline]
    pub fn feed(&mut self, status: u8, byte: u8) {
        if status & (STATUS_TIMEOUT | STATUS_PARITY) != 0 {
            self.stats.controller_errors = self.stats.controller_errors.saturating_add(1);
            return;
        }

        if status & STATUS_AUXILIARY != 0 {
            self.stats.mouse_bytes = self.stats.mouse_bytes.saturating_add(1);
            let mut events = [None; 3];
            let count = self.mouse.feed(byte, &mut events);
            for event in events.into_iter().take(count).flatten() {
                self.publish(event);
            }
        } else {
            self.stats.keyboard_bytes = self.stats.keyboard_bytes.saturating_add(1);
            if let Some(event) = self.keyboard.feed(byte) {
                self.publish(event);
            }
        }
    }

    #[inline]
    fn publish(&mut self, event: InputEvent) {
        if self.producer.push(event).is_err() {
            self.stats.dropped_events = self.stats.dropped_events.saturating_add(1);
        }
    }

    /// Snapshot counters; call only from the sole producer context.
    pub const fn stats(&self) -> InputPipelineStats { self.stats }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drivers::input::event::{KeyCode, MouseButtons};

    #[test]
    fn routes_keyboard_and_mouse_into_one_ordered_stream() {
        let queue = SpscQueue::<InputEvent, 16>::new();
        let (mut pipeline, mut consumer) = Ps2InputPipeline::new(&queue, false);
        pipeline.feed(0, 0x1E);
        pipeline.feed(STATUS_AUXILIARY, 0x09);
        pipeline.feed(STATUS_AUXILIARY, 0x04);
        pipeline.feed(STATUS_AUXILIARY, 0x00);

        assert_eq!(consumer.pop(), Some(InputEvent::Key {
            code: KeyCode::A, pressed: true, repeat: false
        }));
        assert_eq!(consumer.pop(), Some(InputEvent::MouseMotion { dx: 4, dy: 0 }));
        assert_eq!(consumer.pop(), Some(InputEvent::MouseButtons {
            buttons: MouseButtons::from_bits(1)
        }));
        assert_eq!(consumer.pop(), None);
        assert_eq!(pipeline.stats().keyboard_bytes, 1);
        assert_eq!(pipeline.stats().mouse_bytes, 3);
    }

    #[test]
    fn rejects_corrupt_bytes_and_counts_queue_overflow() {
        let queue = SpscQueue::<InputEvent, 2>::new();
        let (mut pipeline, mut consumer) = Ps2InputPipeline::new(&queue, false);
        pipeline.feed(STATUS_PARITY, 0x1E);
        pipeline.feed(0, 0x1E);
        pipeline.feed(0, 0x30);
        assert_eq!(pipeline.stats().controller_errors, 1);
        assert_eq!(pipeline.stats().dropped_events, 1);
        assert!(consumer.pop().is_some());
    }
}
