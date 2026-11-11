//! Transport-independent, allocation-free input events.
//!
//! PS/2 decoders and USB HID adapters publish into the same event queue. Bus
//! drivers own enumeration and report transport; this module owns only the
//! normalized vocabulary consumed by the input stack/window system.

/// Stable identifier assigned by the input-device manager to one connected
/// physical or logical input collection. Zero is reserved for legacy events
/// that predate source tagging.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[repr(transparent)]
pub struct InputDeviceId(pub u16);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyCode {
    Escape, Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9, Num0,
    Minus, Equal, Backspace, Tab, Q, W, E, R, T, Y, U, I, O, P,
    LeftBracket, RightBracket, Enter, LeftCtrl, A, S, D, F, G, H, J, K, L,
    Semicolon, Apostrophe, Grave, LeftShift, Backslash, Z, X, C, V, B, N, M,
    Comma, Period, Slash, RightShift, KeypadAsterisk, LeftAlt, Space, CapsLock,
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, NumLock, ScrollLock,
    Keypad7, Keypad8, Keypad9, KeypadMinus, Keypad4, Keypad5, Keypad6,
    KeypadPlus, Keypad1, Keypad2, Keypad3, Keypad0, KeypadPeriod, KeypadEnter,
    F11, F12, RightCtrl, RightAlt, Home, Up, PageUp, Left, Right, End, Down,
    PageDown, Insert, Delete, Unknown(u16),
}

/// Contact lifecycle for touchscreens and multitouch digitizers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContactPhase { Down, Move, Up, Cancel }

/// Common game-controller axes. Other(u8) preserves vendor-defined axes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GamepadAxis {
    LeftX, LeftY, RightX, RightY, LeftTrigger, RightTrigger, Other(u8),
}

/// HID usages remain semantic-neutral: keymaps and policy layers translate
/// usage page + usage into characters or actions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputEvent {
    /// Legacy keyboard event emitted by the PS/2 Set-1 decoder.
    Key { code: KeyCode, pressed: bool, repeat: bool },
    /// Legacy relative mouse event emitted by the PS/2 packet decoder.
    MouseMotion { dx: i16, dy: i16 },
    MouseButtons { buttons: MouseButtons },
    MouseWheel { delta: i8 },

    /// HID keyboard usage transition. Modifier bits are HID usages E0-E7.
    HidKey {
        device: InputDeviceId, usage_page: u16, usage: u16,
        pressed: bool, modifiers: u8,
    },
    /// Relative pointer report with horizontal/vertical wheel and button bitmap.
    PointerMotion {
        device: InputDeviceId, dx: i32, dy: i32,
        wheel_x: i16, wheel_y: i16, buttons: u32,
    },
    /// Absolute touch contact; coordinates and pressure are normalized 0..=65535.
    TouchContact {
        device: InputDeviceId, contact: u16, phase: ContactPhase,
        x: u16, y: u16, pressure: u16,
    },
    /// Absolute pen/tablet sample; tilt is signed and pressure is normalized.
    Tablet {
        device: InputDeviceId, x: u16, y: u16, pressure: u16,
        tilt_x: i16, tilt_y: i16, buttons: u32, in_range: bool, tip: bool,
    },
    GamepadButton { device: InputDeviceId, button: u16, pressed: bool },
    GamepadAxis { device: InputDeviceId, axis: GamepadAxis, value: i16 },
    /// Consumer-page controls such as volume, mute, play/pause and media keys.
    ConsumerControl {
        device: InputDeviceId, usage_page: u16, usage: u16, pressed: bool,
    },
    /// Device remains connected, but consumers must clear all held keys,
    /// buttons and contacts before applying the next complete report snapshot.
    DeviceStateReset { device: InputDeviceId },
    /// Lifecycle events let consumers discard per-device state after unplug.
    DeviceConnected { device: InputDeviceId },
    DeviceDisconnected { device: InputDeviceId },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MouseButtons {
    pub left: bool, pub right: bool, pub middle: bool, pub extra1: bool, pub extra2: bool,
}
impl MouseButtons {
    pub const fn from_bits(bits: u8) -> Self {
        Self {
            left: bits & 1 != 0, right: bits & 2 != 0, middle: bits & 4 != 0,
            extra1: bits & 8 != 0, extra2: bits & 16 != 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_hid_events_retain_device_identity_and_usage() {
        let event = InputEvent::HidKey {
            device: InputDeviceId(7), usage_page: 0x07, usage: 0x04,
            pressed: true, modifiers: 0x02,
        };
        assert_eq!(event, InputEvent::HidKey {
            device: InputDeviceId(7), usage_page: 0x07, usage: 0x04,
            pressed: true, modifiers: 0x02,
        });
    }

    #[test]
    fn touch_and_gamepad_events_preserve_normalized_values() {
        let touch = InputEvent::TouchContact {
            device: InputDeviceId(2), contact: 3, phase: ContactPhase::Move,
            x: 32_768, y: 65_535, pressure: 1_024,
        };
        assert!(matches!(touch, InputEvent::TouchContact { x: 32_768, .. }));
        let axis = InputEvent::GamepadAxis {
            device: InputDeviceId(2), axis: GamepadAxis::LeftX, value: i16::MIN,
        };
        assert!(matches!(axis, InputEvent::GamepadAxis { value: i16::MIN, .. }));
    }
}
