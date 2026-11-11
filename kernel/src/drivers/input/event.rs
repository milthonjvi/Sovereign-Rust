//! Normalized events emitted by input devices.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyCode {
 Escape, Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9, Num0,
 Minus, Equal, Backspace, Tab, Q, W, E, R, T, Y, U, I, O, P,
 LeftBracket, RightBracket, Enter, LeftCtrl, A, S, D, F, G, H, J, K, L,
 Semicolon, Apostrophe, Grave, LeftShift, Backslash, Z, X, C, V, B, N, M,
 Comma, Period, Slash, RightShift, KeypadAsterisk, LeftAlt, Space, CapsLock,
 F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, NumLock, ScrollLock,
 Keypad7, Keypad8, Keypad9, KeypadMinus, Keypad4, Keypad5, Keypad6,
 KeypadPlus, Keypad1, Keypad2, Keypad3, Keypad0, KeypadPeriod, KeypadEnter, F11, F12,
 RightCtrl, RightAlt, Home, Up, PageUp, Left, Right, End, Down, PageDown,
 Insert, Delete, Unknown(u16),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputEvent {
 Key { code: KeyCode, pressed: bool, repeat: bool },
 MouseMotion { dx: i16, dy: i16 },
 MouseButtons { buttons: MouseButtons },
 MouseWheel { delta: i8 },
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MouseButtons { pub left: bool, pub right: bool, pub middle: bool, pub extra1: bool, pub extra2: bool }
impl MouseButtons {
 pub const fn from_bits(bits: u8) -> Self {
  Self { left: bits & 1 != 0, right: bits & 2 != 0, middle: bits & 4 != 0, extra1: bits & 8 != 0, extra2: bits & 16 != 0 }
 }
}
