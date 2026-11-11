//! xHCI register and Transfer Request Block definitions.
//! DMA and MMIO mapping must be supplied by kernel memory and PCI services.
pub const TRB_SIZE:usize=16;
#[derive(Clone,Copy,Debug,Eq,PartialEq)]#[repr(C,align(16))]
pub struct Trb{pub parameter:u64,pub status:u32,pub control:u32}
impl Trb{
 pub const EMPTY:Self=Self{parameter:0,status:0,control:0};
 pub const fn trb_type(self)->u8{((self.control>>10)&63)as u8}
 pub const fn cycle_bit(self)->bool{self.control&1!=0}
 pub const fn with_cycle(mut self,cycle:bool)->Self{if cycle{self.control|=1}else{self.control&=!1}self}
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]pub enum ControllerState{Detached,Discovered,Halted,Resetting,Reset,Running,Failed}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]pub enum XhciError{InvalidCapability,UnsupportedVersion,NotReady,HardwareFault,InvalidRing,RingFull}
/// Bounds-checked access to an already mapped xHCI MMIO region.
pub trait RegisterIo{fn read32(&self,offset:usize)->Result<u32,XhciError>;fn write32(&mut self,offset:usize,value:u32)->Result<(),XhciError>;}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct CapabilityRegisters{pub capability_length:u8,pub interface_version:u16,pub max_device_slots:u8,pub max_interrupters:u16,pub max_ports:u8}
impl CapabilityRegisters{
 pub fn parse(cap0:u32,hcs1:u32)->Result<Self,XhciError>{
  let len=(cap0&255)as u8;let version=(cap0>>16)as u16;
  if len<0x20{return Err(XhciError::InvalidCapability)}
  if version<0x0100{return Err(XhciError::UnsupportedVersion)}
  Ok(Self{capability_length:len,interface_version:version,max_device_slots:(hcs1&255)as u8,max_interrupters:((hcs1>>8)&0x7ff)as u16,max_ports:(hcs1>>24)as u8})
 }
}


/// DMA-backed producer ring. The caller must provide physically contiguous,
/// DMA-coherent storage mapped at physical_base and keep it alive while in use.
/// One TRB slot is reserved for the Link TRB; in-flight accounting prevents
/// software from overwriting commands not yet retired by completion events.
pub struct XhciProducerRing<'a,const N:usize>{entries:&'a mut[Trb;N],physical_base:u64,enqueue:usize,cycle:bool,in_flight:usize}
impl<'a,const N:usize>XhciProducerRing<'a,N>{
 /// # Safety
 /// `physical_base` must be the DMA address of `entries`, which must be
 /// contiguous, coherent, 64-byte aligned, and remain pinned for the ring lifetime.
 pub unsafe fn new(entries:&'a mut[Trb;N],physical_base:u64)->Result<Self,XhciError>{
  if N<2||physical_base&0x3f!=0{return Err(XhciError::InvalidRing)}
  for entry in entries.iter_mut(){unsafe{core::ptr::write_volatile(entry,Trb::EMPTY);}}
  let mut ring=Self{entries,physical_base,enqueue:0,cycle:true,in_flight:0};
  ring.write_link();Ok(ring)
 }
 fn write_link(&mut self){
  let control=(6u32<<10)|(1<<1)|(self.cycle as u32);
  let entry=&mut self.entries[N-1];
  unsafe{
   core::ptr::write_volatile(&mut entry.control,control^1);
   core::ptr::write_volatile(&mut entry.parameter,self.physical_base);
   core::ptr::write_volatile(&mut entry.status,0);
  }
  core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
  unsafe{core::ptr::write_volatile(&mut entry.control,control);}
 }
 pub fn push(&mut self,mut trb:Trb)->Result<usize,XhciError>{
  if self.in_flight>=N-1{return Err(XhciError::RingFull)}
  let index=self.enqueue;
  trb=trb.with_cycle(self.cycle);
  let entry=&mut self.entries[index];
  unsafe{
   core::ptr::write_volatile(&mut entry.control,trb.control^1);
   core::ptr::write_volatile(&mut entry.parameter,trb.parameter);
   core::ptr::write_volatile(&mut entry.status,trb.status);
  }
  core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
  unsafe{core::ptr::write_volatile(&mut entry.control,trb.control);}
  self.in_flight+=1;self.enqueue+=1;
  if self.enqueue==N-1{
   self.write_link();
   core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
   self.enqueue=0;self.cycle=!self.cycle;
  }
  Ok(index)
 }
 /// Retires completed TRBs. The caller must only retire work confirmed by
 /// command/transfer completion events.
 pub fn retire(&mut self,count:usize)->Result<(),XhciError>{
  if count>self.in_flight{return Err(XhciError::InvalidRing)}
  self.in_flight-=count;Ok(())
 }
 pub const fn in_flight(&self)->usize{self.in_flight}
 pub const fn producer_cycle(&self)->bool{self.cycle}
 pub const fn physical_base(&self)->u64{self.physical_base}
}

/// Consumer for an already configured DMA event ring. Event TRBs are returned
/// only when their Cycle bit matches the consumer cycle state.
pub struct XhciEventRing<'a,const N:usize>{entries:&'a mut[Trb;N],dequeue:usize,cycle:bool}
impl<'a,const N:usize>XhciEventRing<'a,N>{
 pub fn new(entries:&'a[Trb;N])->Result<Self,XhciError>{
  if N==0{return Err(XhciError::InvalidRing)}
  Ok(Self{entries,dequeue:0,cycle:true})
 }
 pub fn pop(&mut self)->Option<Trb>{
  let entry=&self.entries[self.dequeue];
  let cycle=unsafe{core::ptr::read_volatile(&entry.control)&1!=0};
  if cycle!=self.cycle{return None}
  core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire);
  let trb=unsafe{core::ptr::read_volatile(entry)};
  self.dequeue+=1;
  if self.dequeue==N{self.dequeue=0;self.cycle=!self.cycle;}
  Some(trb)
 }
 pub const fn dequeue_index(&self)->usize{self.dequeue}
 pub const fn consumer_cycle(&self)->bool{self.cycle}
}

#[cfg(test)]
mod ring_tests{
 use super::*;
 #[test]fn producer_ring_wraps_with_link_trb_and_cycle_toggle(){
  let mut entries=[Trb::EMPTY;4];
  let mut ring=unsafe{XhciProducerRing::new(&mut entries,0x1000)}.unwrap();
  assert_eq!(ring.push(Trb{parameter:1,status:0,control:1}).unwrap(),0);
  assert_eq!(ring.push(Trb{parameter:2,status:0,control:1}).unwrap(),1);
  assert_eq!(ring.push(Trb{parameter:3,status:0,control:1}).unwrap(),2);
  assert!(!ring.producer_cycle());assert_eq!(ring.in_flight(),3);
  assert_eq!(ring.push(Trb::EMPTY),Err(XhciError::RingFull));
  ring.retire(2).unwrap();assert_eq!(ring.push(Trb::EMPTY).unwrap(),0);
  assert_eq!(ring.in_flight(),2);
 }
 #[test]fn producer_requires_dma_alignment(){
  let mut entries=[Trb::EMPTY;4];
  assert!(matches!(unsafe{XhciProducerRing::new(&mut entries,0x1001)},Err(XhciError::InvalidRing)));
 }
 #[test]fn event_ring_consumes_only_matching_cycle(){
  let mut entries=[Trb::EMPTY;2];
  let mut ring=unsafe{XhciEventRing::new(&mut entries)}.unwrap();
  unsafe{
   core::ptr::write_volatile(&mut ring.entries[0],Trb{parameter:7,status:0,control:1});
   core::ptr::write_volatile(&mut ring.entries[1],Trb{parameter:8,status:0,control:1});
  }
  assert_eq!(ring.pop().unwrap().parameter,7);assert_eq!(ring.pop().unwrap().parameter,8);
  assert_eq!(ring.pop(),None);assert_eq!(ring.dequeue_index(),0);assert!(!ring.consumer_cycle());
 }
}


#[derive(Clone,Copy,Debug,Eq,PartialEq)]
#[repr(u8)]
pub enum TrbType{Normal=1,SetupStage=2,DataStage=3,StatusStage=4,Isoch=5,Link=6,EventData=7,NoopTransfer=8,EnableSlot=9,DisableSlot=10,AddressDevice=11,ConfigureEndpoint=12,EvaluateContext=13,ResetEndpoint=14,StopEndpoint=15,SetTrDequeuePointer=16,ResetDevice=17,ForceEvent=18,NegotiateBandwidth=19,SetLatencyTolerance=20,GetPortBandwidth=21,ForceHeader=22,NoopCommand=23,TransferEvent=32,CommandCompletion=33,PortStatusChange=34,HostControllerEvent=37}
impl Trb{
 pub const fn command(kind:TrbType,parameter:u64,status:u32,slot_id:u8)->Self{
  Self{parameter,status,control:((kind as u32)<<10)|((slot_id as u32)<<24)}
 }
 pub const fn enable_slot()->Self{Self::command(TrbType::EnableSlot,0,0,0)}
 pub const fn address_device(input_context:u64,slot_id:u8)->Self{Self::command(TrbType::AddressDevice,input_context,0,slot_id)}
 pub const fn configure_endpoint(input_context:u64,slot_id:u8)->Self{Self::command(TrbType::ConfigureEndpoint,input_context,0,slot_id)}
 pub const fn evaluate_context(input_context:u64,slot_id:u8)->Self{Self::command(TrbType::EvaluateContext,input_context,0,slot_id)}
 pub const fn noop_command()->Self{Self::command(TrbType::NoopCommand,0,0,0)}
 pub const fn normal_transfer(buffer:u64,length:u32,td_size:u8,interrupt_on_completion:bool)->Self{
  Self{parameter:buffer,status:(length&0x1ffff)|(((td_size as u32)&0x1f)<<17),
   control:((TrbType::Normal as u32)<<10)|(1<<4)|((interrupt_on_completion as u32)<<5)|(1<<2)}
 }
 pub fn setup_stage(setup:super::transfer::SetupPacket)->Self{
  let encoded=setup.encode();
  let parameter=u64::from_le_bytes(encoded);
  let trt=if setup.length==0{0}else if setup.request_type&0x80!=0{3}else{2};
  Self{parameter,status:8,control:((TrbType::SetupStage as u32)<<10)|(1<<6)|(1<<4)|(trt<<16)}
 }
 pub const fn data_stage(buffer:u64,length:u32,td_size:u8,direction_in:bool,interrupt_on_completion:bool)->Self{
  Self{parameter:buffer,status:(length&0x1ffff)|(((td_size as u32)&0x1f)<<17),
   control:((TrbType::DataStage as u32)<<10)|((direction_in as u32)<<16)|(1<<4)|(1<<2)|((interrupt_on_completion as u32)<<5)}
 }
 pub const fn status_stage(direction_in:bool,interrupt_on_completion:bool)->Self{
  Self{parameter:0,status:0,control:((TrbType::StatusStage as u32)<<10)|((direction_in as u32)<<16)|((interrupt_on_completion as u32)<<5)}
 }
}

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct TransferEvent{pub pointer:u64,pub residual_length:u32,pub completion_code:u8,pub endpoint_id:u8,pub slot_id:u8}
impl TransferEvent{
 pub fn parse(trb:Trb)->Result<Self,XhciError>{
  if trb.trb_type()!=TrbType::TransferEvent as u8{return Err(XhciError::InvalidCapability)}
  Ok(Self{pointer:trb.parameter,residual_length:trb.status&0x00ff_ffff,
   completion_code:(trb.status>>24)as u8,endpoint_id:((trb.control>>16)&0x1f)as u8,slot_id:(trb.control>>24)as u8})
 }
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct CommandCompletionEvent{pub command_pointer:u64,pub completion_code:u8,pub slot_id:u8}
impl CommandCompletionEvent{
 pub fn parse(trb:Trb)->Result<Self,XhciError>{
  if trb.trb_type()!=TrbType::CommandCompletion as u8{return Err(XhciError::InvalidCapability)}
  Ok(Self{command_pointer:trb.parameter,completion_code:(trb.status>>24)as u8,slot_id:(trb.control>>24)as u8})
 }
}

#[cfg(test)]
mod trb_builder_tests{
 use super::*;
 use crate::drivers::usb::transfer::SetupPacket;
 #[test]fn builds_control_setup_and_data_trbs(){
  let setup=SetupPacket::get_descriptor(1,0,18);
  let setup_trb=Trb::setup_stage(setup);
  assert_eq!(setup_trb.trb_type(),TrbType::SetupStage as u8);
  assert_eq!(setup_trb.parameter,u64::from_le_bytes(setup.encode()));
  assert_eq!((setup_trb.control>>16)&3,3);
  let data=Trb::data_stage(0x2000,18,0,true,true);
  assert_eq!(data.trb_type(),TrbType::DataStage as u8);
  assert_ne!(data.control&(1<<16),0);
  let status=Trb::status_stage(false,true);
  assert_eq!(status.trb_type(),TrbType::StatusStage as u8);
 }
 #[test]fn decodes_transfer_and_command_completion_events(){
  let transfer=Trb{parameter:0x1230,status:(1<<24)|7,control:((TrbType::TransferEvent as u32)<<10)|(3<<16)|(2<<24)};
  let event=TransferEvent::parse(transfer).unwrap();
  assert_eq!(event.pointer,0x1230);assert_eq!(event.residual_length,7);
  assert_eq!(event.completion_code,1);assert_eq!(event.endpoint_id,3);assert_eq!(event.slot_id,2);
  let command=Trb{parameter:0x8000,status:1<<24,control:((TrbType::CommandCompletion as u32)<<10)|(2<<24)};
  assert_eq!(CommandCompletionEvent::parse(command).unwrap().command_pointer,0x8000);
 }
}


pub const OP_USBCMD:usize=0x00;
pub const OP_USBSTS:usize=0x04;
pub const OP_CRCR:usize=0x18;
pub const OP_DCBAAP:usize=0x30;
pub const OP_CONFIG:usize=0x38;
pub const OP_PORT_BASE:usize=0x400;
pub const OP_PORT_STRIDE:usize=0x10;
pub const USBCMD_RUN:u32=1;
pub const USBCMD_RESET:u32=1<<1;
pub const USBSTS_HALTED:u32=1;
pub const USBSTS_CONTROLLER_NOT_READY:u32=1<<11;

/// Non-blocking xHCI bring-up. DMA memory and MMIO mapping are provided by
/// kernel memory/PCI layers; every poll returns so the caller can enforce timeout.
pub struct XhciController{pub state:ControllerState,pub capabilities:CapabilityRegisters,operational_base:usize,pub enabled_slots:u8}
impl XhciController{
 pub fn discover<I:RegisterIo>(io:&I)->Result<Self,XhciError>{
  let cap0=io.read32(0)?;let hcs1=io.read32(4)?;
  let capabilities=CapabilityRegisters::parse(cap0,hcs1)?;
  Ok(Self{state:ControllerState::Discovered,capabilities,operational_base:capabilities.capability_length as usize,enabled_slots:0})
 }
 fn op(&self,offset:usize)->usize{self.operational_base+offset}
 pub fn request_halt<I:RegisterIo>(&mut self,io:&mut I)->Result<(),XhciError>{
  if self.state!=ControllerState::Discovered&&self.state!=ControllerState::Running{return Err(XhciError::InvalidState)}
  let command=io.read32(self.op(OP_USBCMD))?;
  io.write32(self.op(OP_USBCMD),command&!USBCMD_RUN)?;Ok(())
 }
 pub fn poll_halted<I:RegisterIo>(&mut self,io:&I)->Result<bool,XhciError>{
  if self.state!=ControllerState::Discovered&&self.state!=ControllerState::Running{return Err(XhciError::InvalidState)}
  if io.read32(self.op(OP_USBSTS))?&USBSTS_HALTED!=0{self.state=ControllerState::Halted;return Ok(true)}
  Ok(false)
 }
 pub fn request_reset<I:RegisterIo>(&mut self,io:&mut I)->Result<(),XhciError>{
  if self.state!=ControllerState::Halted{return Err(XhciError::InvalidState)}
  let command=io.read32(self.op(OP_USBCMD))?;
  io.write32(self.op(OP_USBCMD),command|USBCMD_RESET)?;self.state=ControllerState::Resetting;Ok(())
 }
 pub fn poll_reset<I:RegisterIo>(&mut self,io:&I)->Result<bool,XhciError>{
  if self.state!=ControllerState::Resetting{return Err(XhciError::InvalidState)}
  let command=io.read32(self.op(OP_USBCMD))?;let status=io.read32(self.op(OP_USBSTS))?;
  if command&USBCMD_RESET==0&&status&USBSTS_CONTROLLER_NOT_READY==0{self.state=ControllerState::Reset;return Ok(true)}
  Ok(false)
 }
 /// Addresses must be physical, DMA-visible and aligned as required by xHCI.
 pub fn configure_dma<I:RegisterIo>(&mut self,io:&mut I,dcbaa:u64,command_ring:u64,slots:u8)->Result<(),XhciError>{
  if self.state!=ControllerState::Reset{return Err(XhciError::InvalidState)}
  if dcbaa&0x3f!=0||command_ring&0x3f!=0{return Err(XhciError::InvalidDmaAddress)}
  if slots==0||slots>self.capabilities.max_device_slots{return Err(XhciError::InvalidCapability)}
  io.write32(self.op(OP_DCBAAP)+4,(dcbaa>>32)as u32)?;
  io.write32(self.op(OP_DCBAAP),dcbaa as u32)?;
  io.write32(self.op(OP_CRCR)+4,(command_ring>>32)as u32)?;
  io.write32(self.op(OP_CRCR),(command_ring as u32)|1)?;
  io.write32(self.op(OP_CONFIG),slots as u32)?;
  self.enabled_slots=slots;Ok(())
 }
 pub fn start<I:RegisterIo>(&mut self,io:&mut I)->Result<(),XhciError>{
  if self.state!=ControllerState::Reset||self.enabled_slots==0{return Err(XhciError::InvalidState)}
  let command=io.read32(self.op(OP_USBCMD))?;
  io.write32(self.op(OP_USBCMD),command|USBCMD_RUN)?;Ok(())
 }
 pub fn poll_running<I:RegisterIo>(&mut self,io:&I)->Result<bool,XhciError>{
  if self.state!=ControllerState::Reset{return Err(XhciError::InvalidState)}
  let status=io.read32(self.op(OP_USBSTS))?;
  if status&USBSTS_CONTROLLER_NOT_READY==0&&status&USBSTS_HALTED==0{self.state=ControllerState::Running;return Ok(true)}
  Ok(false)
 }
 pub fn read_port_status<I:RegisterIo>(&self,io:&I,port:u8)->Result<super::super::controller::PortStatus,XhciError>{
  if self.state!=ControllerState::Running{return Err(XhciError::NotReady)}
  if port==0||port>self.capabilities.max_ports{return Err(XhciError::InvalidCapability)}
  let value=io.read32(self.op(OP_PORT_BASE)+(port as usize-1)*OP_PORT_STRIDE)?;
  let speed=match (value>>10)&0xf{1=>Some(super::super::transfer::UsbSpeed::Full),2=>Some(super::super::transfer::UsbSpeed::Low),3=>Some(super::super::transfer::UsbSpeed::High),4=>Some(super::super::transfer::UsbSpeed::Super),5=>Some(super::super::transfer::UsbSpeed::SuperPlus),_=>None};
  Ok(super::super::controller::PortStatus{connected:value&1!=0,enabled:value&2!=0,suspended:(value>>5)&0xf==3,over_current:value&(1<<3)!=0,speed})
 }
}

#[cfg(test)]
mod controller_state_tests{
 use super::*;
 struct FakeRegisters{words:[u32;256]}
 impl FakeRegisters{fn new()->Self{Self{words:[0;256]}}}
 impl RegisterIo for FakeRegisters{
  fn read32(&self,offset:usize)->Result<u32,XhciError>{self.words.get(offset/4).copied().ok_or(XhciError::HardwareFault)}
  fn write32(&mut self,offset:usize,value:u32)->Result<(),XhciError>{
   let index=offset/4;if index>=self.words.len(){return Err(XhciError::HardwareFault)}
   self.words[index]=value;
   if offset==0x40{
    if value&USBCMD_RESET!=0{self.words[index]&=!USBCMD_RESET;}
    if value&USBCMD_RUN!=0{self.words[index+1]&=!USBSTS_HALTED;}else{self.words[index+1]|=USBSTS_HALTED;}
   }
   Ok(())
  }
 }
 #[test]fn runs_halt_reset_configure_and_start_sequence(){
  let mut io=FakeRegisters::new();io.words[0]=0x0101_0040;io.words[1]=8|(1<<24);io.words[17]=USBSTS_HALTED;
  let mut h=XhciController::discover(&io).unwrap();
  h.request_halt(&mut io).unwrap();assert!(h.poll_halted(&io).unwrap());
  h.request_reset(&mut io).unwrap();assert!(h.poll_reset(&io).unwrap());
  h.configure_dma(&mut io,0x2000,0x3000,8).unwrap();
  assert_eq!(io.words[0x40/4+OP_CONFIG/4],8);
  h.start(&mut io).unwrap();assert!(h.poll_running(&io).unwrap());
  assert_eq!(h.state,ControllerState::Running);
 }
 #[test]fn rejects_unaligned_dma_addresses(){
  let mut io=FakeRegisters::new();io.words[0]=0x0101_0040;io.words[1]=8;
  let mut h=XhciController::discover(&io).unwrap();h.state=ControllerState::Reset;
  assert_eq!(h.configure_dma(&mut io,0x2001,0x3000,4),Err(XhciError::InvalidDmaAddress));
 }
}
