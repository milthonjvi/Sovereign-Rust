//! PS/2 mouse packet decoder for three-byte and wheel four-byte packets.
use crate::drivers::input::event::{InputEvent, MouseButtons};
#[derive(Clone, Copy, Debug)]
pub struct Ps2MouseDecoder { packet: [u8;4], index: u8, packet_len: u8, previous_buttons: Option<MouseButtons> }
impl Ps2MouseDecoder {
 pub const fn new(wheel_enabled: bool) -> Self {
  Self { packet:[0;4], index:0, packet_len:if wheel_enabled {4} else {3}, previous_buttons:None }
 }
 /// Feed one byte. A complete packet can produce motion, button, and wheel events.
 pub fn feed(&mut self, byte:u8, out:&mut [Option<InputEvent>;3])->usize {
  out.fill(None);
  if self.index==0 && byte&0x08==0 { return 0; }
  self.packet[self.index as usize]=byte; self.index+=1;
  if self.index<self.packet_len { return 0; }
  self.index=0; let flags=self.packet[0];
  let dx=axis(self.packet[1],flags&0x10!=0); let dy=-axis(self.packet[2],flags&0x20!=0);
  let extra=if self.packet_len==4 {(self.packet[3]&0x30)>>1} else {0};
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
  assert_eq!(d.feed(0xFF,&mut out),2);
  assert_eq!(out[0],Some(InputEvent::MouseMotion{dx:-1,dy:1}));
  assert_eq!(d.feed(0x0F,&mut out),3);
  assert_eq!(out[2],Some(InputEvent::MouseWheel{delta:-1}));
 }
}
