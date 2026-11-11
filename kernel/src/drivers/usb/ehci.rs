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
pub const QTD_STATUS_PING_STATE:u32=1;
pub const QTD_PID_OUT:u8=0;
pub const QTD_PID_IN:u8=1;
pub const QTD_PID_SETUP:u8=2;
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
  let packet_valid=match speed{EhciSpeed::Low=>max_packet<=8,EhciSpeed::Full=>max_packet<=1023,EhciSpeed::High=>max_packet<=1024};
  if address>127||endpoint>15||max_packet==0||!packet_valid||hub_address>127||port_number>127||!(1..=3).contains(&multiplier){return Err(UsbError::MalformedDescriptor)}
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


#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub enum EhciState{Discovered,Halted,Resetting,Reset,Running,Failed}
pub trait EhciRegisterIo{fn read32(&self,offset:usize)->Result<u32,UsbError>;fn write32(&mut self,offset:usize,value:u32)->Result<(),UsbError>;}
pub struct EhciController{pub state:EhciState,pub version:u16,pub port_count:u8,pub address_64bit:bool,operational_base:usize,schedule_configured:bool,segment:u32}
impl EhciController{
 pub fn discover<I:EhciRegisterIo>(io:&I)->Result<Self,UsbError>{
  let cap=io.read32(0)?;let length=(cap&0xff)as usize;let version=(cap>>16)as u16;
  if length<0x10||version<0x0100{return Err(UsbError::MalformedDescriptor)}
  let structural=io.read32(4)?;let ports=(structural&0x0f)as u8;
  if ports==0{return Err(UsbError::MalformedDescriptor)}
  let capability=io.read32(8)?;
  Ok(Self{state:EhciState::Discovered,version,port_count:ports,address_64bit:capability&1!=0,operational_base:length,schedule_configured:false,segment:0})
 }
 fn op(&self,offset:usize)->usize{self.operational_base+offset}
 pub fn request_halt<I:EhciRegisterIo>(&mut self,io:&mut I)->Result<(),UsbError>{
  if self.state!=EhciState::Discovered&&self.state!=EhciState::Running{return Err(UsbError::Unsupported)}
  let command=io.read32(self.op(USBCMD))?;
  io.write32(self.op(USBCMD),command&!CMD_RUN)
 }
 pub fn poll_halted<I:EhciRegisterIo>(&mut self,io:&I)->Result<bool,UsbError>{
  if self.state!=EhciState::Discovered&&self.state!=EhciState::Running{return Err(UsbError::Unsupported)}
  if io.read32(self.op(USBSTS))?&STS_HALTED!=0{self.state=EhciState::Halted;return Ok(true)}
  Ok(false)
 }
 pub fn request_reset<I:EhciRegisterIo>(&mut self,io:&mut I)->Result<(),UsbError>{
  if self.state!=EhciState::Halted{return Err(UsbError::Unsupported)}
  let command=io.read32(self.op(USBCMD))?;
  io.write32(self.op(USBCMD),command|CMD_RESET)?;self.state=EhciState::Resetting;Ok(())
 }
 pub fn poll_reset<I:EhciRegisterIo>(&mut self,io:&I)->Result<bool,UsbError>{
  if self.state!=EhciState::Resetting{return Err(UsbError::Unsupported)}
  if io.read32(self.op(USBCMD))?&CMD_RESET==0{self.state=EhciState::Reset;return Ok(true)}
  Ok(false)
 }
 /// Programs the periodic frame list and asynchronous QH using one 4-GiB
 /// segment. Caller must provide pinned, coherent DMA memory at these addresses.
 pub unsafe fn configure_schedules<I:EhciRegisterIo>(&mut self,io:&mut I,periodic_list:u64,async_head_qh:u64)->Result<(),UsbError>{
  if self.state!=EhciState::Reset||self.schedule_configured{return Err(UsbError::Unsupported)}
  if periodic_list&0xfff!=0||async_head_qh&0x1f!=0{return Err(UsbError::InvalidAddress)}
  let periodic_segment=(periodic_list>>32)as u32;let async_segment=(async_head_qh>>32)as u32;
  if periodic_segment!=async_segment||(!self.address_64bit&&periodic_segment!=0){return Err(UsbError::InvalidAddress)}
  io.write32(self.op(CTRLDSSEGMENT),periodic_segment)?;
  io.write32(self.op(PERIODICLISTBASE),periodic_list as u32)?;
  io.write32(self.op(ASYNCLISTADDR),async_head_qh as u32)?;
  io.write32(self.op(CONFIGFLAG),1)?;
  self.segment=periodic_segment;self.schedule_configured=true;Ok(())
 }
 pub fn start<I:EhciRegisterIo>(&mut self,io:&mut I)->Result<(),UsbError>{
  if self.state!=EhciState::Reset||!self.schedule_configured{return Err(UsbError::Unsupported)}
  let command=io.read32(self.op(USBCMD))?;
  io.write32(self.op(USBCMD),command|CMD_RUN|(1<<4)|(1<<5))
 }
 pub fn poll_running<I:EhciRegisterIo>(&mut self,io:&I)->Result<bool,UsbError>{
  if self.state!=EhciState::Reset{return Err(UsbError::Unsupported)}
  if io.read32(self.op(USBSTS))?&STS_HALTED==0{self.state=EhciState::Running;return Ok(true)}
  Ok(false)
 }
 pub fn port_status<I:EhciRegisterIo>(&self,io:&I,port:u8)->Result<super::controller::PortStatus,UsbError>{
  if self.state!=EhciState::Running{return Err(UsbError::ControllerUnavailable)}
  if port==0||port>self.port_count{return Err(UsbError::InvalidAddress)}
  let value=io.read32(self.op(PORTSC_BASE)+(port as usize-1)*4)?;
  Ok(super::controller::PortStatus{connected:value&1!=0,enabled:value&(1<<2)!=0,
   suspended:value&(1<<7)!=0,over_current:value&(1<<4)!=0,speed:None})
 }
 pub fn request_port_reset<I:EhciRegisterIo>(&self,io:&mut I,port:u8)->Result<(),UsbError>{
  if self.state!=EhciState::Running||port==0||port>self.port_count{return Err(UsbError::InvalidAddress)}
  let offset=self.op(PORTSC_BASE)+(port as usize-1)*4;let value=io.read32(offset)?;
  if value&1==0{return Err(UsbError::ControllerUnavailable)}
  const PRESERVE:u32=(1<<12)|(1<<13)|(3<<14)|(7<<16)|(7<<24);\n  io.write32(offset,(value&PRESERVE)|(1<<8))
 }
 pub fn port_reset_complete<I:EhciRegisterIo>(&self,io:&I,port:u8)->Result<bool,UsbError>{
  if port==0||port>self.port_count{return Err(UsbError::InvalidAddress)}
  let value=io.read32(self.op(PORTSC_BASE)+(port as usize-1)*4)?;
  Ok(value&(1<<8)==0&&value&(1<<2)!=0)
 }
 pub fn clear_port_changes<I:EhciRegisterIo>(&self,io:&mut I,port:u8,changes:u32)->Result<(),UsbError>{
  const MASK:u32=(1<<1)|(1<<3)|(1<<5)|(1<<19);
  if self.state!=EhciState::Running{return Err(UsbError::ControllerUnavailable)}
  if port==0||port>self.port_count||changes&!MASK!=0{return Err(UsbError::InvalidAddress)}
  let offset=self.op(PORTSC_BASE)+(port as usize-1)*4;
  let current=io.read32(offset)?;
  const PRESERVE:u32=(1<<2)|(1<<7)|(1<<12)|(1<<13)|(3<<14)|(7<<16)|(7<<24);
  io.write32(offset,(current&PRESERVE)|changes)
 }
}

#[cfg(test)]
mod controller_tests{
 use super::*;
 struct FakeEhci{registers:[u32;128]}
 impl FakeEhci{fn new()->Self{Self{registers:[0;128]}}}
 impl EhciRegisterIo for FakeEhci{
  fn read32(&self,offset:usize)->Result<u32,UsbError>{self.registers.get(offset/4).copied().ok_or(UsbError::BufferTooShort)}
  fn write32(&mut self,offset:usize,value:u32)->Result<(),UsbError>{
   let index=offset/4;if index>=self.registers.len(){return Err(UsbError::BufferTooShort)}
   self.registers[index]=value;
   if offset==0x20{
    if value&CMD_RESET!=0{self.registers[index]&=!CMD_RESET;}
    if value&CMD_RUN!=0{self.registers[index+1]&=!STS_HALTED;}else{self.registers[index+1]|=STS_HALTED;}
   }
   Ok(())
  }
 }
 #[test]fn halts_resets_configures_schedules_and_runs(){
  let mut io=FakeEhci::new();io.registers[0]=0x0100_0020;io.registers[1]=4;io.registers[2]=1;io.registers[9]=STS_HALTED;
  let mut h=EhciController::discover(&io).unwrap();
  h.request_halt(&mut io).unwrap();assert!(h.poll_halted(&io).unwrap());
  h.request_reset(&mut io).unwrap();assert!(h.poll_reset(&io).unwrap());
  unsafe{h.configure_schedules(&mut io,0x1000,0x2000)}.unwrap();
  h.start(&mut io).unwrap();assert!(h.poll_running(&io).unwrap());
  assert_eq!(h.state,EhciState::Running);
 }
 #[test]fn rejects_misaligned_ehci_schedule_memory(){
  let mut io=FakeEhci::new();io.registers[0]=0x0100_0020;io.registers[1]=4;
  let mut h=EhciController::discover(&io).unwrap();h.state=EhciState::Reset;
  assert_eq!(unsafe{h.configure_schedules(&mut io,0x1001,0x2000)},Err(UsbError::InvalidAddress));
 }
}
