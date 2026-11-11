//! Semantic adapter from successful USB HID reports to shared InputEvent.
//! USB owns enumeration, endpoints and transfers; Input owns semantic state.
use super::event::{InputDeviceId, InputEvent};
use super::queue::SpscProducer;
use crate::drivers::usb::hid::{BootKeyboardReport, BootMouseReport, BootMouseReportWheel};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HidPublishStats { pub published: u8, pub dropped: u8 }
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HidInputError { MalformedReport, QueueUnavailable }

/// One adapter per configured USB boot-keyboard interface/collection.
pub struct UsbBootKeyboardAdapter {
    device: InputDeviceId, previous: Option<BootKeyboardReport>, resync_required: bool, pub dropped_events: u64,
}
impl UsbBootKeyboardAdapter {
    pub const fn new(device: InputDeviceId) -> Self {
        Self { device, previous: None, resync_required: false, dropped_events: 0 }
    }
    pub const fn device_id(&self) -> InputDeviceId { self.device }

    /// Emits HID usage transitions, including modifiers E0-E7 as individual keys.
    /// No keymap or character translation occurs here.
    pub fn publish_report<const N: usize>(
        &mut self, bytes: &[u8], producer: &mut SpscProducer<'_, InputEvent, N>,
    ) -> Result<HidPublishStats, HidInputError> {
        let now = BootKeyboardReport::parse(bytes).map_err(|_| HidInputError::MalformedReport)?;
        let mut stats = HidPublishStats::default();
        if self.resync_required {
            if producer.push(InputEvent::DeviceStateReset { device: self.device }).is_err() {
                stats.dropped = 1;
                self.dropped_events = self.dropped_events.saturating_add(1);
                return Ok(stats);
            }
            stats.published = 1;
            self.previous = None;
            self.resync_required = false;
        }
        let old = self.previous.unwrap_or(BootKeyboardReport { modifiers: 0, keys: [0; 6] });
        for i in 0..6 {
            let key = now.keys[i];
            if key != 0 && !now.keys[..i].contains(&key) && !old.keys.contains(&key) {
                self.emit(InputEvent::HidKey { device: self.device, usage_page: 7,
                    usage: key as u16, pressed: true, modifiers: now.modifiers }, producer, &mut stats);
            }
        }
        for i in 0..6 {
            let key = old.keys[i];
            if key != 0 && !old.keys[..i].contains(&key) && !now.keys.contains(&key) {
                self.emit(InputEvent::HidKey { device: self.device, usage_page: 7,
                    usage: key as u16, pressed: false, modifiers: now.modifiers }, producer, &mut stats);
            }
        }
        let changed = old.modifiers ^ now.modifiers;
        for bit in 0..8 {
            let mask = 1u8 << bit;
            if changed & mask != 0 {
                self.emit(InputEvent::HidKey { device: self.device, usage_page: 7,
                    usage: 0xE0 + bit as u16, pressed: now.modifiers & mask != 0,
                    modifiers: now.modifiers }, producer, &mut stats);
            }
        }
        self.previous = Some(now);
        Ok(stats)
    }

    pub fn publish_connected<const N: usize>(
        &mut self, p: &mut SpscProducer<'_, InputEvent, N>,
    ) -> HidPublishStats {
        let mut s = HidPublishStats::default();
        self.emit(InputEvent::DeviceConnected { device: self.device }, p, &mut s); s
    }
    pub fn publish_disconnected<const N: usize>(
        &mut self, p: &mut SpscProducer<'_, InputEvent, N>,
    ) -> HidPublishStats {
        self.previous = None;
        self.resync_required = false;
        let mut s = HidPublishStats::default();
        self.emit(InputEvent::DeviceDisconnected { device: self.device }, p, &mut s); s
    }
    fn emit<const N: usize>(
        &mut self, e: InputEvent, p: &mut SpscProducer<'_, InputEvent, N>, s: &mut HidPublishStats,
    ) {
        if p.push(e).is_ok() { s.published = s.published.saturating_add(1); }
        else {
            s.dropped = s.dropped.saturating_add(1);
            self.dropped_events = self.dropped_events.saturating_add(1);
            self.resync_required = true;
        }
    }
}

/// One adapter per configured USB boot-mouse interface/collection.
pub struct UsbBootMouseAdapter {
    device: InputDeviceId, previous_buttons: Option<u32>, resync_required: bool, pub dropped_events: u64,
}
impl UsbBootMouseAdapter {
    pub const fn new(device: InputDeviceId) -> Self {
        Self { device, previous_buttons: None, resync_required: false, dropped_events: 0 }
    }
    pub const fn device_id(&self) -> InputDeviceId { self.device }
    pub fn publish_report<const N: usize>(
        &mut self, bytes: &[u8], p: &mut SpscProducer<'_, InputEvent, N>,
    ) -> Result<HidPublishStats, HidInputError> {
        let r = BootMouseReport::parse(bytes).map_err(|_| HidInputError::MalformedReport)?;
        self.recover_if_needed(p)?;
        self.pointer(r.x as i32, r.y as i32, 0, r.buttons as u32, p)
    }
    pub fn publish_wheel_report<const N: usize>(
        &mut self, bytes: &[u8], p: &mut SpscProducer<'_, InputEvent, N>,
    ) -> Result<HidPublishStats, HidInputError> {
        let r = BootMouseReportWheel::parse(bytes).map_err(|_| HidInputError::MalformedReport)?;
        self.recover_if_needed(p)?;
        self.pointer(r.base.x as i32, r.base.y as i32, r.wheel as i16, r.base.buttons as u32, p)
    }
    fn recover_if_needed<const N: usize>(
        &mut self, p: &mut SpscProducer<'_, InputEvent, N>,
    ) -> Result<(), HidInputError> {
        if self.resync_required {
            if p.push(InputEvent::DeviceStateReset { device: self.device }).is_err() {
                self.dropped_events = self.dropped_events.saturating_add(1);
                return Err(HidInputError::QueueUnavailable);
            }
            self.previous_buttons = None;
            self.resync_required = false;
        }
        Ok(())
    }
    pub fn publish_connected<const N: usize>(
        &mut self, p: &mut SpscProducer<'_, InputEvent, N>,
    ) -> HidPublishStats {
        let mut s = HidPublishStats::default();
        self.emit(InputEvent::DeviceConnected { device: self.device }, p, &mut s); s
    }
    pub fn publish_disconnected<const N: usize>(
        &mut self, p: &mut SpscProducer<'_, InputEvent, N>,
    ) -> HidPublishStats {
        self.previous_buttons = None;
        self.resync_required = false;
        let mut s = HidPublishStats::default();
        self.emit(InputEvent::DeviceDisconnected { device: self.device }, p, &mut s); s
    }
    fn pointer<const N: usize>(
        &mut self, dx: i32, dy: i32, wheel_y: i16, buttons: u32,
        p: &mut SpscProducer<'_, InputEvent, N>,
    ) -> Result<HidPublishStats, HidInputError> {
        let mut s = HidPublishStats::default();
        if dx != 0 || dy != 0 || wheel_y != 0 || self.previous_buttons != Some(buttons) {
            self.emit(InputEvent::PointerMotion { device: self.device, dx, dy,
                wheel_x: 0, wheel_y, buttons }, p, &mut s);
        }
        self.previous_buttons = Some(buttons);
        Ok(s)
    }
    fn emit<const N: usize>(
        &mut self, e: InputEvent, p: &mut SpscProducer<'_, InputEvent, N>, s: &mut HidPublishStats,
    ) {
        if p.push(e).is_ok() { s.published = s.published.saturating_add(1); }
        else {
            s.dropped = s.dropped.saturating_add(1);
            self.dropped_events = self.dropped_events.saturating_add(1);
            self.resync_required = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::queue::SpscQueue;
    #[test]
    fn keyboard_emits_key_and_modifier_transitions() {
        let mut q = SpscQueue::<InputEvent, 16>::new();
        let (mut p, mut c) = q.split();
        let mut a = UsbBootKeyboardAdapter::new(InputDeviceId(4));
        assert_eq!(a.publish_report(&[2,0,4,5,0,0,0,0], &mut p).unwrap().published, 3);
        assert!(matches!(c.pop(), Some(InputEvent::HidKey { usage: 4, pressed: true, .. })));
        assert!(matches!(c.pop(), Some(InputEvent::HidKey { usage: 5, pressed: true, .. })));
        assert!(matches!(c.pop(), Some(InputEvent::HidKey { usage: 0xE1, pressed: true, .. })));
    }
    #[test]
    fn mouse_preserves_signed_motion_wheel_and_button_bitmap() {
        let mut q = SpscQueue::<InputEvent, 8>::new();
        let (mut p, mut c) = q.split();
        let mut a = UsbBootMouseAdapter::new(InputDeviceId(8));
        a.publish_wheel_report(&[5,0xFE,3,0xFF], &mut p).unwrap();
        assert!(matches!(c.pop(), Some(InputEvent::PointerMotion {
            dx: -2, dy: 3, wheel_y: -1, buttons: 5, ..
        })));
    }
    #[test]
    fn boot_keyboard_resets_consumer_state_after_queue_overflow() {
        let mut q = SpscQueue::<InputEvent, 4>::new();
        let (mut p, mut c) = q.split();
        for id in 1..=3 {
            p.push(InputEvent::DeviceConnected { device: InputDeviceId(id) }).unwrap();
        }
        let mut a = UsbBootKeyboardAdapter::new(InputDeviceId(70));
        let first = a.publish_report(&[0,0,4,0,0,0,0,0], &mut p).unwrap();
        assert_eq!(first.dropped, 1);
        while c.pop().is_some() {}
        a.publish_report(&[0,0,4,0,0,0,0,0], &mut p).unwrap();
        assert!(matches!(c.pop(), Some(InputEvent::DeviceStateReset { device: InputDeviceId(70) })));
        assert!(matches!(c.pop(), Some(InputEvent::HidKey { usage: 4, pressed: true, .. })));
    }

    #[test]
    fn boot_mouse_resets_button_state_after_queue_overflow() {
        let mut q = SpscQueue::<InputEvent, 4>::new();
        let (mut p, mut c) = q.split();
        for id in 1..=3 {
            p.push(InputEvent::DeviceConnected { device: InputDeviceId(id) }).unwrap();
        }
        let mut a = UsbBootMouseAdapter::new(InputDeviceId(71));
        let first = a.publish_report(&[1,1,0], &mut p).unwrap();
        assert_eq!(first.dropped, 1);
        while c.pop().is_some() {}
        a.publish_report(&[1,1,0], &mut p).unwrap();
        assert!(matches!(c.pop(), Some(InputEvent::DeviceStateReset { device: InputDeviceId(71) })));
        assert!(matches!(c.pop(), Some(InputEvent::PointerMotion { buttons: 1, .. })));
    }

    #[test]
    fn malformed_keyboard_report_is_rejected() {
        let mut q = SpscQueue::<InputEvent, 8>::new();
        let (mut p, mut c) = q.split();
        let mut a = UsbBootKeyboardAdapter::new(InputDeviceId(1));
        assert_eq!(a.publish_report(&[0,0,4], &mut p), Err(HidInputError::MalformedReport));
        a.publish_report(&[0,0,4,0,0,0,0,0], &mut p).unwrap();
        assert!(matches!(c.pop(), Some(InputEvent::HidKey { usage: 4, pressed: true, .. })));
    }
}
