//! PS/2 scan-code Set 1 decoder, including E0 extended keys.
use crate::drivers::input::event::{InputEvent, KeyCode};
#[derive(Clone, Copy, Debug, Default)]
pub struct Set1Decoder { extended: bool, pause_remaining: u8, keys_down: [u64; 4] }
impl Set1Decoder {
 pub const fn new() -> Self { Self { extended: false, pause_remaining: 0, keys_down: [0; 4] } }
 pub fn feed(&mut self, byte: u8) -> Option<InputEvent> {
  if self.pause_remaining != 0 { self.pause_remaining -= 1; return None; }
  if byte == 0xE0 { self.extended = true; return None; }
  if byte == 0xE1 { self.extended = false; self.pause_remaining = 5; return None; }
  let extended = core::mem::replace(&mut self.extended, false);
  let pressed = byte & 0x80 == 0; let scan = (byte & 0x7F) as u16;
  let code = map_scancode(scan, extended); let index = scan as usize + if extended { 128 } else { 0 };
  let mask = 1u64 << (index % 64); let slot = &mut self.keys_down[index / 64];
  let was_down = *slot & mask != 0;
  if pressed { *slot |= mask; } else { *slot &= !mask; }
  Some(InputEvent::Key { code, pressed, repeat: pressed && was_down })
 }
}
fn map_scancode(s: u16, e: bool) -> KeyCode {
 if e { return match s {
  0x1C => KeyCode::Keypad0, 0x1D => KeyCode::RightCtrl, 0x35 => KeyCode::Slash, 0x38 => KeyCode::RightAlt,
  0x47 => KeyCode::Home, 0x48 => KeyCode::Up, 0x49 => KeyCode::PageUp, 0x4B => KeyCode::Left,
  0x4D => KeyCode::Right, 0x4F => KeyCode::End, 0x50 => KeyCode::Down, 0x51 => KeyCode::PageDown,
  0x52 => KeyCode::Insert, 0x53 => KeyCode::Delete, _ => KeyCode::Unknown(0x100 | s),
 }; }
 match s {
  0x01=>KeyCode::Escape,0x02=>KeyCode::Num1,0x03=>KeyCode::Num2,0x04=>KeyCode::Num3,0x05=>KeyCode::Num4,
  0x06=>KeyCode::Num5,0x07=>KeyCode::Num6,0x08=>KeyCode::Num7,0x09=>KeyCode::Num8,0x0A=>KeyCode::Num9,
  0x0B=>KeyCode::Num0,0x0C=>KeyCode::Minus,0x0D=>KeyCode::Equal,0x0E=>KeyCode::Backspace,0x0F=>KeyCode::Tab,
  0x10=>KeyCode::Q,0x11=>KeyCode::W,0x12=>KeyCode::E,0x13=>KeyCode::R,0x14=>KeyCode::T,0x15=>KeyCode::Y,
  0x16=>KeyCode::U,0x17=>KeyCode::I,0x18=>KeyCode::O,0x19=>KeyCode::P,0x1A=>KeyCode::LeftBracket,
  0x1B=>KeyCode::RightBracket,0x1C=>KeyCode::Enter,0x1D=>KeyCode::LeftCtrl,0x1E=>KeyCode::A,0x1F=>KeyCode::S,
  0x20=>KeyCode::D,0x21=>KeyCode::F,0x22=>KeyCode::G,0x23=>KeyCode::H,0x24=>KeyCode::J,0x25=>KeyCode::K,
  0x26=>KeyCode::L,0x27=>KeyCode::Semicolon,0x28=>KeyCode::Apostrophe,0x29=>KeyCode::Grave,0x2A=>KeyCode::LeftShift,
  0x2B=>KeyCode::Backslash,0x2C=>KeyCode::Z,0x2D=>KeyCode::X,0x2E=>KeyCode::C,0x2F=>KeyCode::V,
  0x30=>KeyCode::B,0x31=>KeyCode::N,0x32=>KeyCode::M,0x33=>KeyCode::Comma,0x34=>KeyCode::Period,
  0x35=>KeyCode::Slash,0x36=>KeyCode::RightShift,0x37=>KeyCode::KeypadAsterisk,0x38=>KeyCode::LeftAlt,
  0x39=>KeyCode::Space,0x3A=>KeyCode::CapsLock,0x3B=>KeyCode::F1,0x3C=>KeyCode::F2,0x3D=>KeyCode::F3,
  0x3E=>KeyCode::F4,0x3F=>KeyCode::F5,0x40=>KeyCode::F6,0x41=>KeyCode::F7,0x42=>KeyCode::F8,
  0x43=>KeyCode::F9,0x44=>KeyCode::F10,0x45=>KeyCode::NumLock,0x46=>KeyCode::ScrollLock,
  0x47=>KeyCode::Keypad7,0x48=>KeyCode::Keypad8,0x49=>KeyCode::Keypad9,0x4A=>KeyCode::KeypadMinus,
  0x4B=>KeyCode::Keypad4,0x4C=>KeyCode::Keypad5,0x4D=>KeyCode::Keypad6,0x4E=>KeyCode::KeypadPlus,
  0x4F=>KeyCode::Keypad1,0x50=>KeyCode::Keypad2,0x51=>KeyCode::Keypad3,0x52=>KeyCode::Keypad0,
  0x53=>KeyCode::KeypadPeriod,0x57=>KeyCode::F11,0x58=>KeyCode::F12,_=>KeyCode::Unknown(s),
 }
}
#[cfg(test)]
mod tests {
 use super::*;
 #[test] fn press_repeat_release() {
  let mut d=Set1Decoder::new();
  assert_eq!(d.feed(0x1E),Some(InputEvent::Key{code:KeyCode::A,pressed:true,repeat:false}));
  assert_eq!(d.feed(0x1E),Some(InputEvent::Key{code:KeyCode::A,pressed:true,repeat:true}));
  assert_eq!(d.feed(0x9E),Some(InputEvent::Key{code:KeyCode::A,pressed:false,repeat:false}));
 }
 #[test] fn extended_navigation() {
  let mut d=Set1Decoder::new(); assert_eq!(d.feed(0xE0),None);
  assert_eq!(d.feed(0x48),Some(InputEvent::Key{code:KeyCode::Up,pressed:true,repeat:false}));
 }
}
