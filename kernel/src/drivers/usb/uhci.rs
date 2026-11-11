//! UHCI (USB 1.0/1.1) legacy host-controller register definitions.
pub const USBCMD:u16=0x00;pub const USBSTS:u16=0x02;pub const USBINTR:u16=0x04;
pub const FRNUM:u16=0x06;pub const FLBASEADD:u16=0x08;pub const SOFMOD:u16=0x0c;
pub const PORTSC1:u16=0x10;pub const PORTSC2:u16=0x12;
pub const CMD_RUN:u16=1<<0;pub const CMD_HCRESET:u16=1<<1;pub const CMD_GRESET:u16=1<<2;
pub const STS_USBINT:u16=1<<0;pub const STS_USBERRINT:u16=1<<1;pub const STS_RESUME:u16=1<<2;
pub const STS_HSE:u16=1<<3;pub const STS_HCPE:u16=1<<4;pub const STS_HCHALTED:u16=1<<5;
pub const PORT_CONNECT:u16=1<<0;pub const PORT_CONNECT_CHANGE:u16=1<<1;
pub const PORT_ENABLE:u16=1<<2;pub const PORT_ENABLE_CHANGE:u16=1<<3;
pub const PORT_LOW_SPEED:u16=1<<8;pub const PORT_RESET:u16=1<<9;

use super::UsbError;
pub const LINK_TERMINATE:u32=1;
pub const LINK_QUEUE_HEAD:u32=1<<1;
pub const LINK_DEPTH_FIRST:u32=0;
pub const TD_STATUS_ACTIVE:u32=1<<23;
pub const TD_STATUS_IOC:u32=1<<24;
pub const TD_STATUS_STALL:u32=1<<22;
pub const TD_STATUS_DATA_BUFFER_ERROR:u32=1<<21;
pub const TD_STATUS_BABBLE:u32=1<<20;
pub const TD_STATUS_NAK:u32=1<<19;
pub const TD_STATUS_CRC_TIMEOUT:u32=1<<18;
pub const TD_STATUS_BITSTUFF:u32=1<<17;

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
#[repr(C,align(16))]
pub struct UhciTransferDescriptor{pub link_pointer:u32,pub status:u32,pub token:u32,pub buffer_pointer:u32}
impl UhciTransferDescriptor{
 pub fn new(pid:u8,address:u8,endpoint:u8,data_toggle:bool,max_length:u16,buffer_physical:u64,interrupt_on_complete:bool)->Result<Self,UsbError>{
  if address>127||endpoint>15||max_length>2048||buffer_physical>u32::MAX as u64{return Err(UsbError::InvalidAddress)}
  let encoded_length=if max_length==0{0x7ff}else{max_length as u32-1};
  let token=pid as u32|((address as u32)<<8)|((endpoint as u32)<<15)|
   ((data_toggle as u32)<<19)|(encoded_length<<21);
  let status=TD_STATUS_ACTIVE|if interrupt_on_complete{TD_STATUS_IOC}else{0};
  Ok(Self{link_pointer:LINK_TERMINATE,status,token,buffer_pointer:buffer_physical as u32})
 }
 pub const fn active(self)->bool{self.status&TD_STATUS_ACTIVE!=0}
 pub const fn actual_length(self)->Option<u16>{
  if self.active(){None}else{let raw=(self.status&0x7ff)as u16;Some(if raw==0x7ff{0}else{raw+1})}
 }
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
#[repr(C,align(16))]
pub struct UhciQueueHead{pub horizontal_link:u32,pub element_link:u32}
impl UhciQueueHead{
 pub const fn empty()->Self{Self{horizontal_link:LINK_TERMINATE,element_link:LINK_TERMINATE}}
 pub fn set_horizontal(&mut self,physical:u64,queue_head:bool)->Result<(),UsbError>{
  self.horizontal_link=uhci_link(physical,queue_head)?;Ok(())
 }
 pub fn set_element(&mut self,physical:u64,queue_head:bool)->Result<(),UsbError>{
  self.element_link=uhci_link(physical,queue_head)?;Ok(())
 }
}
fn uhci_link(physical:u64,queue_head:bool)->Result<u32,UsbError>{
 if physical>u32::MAX as u64||physical&0xf!=0{return Err(UsbError::InvalidAddress)}
 Ok(physical as u32|if queue_head{LINK_QUEUE_HEAD}else{0})
}
#[derive(Clone,Copy)]
#[repr(C,align(4096))]
pub struct UhciFrameList{pub entries:[u32;1024]}
impl UhciFrameList{
 pub const fn new()->Self{Self{entries:[LINK_TERMINATE;1024]}}
 pub fn set(&mut self,frame:usize,physical:u64,queue_head:bool)->Result<(),UsbError>{
  if frame>=1024{return Err(UsbError::InvalidAddress)}
  self.entries[frame]=uhci_link(physical,queue_head)?;Ok(())
 }
 pub fn clear(&mut self,frame:usize)->Result<(),UsbError>{
  if frame>=1024{return Err(UsbError::InvalidAddress)}
  self.entries[frame]=LINK_TERMINATE;Ok(())
 }
}
impl Default for UhciFrameList{fn default()->Self{Self::new()}}

#[cfg(test)]
mod schedule_tests{
 use super::*;
 #[test]fn builds_uhci_td_token_and_status(){
  let td=UhciTransferDescriptor::new(0x2d,5,3,true,64,0x1000,true).unwrap();
  assert_eq!(td.token&0xff,0x2d);assert_eq!((td.token>>8)&0x7f,5);
  assert_eq!((td.token>>15)&0xf,3);assert_ne!(td.status&TD_STATUS_ACTIVE,0);
  assert_ne!(td.status&TD_STATUS_IOC,0);
 }
 #[test]fn validates_link_alignment_and_frame_index(){
  let mut frames=UhciFrameList::new();frames.set(1023,0x2000,true).unwrap();
  assert_eq!(frames.entries[1023],0x2002);
  assert_eq!(frames.set(1024,0x2000,false),Err(UsbError::InvalidAddress));
  assert_eq!(frames.set(0,0x2001,false),Err(UsbError::InvalidAddress));
 }
 #[test]fn zero_length_td_uses_uhci_length_encoding(){
  let td=UhciTransferDescriptor::new(0x69,0,0,false,0,0,false).unwrap();
  assert_eq!(td.token>>21,0x7ff);
 }
}
