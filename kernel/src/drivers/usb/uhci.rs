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
#[repr(u8)]
pub enum UhciPid{Out=0xe1,In=0x69,Setup=0x2d}

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
#[repr(C,align(16))]
pub struct UhciTransferDescriptor{pub link_pointer:u32,pub status:u32,pub token:u32,pub buffer_pointer:u32}
impl UhciTransferDescriptor{
 pub fn new(pid:UhciPid,address:u8,endpoint:u8,data_toggle:bool,max_length:u16,buffer_physical:u64,interrupt_on_complete:bool)->Result<Self,UsbError>{
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
  let td=UhciTransferDescriptor::new(UhciPid::Setup,5,3,true,64,0x1000,true).unwrap();
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
  let td=UhciTransferDescriptor::new(UhciPid::In,0,0,false,0,0,false).unwrap();
  assert_eq!(td.token>>21,0x7ff);
 }
}


pub const CMD_MAX_PACKET:u16=1<<7;
pub const PORT_SUSPEND:u16=1<<12;
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub enum UhciState{Discovered,Resetting,Reset,Running,Halted}
pub trait UhciPortIo{fn read16(&self,port:u16)->Result<u16,super::UsbError>;fn write16(&mut self,port:u16,value:u16)->Result<(),super::UsbError>;fn write32(&mut self,port:u16,value:u32)->Result<(),super::UsbError>;}
pub struct UhciController{pub state:UhciState,io_base:u16,frame_list_configured:bool}
impl UhciController{
 pub const fn new(io_base:u16)->Self{Self{state:UhciState::Discovered,io_base,frame_list_configured:false}}
 fn reg(&self,offset:u16)->u16{self.io_base+offset}
 pub fn request_reset<I:UhciPortIo>(&mut self,io:&mut I)->Result<(),super::UsbError>{
  if self.state!=UhciState::Discovered&&self.state!=UhciState::Halted{return Err(super::UsbError::Unsupported)}
  io.write16(self.reg(USBCMD),CMD_HCRESET)?;self.state=UhciState::Resetting;Ok(())
 }
 pub fn poll_reset<I:UhciPortIo>(&mut self,io:&I)->Result<bool,super::UsbError>{
  if self.state!=UhciState::Resetting{return Err(super::UsbError::Unsupported)}
  if io.read16(self.reg(USBCMD))?&CMD_HCRESET==0{self.state=UhciState::Reset;return Ok(true)}
  Ok(false)
 }
 /// Frame list must be pinned, coherent DMA memory aligned to 4 KiB.
 pub unsafe fn configure_frame_list<I:UhciPortIo>(&mut self,io:&mut I,physical:u64)->Result<(),super::UsbError>{
  if self.state!=UhciState::Reset||self.frame_list_configured{return Err(super::UsbError::Unsupported)}
  if physical>u32::MAX as u64||physical&0xfff!=0{return Err(super::UsbError::InvalidAddress)}
  io.write32(self.reg(FLBASEADD),physical as u32)?;
  io.write16(self.reg(SOFMOD),0x40)?;
  self.frame_list_configured=true;Ok(())
 }
 pub fn start<I:UhciPortIo>(&mut self,io:&mut I)->Result<(),super::UsbError>{
  if self.state!=UhciState::Reset||!self.frame_list_configured{return Err(super::UsbError::Unsupported)}
  io.write16(self.reg(USBINTR),STS_USBINT|STS_USBERRINT|STS_RESUME|STS_HSE)?;
  io.write16(self.reg(USBCMD),CMD_RUN|CMD_MAX_PACKET)?;Ok(())
 }
 pub fn poll_running<I:UhciPortIo>(&mut self,io:&I)->Result<bool,super::UsbError>{
  if self.state!=UhciState::Reset{return Err(super::UsbError::Unsupported)}
  if io.read16(self.reg(USBSTS))?&STS_HCHALTED==0{self.state=UhciState::Running;return Ok(true)}
  Ok(false)
 }
 pub fn request_halt<I:UhciPortIo>(&mut self,io:&mut I)->Result<(),super::UsbError>{
  if self.state!=UhciState::Running{return Err(super::UsbError::Unsupported)}
  let command=io.read16(self.reg(USBCMD))?;io.write16(self.reg(USBCMD),command&!CMD_RUN)
 }
 pub fn poll_halted<I:UhciPortIo>(&mut self,io:&I)->Result<bool,super::UsbError>{
  if self.state!=UhciState::Running{return Err(super::UsbError::Unsupported)}
  if io.read16(self.reg(USBSTS))?&STS_HCHALTED!=0{self.state=UhciState::Halted;return Ok(true)}
  Ok(false)
 }
 pub fn port_status<I:UhciPortIo>(&self,io:&I,port:u8)->Result<super::controller::PortStatus,super::UsbError>{
  if self.state!=UhciState::Running{return Err(super::UsbError::ControllerUnavailable)}
  let offset=match port{1=>PORTSC1,2=>PORTSC2,_=>return Err(super::UsbError::InvalidAddress)};
  let value=io.read16(self.reg(offset))?;
  Ok(super::controller::PortStatus{connected:value&PORT_CONNECT!=0,enabled:value&PORT_ENABLE!=0,
   suspended:value&PORT_SUSPEND!=0,over_current:false,
   speed:if value&PORT_CONNECT==0{None}else if value&PORT_LOW_SPEED!=0{Some(super::transfer::UsbSpeed::Low)}else{Some(super::transfer::UsbSpeed::Full)}})
 }
 pub fn request_port_reset<I:UhciPortIo>(&self,io:&mut I,port:u8)->Result<(),super::UsbError>{
  if self.state!=UhciState::Running{return Err(super::UsbError::ControllerUnavailable)}
  let offset=match port{1=>PORTSC1,2=>PORTSC2,_=>return Err(super::UsbError::InvalidAddress)};
  let value=io.read16(self.reg(offset))?;
  if value&PORT_CONNECT==0{return Err(super::UsbError::ControllerUnavailable)}
  io.write16(self.reg(offset),(value&!(PORT_CONNECT_CHANGE|PORT_ENABLE_CHANGE))|PORT_RESET)
 }
 pub fn port_reset_complete<I:UhciPortIo>(&self,io:&I,port:u8)->Result<bool,super::UsbError>{
  if self.state!=UhciState::Running{return Err(super::UsbError::ControllerUnavailable)}
  let offset=match port{1=>PORTSC1,2=>PORTSC2,_=>return Err(super::UsbError::InvalidAddress)};
  let value=io.read16(self.reg(offset))?;Ok(value&PORT_RESET==0&&value&PORT_ENABLE!=0)
 }
 pub fn clear_port_changes<I:UhciPortIo>(&self,io:&mut I,port:u8,changes:u16)->Result<(),super::UsbError>{
  if self.state!=UhciState::Running{return Err(super::UsbError::ControllerUnavailable)}
  let offset=match port{1=>PORTSC1,2=>PORTSC2,_=>return Err(super::UsbError::InvalidAddress)};
  if changes&!(PORT_CONNECT_CHANGE|PORT_ENABLE_CHANGE)!=0{return Err(super::UsbError::InvalidAddress)}
  let current=io.read16(self.reg(offset))?;
  let preserve=current&(PORT_ENABLE|PORT_RESET|PORT_SUSPEND);
  io.write16(self.reg(offset),preserve|changes)
 }
}

#[cfg(test)]
mod controller_tests{
 use super::*;
 struct FakeUhci{ports:[u16;64]}
 impl FakeUhci{fn new()->Self{Self{ports:[0;64]}}}
 impl UhciPortIo for FakeUhci{
  fn read16(&self,port:u16)->Result<u16,super::super::UsbError>{self.ports.get(port as usize/2).copied().ok_or(super::super::UsbError::BufferTooShort)}
  fn write16(&mut self,port:u16,value:u16)->Result<(),super::super::UsbError>{
   let i=port as usize/2;if i>=self.ports.len(){return Err(super::super::UsbError::BufferTooShort)}
   self.ports[i]=value;
   if port==USBCMD&&value&CMD_HCRESET!=0{self.ports[i]&=!CMD_HCRESET;}
   if port==USBCMD{if value&CMD_RUN!=0{self.ports[(port as usize+2)/2]&=!STS_HCHALTED}else{self.ports[(port as usize+2)/2]|=STS_HCHALTED}}
   Ok(())
  }
  fn write32(&mut self,port:u16,value:u32)->Result<(),super::super::UsbError>{self.write16(port,value as u16)}
 }
 #[test]fn resets_configures_frame_list_and_runs(){
  let mut io=FakeUhci::new();let mut h=UhciController::new(0);
  h.request_reset(&mut io).unwrap();assert!(h.poll_reset(&io).unwrap());
  unsafe{h.configure_frame_list(&mut io,0x1000)}.unwrap();
  h.start(&mut io).unwrap();assert!(h.poll_running(&io).unwrap());assert_eq!(h.state,UhciState::Running);
 }
 #[test]fn rejects_unaligned_frame_list(){
  let mut io=FakeUhci::new();let mut h=UhciController::new(0);h.state=UhciState::Reset;
  assert_eq!(unsafe{h.configure_frame_list(&mut io,0x1001)},Err(super::super::UsbError::InvalidAddress));
 }
}
