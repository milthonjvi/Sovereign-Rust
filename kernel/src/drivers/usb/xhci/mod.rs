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
#[derive(Clone,Copy,Debug,Eq,PartialEq)]pub enum XhciError{InvalidCapability,UnsupportedVersion,NotReady,HardwareFault}
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
