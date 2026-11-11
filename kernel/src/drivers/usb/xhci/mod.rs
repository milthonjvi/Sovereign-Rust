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
#[derive(Clone,Copy,Debug,Eq,PartialEq)]pub enum ControllerState{Detached,Discovered,Reset,Running,Halted,Failed}
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
 pub fn new(entries:&'a mut[Trb;N],physical_base:u64)->Result<Self,XhciError>{
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
pub struct XhciEventRing<'a,const N:usize>{entries:&'a[Trb;N],dequeue:usize,cycle:bool}
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
  let mut ring=XhciProducerRing::new(&mut entries,0x1000).unwrap();
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
  assert!(matches!(XhciProducerRing::new(&mut entries,0x1001),Err(XhciError::InvalidRing)));
 }
 #[test]fn event_ring_consumes_only_matching_cycle(){
  let mut entries=[Trb::EMPTY;2];
  entries[0]=Trb{parameter:7,status:0,control:1};
  entries[1]=Trb{parameter:8,status:0,control:1};
  let mut ring=XhciEventRing::new(&entries).unwrap();
  assert_eq!(ring.pop().unwrap().parameter,7);assert_eq!(ring.pop().unwrap().parameter,8);
  assert_eq!(ring.pop(),None);assert_eq!(ring.dequeue_index(),0);assert!(!ring.consumer_cycle());
 }
}
