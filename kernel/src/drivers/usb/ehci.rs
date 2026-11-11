//! EHCI (USB 2.0 high-speed) capability and operational register offsets.
pub const CAPLENGTH:usize=0x00;pub const HCIVERSION:usize=0x02;pub const HCSPARAMS:usize=0x04;
pub const HCCPARAMS:usize=0x08;pub const HCSP_PORTROUTE:usize=0x0c;
pub const USBCMD:usize=0x00;pub const USBSTS:usize=0x04;pub const USBINTR:usize=0x08;
pub const FRINDEX:usize=0x0c;pub const CTRLDSSEGMENT:usize=0x10;pub const PERIODICLISTBASE:usize=0x14;
pub const ASYNCLISTADDR:usize=0x18;pub const CONFIGFLAG:usize=0x40;pub const PORTSC_BASE:usize=0x44;
pub const CMD_RUN:u32=1;pub const CMD_RESET:u32=1<<1;pub const STS_HALTED:u32=1<<12;

use super::UsbError;
pub const LINK_TERMINATE:u32=1;
pub const LINK_TYPE_QH:u32=1<<1;
pub const QTD_STATUS_ACTIVE:u32=1<<7;
pub const QTD_STATUS_HALTED:u32=1<<6;
pub const QTD_STATUS_DATA_BUFFER_ERROR:u32=1<<5;
pub const QTD_STATUS_BABBLE:u32=1<<4;
pub const QTD_STATUS_TRANSACTION_ERROR:u32=1<<3;
pub const QTD_STATUS_MISSED_MICROFRAME:u32=1<<2;
pub const QTD_STATUS_SPLIT_STATE:u32=1<<1;
pub const QTD_STATUS_PING_STATE:u32=1;\npub const QTD_PID_OUT:u8=0;\npub const QTD_PID_IN:u8=1;\npub const QTD_PID_SETUP:u8=2;
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub enum EhciSpeed{Full,Low,High}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
#[repr(C,align(32))]
pub struct EhciQueueHead{
 pub horizontal_link:u32,pub endpoint_characteristics:u32,pub endpoint_capabilities:u32,
 pub current_qtd:u32,pub overlay_next_qtd:u32,pub overlay_alternate_qtd:u32,
 pub overlay_token:u32,pub overlay_buffer:[u32;5],
}
impl EhciQueueHead{
 pub fn new(address:u8,endpoint:u8,speed:EhciSpeed,max_packet:u16,data_toggle_control:bool,head:bool,hub_address:u8,port_number:u8,multiplier:u8)->Result<Self,UsbError>{
  let packet_valid=match speed{EhciSpeed::Low=>max_packet<=8,EhciSpeed::Full=>max_packet<=1023,EhciSpeed::High=>max_packet<=1024};\n  if address>127||endpoint>15||max_packet==0||!packet_valid||hub_address>127||port_number>127||!(1..=3).contains(&multiplier){return Err(UsbError::MalformedDescriptor)}
  if speed!=EhciSpeed::High&&multiplier!=1{return Err(UsbError::MalformedDescriptor)}
  let speed_bits=match speed{EhciSpeed::Full=>0,EhciSpeed::Low=>1,EhciSpeed::High=>2};
  let characteristics=address as u32|((endpoint as u32)<<8)|(speed_bits<<12)|
   ((data_toggle_control as u32)<<14)|((head as u32)<<15)|((max_packet as u32)<<16);
  let capabilities=((hub_address as u32)<<16)|((port_number as u32)<<23)|((multiplier as u32)<<30);
  Ok(Self{horizontal_link:LINK_TERMINATE,endpoint_characteristics:characteristics,
   endpoint_capabilities:capabilities,current_qtd:0,overlay_next_qtd:LINK_TERMINATE,
   overlay_alternate_qtd:LINK_TERMINATE,overlay_token:0,overlay_buffer:[0;5]})
 }
 pub fn set_periodic_masks(&mut self,start_mask:u8,complete_mask:u8){
  self.endpoint_capabilities=(self.endpoint_capabilities&!0xffff)|(start_mask as u32)|((complete_mask as u32)<<8);
 }
 pub fn set_horizontal_qh(&mut self,physical:u64)->Result<(),UsbError>{
  if physical>u32::MAX as u64||physical&0x1f!=0{return Err(UsbError::InvalidAddress)}
  self.horizontal_link=physical as u32|LINK_TYPE_QH;Ok(())
 }
 pub fn set_overlay(&mut self,next:u64,alternate:u64)->Result<(),UsbError>{
  self.overlay_next_qtd=ehci_qtd_link(next)?;self.overlay_alternate_qtd=ehci_qtd_link(alternate)?;Ok(())
 }
}
fn ehci_qtd_link(physical:u64)->Result<u32,UsbError>{
 if physical==0{return Ok(LINK_TERMINATE)}
 if physical>u32::MAX as u64||physical&0x1f!=0{return Err(UsbError::InvalidAddress)}
 Ok(physical as u32)
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
#[repr(C,align(32))]
pub struct EhciQueueTransferDescriptor{pub next_qtd:u32,pub alternate_qtd:u32,pub token:u32,pub buffer:[u32;5]}
impl EhciQueueTransferDescriptor{
 pub fn new(pid:u8,bytes:u16,data_toggle:bool,buffer_pages:[u64;5],next:u64,alternate:u64,interrupt:bool)->Result<Self,UsbError>{
  if pid>2||bytes>0x7fff{return Err(UsbError::MalformedDescriptor)}
  let mut buffer=[0u32;5];
  for (i,address) in buffer_pages.iter().enumerate(){
   if *address>u32::MAX as u64||(i!=0&&*address&0xfff!=0){return Err(UsbError::InvalidAddress)}
   buffer[i]=*address as u32;
  }
  let token=QTD_STATUS_ACTIVE|((pid as u32)<<8)|(3<<10)|((interrupt as u32)<<15)|
   ((bytes as u32)<<16)|((data_toggle as u32)<<31);
  Ok(Self{next_qtd:ehci_qtd_link(next)?,alternate_qtd:ehci_qtd_link(alternate)?,token,buffer})
 }
}
#[derive(Clone,Copy)]
#[repr(C,align(4096))]
pub struct EhciPeriodicFrameList{pub entries:[u32;1024]}
impl EhciPeriodicFrameList{
 pub const fn new()->Self{Self{entries:[LINK_TERMINATE;1024]}}
 pub fn set_queue_head(&mut self,frame:usize,physical:u64)->Result<(),UsbError>{
  if frame>=1024||physical>u32::MAX as u64||physical&0x1f!=0{return Err(UsbError::InvalidAddress)}
  self.entries[frame]=physical as u32|LINK_TYPE_QH;Ok(())
 }
 pub fn clear(&mut self,frame:usize)->Result<(),UsbError>{
  if frame>=1024{return Err(UsbError::InvalidAddress)}
  self.entries[frame]=LINK_TERMINATE;Ok(())
 }
}
impl Default for EhciPeriodicFrameList{fn default()->Self{Self::new()}}

#[cfg(test)]
mod schedule_tests{
 use super::*;
 #[test]fn builds_ehci_queue_head_and_qtd(){
  let qh=EhciQueueHead::new(5,2,EhciSpeed::High,512,true,true,0,0,1).unwrap();
  assert_eq!(qh.endpoint_characteristics&0x7f,5);
  assert_eq!((qh.endpoint_characteristics>>16)&0x7ff,512);
  let qtd=EhciQueueTransferDescriptor::new(1,512,false,[0x1000,0x2000,0,0,0],0,0,true).unwrap();
  assert_ne!(qtd.token&QTD_STATUS_ACTIVE,0);assert_ne!(qtd.token&(1<<15),0);
 }
 #[test]fn validates_ehci_schedule_alignment_and_periodic_frames(){
  let mut list=EhciPeriodicFrameList::new();list.set_queue_head(1023,0x4000).unwrap();
  assert_eq!(list.entries[1023],0x4002);
  assert_eq!(list.set_queue_head(0,0x4001),Err(UsbError::InvalidAddress));
 }
 #[test]fn qh_and_qtd_have_hardware_required_sizes(){
  assert_eq!(core::mem::size_of::<EhciQueueHead>(),64);
  assert_eq!(core::mem::align_of::<EhciQueueHead>(),32);
  assert_eq!(core::mem::size_of::<EhciQueueTransferDescriptor>(),32);
 }
}
