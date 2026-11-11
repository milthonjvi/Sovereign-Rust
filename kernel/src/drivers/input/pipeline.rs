//! Allocation-free PS/2 byte-to-event pipeline.
//!
//! Owns the unique SPSC producer endpoint and both protocol decoders. Invoke
//! from one serialized interrupt producer; consume through the paired endpoint.
//! Hardware port reads and PIC EOI remain the IRQ adapter's responsibility.

use super::event::{InputDeviceId, InputEvent};
use super::keyboard::set1::Set1Decoder;
use super::mouse::packet::Ps2MouseDecoder;
use super::queue::{SpscConsumer, SpscProducer, SpscQueue};
use super::ps2::{Ps2Controller, Ps2DataByte, Ps2Error, Ps2MouseProtocol};

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
    resync_required: bool,
}

impl<'a, const N: usize> Ps2InputPipeline<'a, N> {
    /// Creates the IRQ-side pipeline and paired task-side consumer.
    pub fn new(queue: &'a mut SpscQueue<InputEvent, N>, wheel_enabled: bool)
        -> (Self, SpscConsumer<'a, InputEvent, N>)
    {
        Self::new_with_protocol(queue, if wheel_enabled { Ps2MouseProtocol::Wheel } else { Ps2MouseProtocol::Standard })
    }

    /// Constructs decoders using the exact protocol negotiated with the mouse.
    pub fn new_with_protocol(
        queue: &'a mut SpscQueue<InputEvent, N>, protocol: Ps2MouseProtocol,
    ) -> (Self, SpscConsumer<'a, InputEvent, N>) {
        let (producer, consumer) = queue.split();
        (
            Self {
                producer,
                keyboard: Set1Decoder::new(),
                mouse: Ps2MouseDecoder::new_with_protocol(protocol),
                stats: InputPipelineStats::default(),
                resync_required: false,
            },
            consumer,
        )
    }

    /// Processes a status-preserving byte returned by the i8042 controller.
    #[inline]
    pub fn feed_data_byte(&mut self, byte: Ps2DataByte) {
        self.feed(byte.status, byte.value);
    }

    /// Drains at most `budget` pending bytes from the controller.
    ///
    /// This is deliberately bounded for interrupt-context callers. Use a
    /// conservative budget in an IRQ handler and let subsequent IRQs continue
    /// draining; polling-mode callers may choose a larger finite budget.
    /// This method does not acknowledge the PIC: EOI belongs to the interrupt
    /// controller/dispatcher layer, outside this input driver.
    pub fn drain_controller(
        &mut self,
        controller: &Ps2Controller,
        budget: usize,
    ) -> Result<usize, Ps2Error> {
        let mut processed = 0;
        while processed < budget {
            let Some(byte) = controller.try_read_with_status()? else {
                break;
            };
            self.feed_data_byte(byte);
            processed += 1;
        }
        Ok(processed)
    }

    /// Process one byte read from i8042 data port 0x60. Error bytes are dropped.
    /// This routine never allocates, blocks, or performs port I/O.
    #[inline]
    pub fn feed(&mut self, status: u8, byte: u8) {
        if status & (STATUS_TIMEOUT | STATUS_PARITY) != 0 {
            self.stats.controller_errors = self.stats.controller_errors.saturating_add(1);
            self.keyboard.reset_stream();
            self.mouse.reset_stream();
            self.resync_required = true;
            return;
        }

        // A lost transition can leave a key or button logically held forever.
        // Publish a consumer reset before accepting more bytes; while the
        // queue remains full, discard input rather than emit out-of-order state.
        if self.resync_required {
            if self.producer.push(InputEvent::DeviceStateReset { device: InputDeviceId(0) }).is_err() {
                self.stats.dropped_events = self.stats.dropped_events.saturating_add(1);
                return;
            }
            self.keyboard.reset_state();
            self.mouse.reset_stream();
            self.resync_required = false;
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
        if self.resync_required { return; }
        if self.producer.push(event).is_err() {
            self.stats.dropped_events = self.stats.dropped_events.saturating_add(1);
            self.resync_required = true;
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
        let mut queue = SpscQueue::<InputEvent, 16>::new();
        let (mut pipeline, mut consumer) = Ps2InputPipeline::new(&mut queue, false);
        pipeline.feed_data_byte(Ps2DataByte { status: 0, value: 0x1E });
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
    fn corrupt_transport_byte_resets_partial_protocol_frames() {
        let mut queue = SpscQueue::<InputEvent, 8>::new();
        let (mut pipeline, mut consumer) = Ps2InputPipeline::new(&mut queue, false);
        pipeline.feed(STATUS_AUXILIARY, 0x09); // Partial mouse packet.
        pipeline.feed(STATUS_PARITY, 0x00); // Corrupt byte invalidates both streams.
        pipeline.feed(STATUS_AUXILIARY, 0x04); // Invalid header is ignored.
        pipeline.feed(STATUS_AUXILIARY, 0x08);
        pipeline.feed(STATUS_AUXILIARY, 0x02);
        pipeline.feed(STATUS_AUXILIARY, 0x00);
        assert_eq!(consumer.pop(), Some(InputEvent::DeviceStateReset { device: InputDeviceId(0) }));
        assert_eq!(consumer.pop(), Some(InputEvent::MouseMotion { dx: 2, dy: 0 }));
        assert_eq!(pipeline.stats().controller_errors, 1);
    }

    #[test]
    fn queue_overflow_resets_consumer_before_accepting_more_input() {
        let mut queue = SpscQueue::<InputEvent, 3>::new();
        let (mut pipeline, mut consumer) = Ps2InputPipeline::new(&mut queue, false);
        pipeline.feed(0, 0x1E); // Key-down is dropped because queue is filled below.
        // Drain the first key event, then fill the queue and force another drop.
        assert!(matches!(consumer.pop(), Some(InputEvent::Key { code: KeyCode::A, pressed: true, .. })));
        pipeline.feed(0, 0x1E); // Repeat event enters the queue.
        pipeline.feed(0, 0x30); // B event enters the queue.
        pipeline.feed(0, 0x2E); // C event is dropped and requires resync.
        while consumer.pop().is_some() {}
        pipeline.feed(0, 0x1E);
        assert_eq!(consumer.pop(), Some(InputEvent::DeviceStateReset { device: InputDeviceId(0) }));
        assert_eq!(consumer.pop(), Some(InputEvent::Key { code: KeyCode::A, pressed: true, repeat: false }));
        assert_eq!(pipeline.stats().dropped_events, 1);
    }

    #[test]
    fn rejects_corrupt_bytes_and_counts_queue_overflow() {
        let mut queue = SpscQueue::<InputEvent, 2>::new();
        let (mut pipeline, mut consumer) = Ps2InputPipeline::new(&mut queue, false);
        pipeline.feed(STATUS_PARITY, 0x1E);
        pipeline.feed(0, 0x1E);
        pipeline.feed(0, 0x30);
        assert_eq!(pipeline.stats().controller_errors, 1);
        assert_eq!(pipeline.stats().dropped_events, 2);
        assert!(matches!(consumer.pop(), Some(InputEvent::DeviceStateReset { device: InputDeviceId(0) })));
    }
}
