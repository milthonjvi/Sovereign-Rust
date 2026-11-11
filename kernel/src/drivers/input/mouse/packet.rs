//! PS/2 mouse packet decoder for three-byte and wheel four-byte packets.
use crate::drivers::input::event::{InputEvent, MouseButtons};
use crate::drivers::input::ps2::Ps2MouseProtocol;
#[derive(Clone, Copy, Debug)]
pub struct Ps2MouseDecoder {
 packet: [u8;4], index: u8, packet_len: u8,
 protocol: Ps2MouseProtocol, previous_buttons: Option<MouseButtons>,
}
impl Ps2MouseDecoder {
 /// Compatibility constructor for standard or IntelliMouse wheel packets.
 pub const fn new(wheel_enabled: bool) -> Self {
  Self::new_with_protocol(if wheel_enabled { Ps2MouseProtocol::Wheel } else { Ps2MouseProtocol::Standard })
 }
 /// Selects packet semantics from the device ID negotiated during PS/2 setup.
 pub const fn new_with_protocol(protocol: Ps2MouseProtocol) -> Self {
  let packet_len = match protocol {
   Ps2MouseProtocol::Standard => 3,
   Ps2MouseProtocol::Wheel | Ps2MouseProtocol::FiveButton => 4,
  };
  Self { packet:[0;4], index:0, packet_len, protocol, previous_buttons:None }
 }
 /// Discards an incomplete packet and forces the next packet to resynchronize.
 pub fn reset_stream(&mut self) {
  self.index = 0;
  self.previous_buttons = None;
 }
 /// Feed one byte. A complete packet can produce motion, button, and wheel events.
 pub fn feed(&mut self, byte:u8, out:&mut [Option<InputEvent>;3])->usize {
  out.fill(None);
  if self.index==0 && byte&0x08==0 { return 0; }
  self.packet[self.index as usize]=byte; self.index+=1;
  if self.index<self.packet_len { return 0; }
  self.index=0; let flags=self.packet[0];
  // Overflow means the corresponding signed delta is not representable; do not
  // synthesize a large movement from the saturated data byte.
  let dx=if flags&0x40!=0 {0} else {axis(self.packet[1],flags&0x10!=0)};
  let dy=if flags&0x80!=0 {0} else {-axis(self.packet[2],flags&0x20!=0)};
  // Bits 4-5 of the fourth byte are button 4/5 only for the Explorer
  // protocol. They are reserved in ordinary IntelliMouse wheel packets.
  let extra=if self.protocol==Ps2MouseProtocol::FiveButton {(self.packet[3]&0x30)>>1} else {0};
  let buttons=MouseButtons::from_bits((flags&7)|extra); let mut count=0;
  if dx!=0 || dy!=0 { out[count]=Some(InputEvent::MouseMotion{dx,dy}); count+=1; }
  if self.previous_buttons!=Some(buttons) {
   out[count]=Some(InputEvent::MouseButtons{buttons}); count+=1; self.previous_buttons=Some(buttons);
  }
  if self.packet_len==4 {
   let wheel=(self.packet[3] as i8)<<4>>4;
   if wheel!=0 { out[count]=Some(InputEvent::MouseWheel{delta:wheel}); count+=1; }
  }
  count
 }
}
fn axis(raw:u8,negative:bool)->i16 { if negative {(raw as i16)-256} else {raw as i16} }
#[cfg(test)]
mod tests {
 use super::*;
 #[test] fn motion_and_button() {
  let mut d=Ps2MouseDecoder::new(false); let mut out=[None;3];
  assert_eq!(d.feed(9,&mut out),0); assert_eq!(d.feed(5,&mut out),0);
  assert_eq!(d.feed(0,&mut out),2);
  assert_eq!(out[0],Some(InputEvent::MouseMotion{dx:5,dy:0}));
  assert_eq!(out[1],Some(InputEvent::MouseButtons{buttons:MouseButtons::from_bits(1)}));
 }
 #[test] fn signed_axes_and_wheel() {
  let mut d=Ps2MouseDecoder::new(true); let mut out=[None;3];
  d.feed(0x18,&mut out); d.feed(0xFF,&mut out);
  assert_eq!(d.feed(0xFF,&mut out),0);
  assert_eq!(d.feed(0x0F,&mut out),3);
  assert_eq!(out[0],Some(InputEvent::MouseMotion{dx:-1,dy:1}));
  assert_eq!(out[2],Some(InputEvent::MouseWheel{delta:-1}));
 }

 #[test] fn standard_wheel_protocol_ignores_reserved_button_bits() {
  let mut d=Ps2MouseDecoder::new_with_protocol(Ps2MouseProtocol::Wheel);
  let mut out=[None;3];
  d.feed(0x08,&mut out); d.feed(0,&mut out); d.feed(0,&mut out);
  assert_eq!(d.feed(0x30,&mut out),0);
 }
 #[test] fn five_button_protocol_decodes_extra_buttons_and_signed_wheel() {
  let mut d=Ps2MouseDecoder::new_with_protocol(Ps2MouseProtocol::FiveButton);
  let mut out=[None;3];
  d.feed(0x08,&mut out); d.feed(0,&mut out); d.feed(0,&mut out);
  assert_eq!(d.feed(0x3F,&mut out),2);
  assert_eq!(out[0],Some(InputEvent::MouseButtons{buttons:MouseButtons::from_bits(0x18)}));
  assert_eq!(out[1],Some(InputEvent::MouseWheel{delta:-1}));
 }
 #[test] fn reset_discards_partial_packet() {
  let mut d=Ps2MouseDecoder::new(false); let mut out=[None;3];
  assert_eq!(d.feed(0x09,&mut out),0);
  d.reset_stream();
  assert_eq!(d.feed(0x04,&mut out),0); // Not a valid packet header.
  assert_eq!(d.feed(0x08,&mut out),0);
  assert_eq!(d.feed(0x02,&mut out),0);
  assert_eq!(d.feed(0x00,&mut out),2);
 }
 #[test] fn overflow_flags_suppress_invalid_axis_deltas() {
  let mut d=Ps2MouseDecoder::new(false); let mut out=[None;3];
  d.feed(0x48,&mut out); // X overflow, valid packet header.
  d.feed(0x7F,&mut out);
  assert_eq!(d.feed(0,&mut out),1);
  assert_eq!(out[0],Some(InputEvent::MouseButtons{buttons:MouseButtons::from_bits(0)}));
 }
}
