//! OHCI (USB 1.0/1.1) memory-mapped register offsets.
pub const REVISION:usize=0x00;pub const CONTROL:usize=0x04;pub const COMMAND_STATUS:usize=0x08;
pub const INTERRUPT_STATUS:usize=0x0c;pub const INTERRUPT_ENABLE:usize=0x10;
pub const HCCA:usize=0x18;pub const PERIOD_CURRENT_ED:usize=0x1c;
pub const CONTROL_HEAD_ED:usize=0x20;pub const CONTROL_CURRENT_ED:usize=0x24;
pub const BULK_HEAD_ED:usize=0x28;pub const BULK_CURRENT_ED:usize=0x2c;
pub const DONE_HEAD:usize=0x30;pub const FRAME_INTERVAL:usize=0x34;
pub const RH_DESCRIPTOR_A:usize=0x48;pub const RH_DESCRIPTOR_B:usize=0x4c;
pub const RH_STATUS:usize=0x50;pub const RH_PORT_STATUS_BASE:usize=0x54;
pub const CONTROL_HCFS_MASK:u32=3<<6;pub const HCFS_RESET:u32=0<<6;pub const HCFS_RESUME:u32=1<<6;pub const HCFS_OPERATIONAL:u32=2<<6;pub const HCFS_SUSPEND:u32=3<<6;

use super::UsbError;
pub const ED_SKIP:u32=1<<14;
pub const ED_LOW_SPEED:u32=1<<13;
pub const ED_MAX_PACKET_SHIFT:u32=16;
pub const TD_ROUNDING:u32=1<<18;
pub const TD_DIRECTION_SETUP:u32=0<<19;
pub const TD_DIRECTION_OUT:u32=1<<19;
pub const TD_DIRECTION_IN:u32=2<<19;
pub const TD_DIRECTION_FROM_ED:u32=3<<19;
pub const TD_TOGGLE_DATA0:u32=0<<24;
pub const TD_TOGGLE_DATA1:u32=1<<24;
pub const TD_TOGGLE_FROM_ED:u32=2<<24;
pub const TD_CC_NOT_ACCESSED:u32=0xf<<28;

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub enum OhciDirection{FromTd,Out,In,Setup}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub enum OhciDataToggle{Data0,Data1,FromEndpoint}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
#[repr(C,align(16))]
pub struct OhciEndpointDescriptor{pub control:u32,pub tail_pointer:u32,pub head_pointer:u32,pub next_ed:u32}
impl OhciEndpointDescriptor{
 pub fn new(address:u8,endpoint:u8,direction:OhciDirection,low_speed:bool,max_packet:u16)->Result<Self,UsbError>{
  if address>127||endpoint>15||max_packet==0||max_packet>2047{return Err(UsbError::MalformedDescriptor)}
  let dir=match direction{OhciDirection::FromTd=>0,OhciDirection::Out=>1,OhciDirection::In=>2,OhciDirection::Setup=>3};
  let control=address as u32|((endpoint as u32)<<7)|(dir<<11)|
   ((low_speed as u32)<<13)|ED_SKIP|((max_packet as u32)<<ED_MAX_PACKET_SHIFT);
  Ok(Self{control,tail_pointer:0,head_pointer:0,next_ed:0})
 }
 pub fn set_queue(&mut self,head:u64,tail:u64)->Result<(),UsbError>{
  if head>u32::MAX as u64||tail>u32::MAX as u64||head&0xf!=0||tail&0xf!=0{return Err(UsbError::InvalidAddress)}
  self.head_pointer=head as u32;self.tail_pointer=tail as u32;Ok(())
 }
 pub fn set_next(&mut self,next:u64)->Result<(),UsbError>{
  if next>u32::MAX as u64||next&0xf!=0{return Err(UsbError::InvalidAddress)}
  self.next_ed=next as u32;Ok(())
 }
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
#[repr(C,align(16))]
pub struct OhciTransferDescriptor{pub control:u32,pub current_buffer:u32,pub next_td:u32,pub buffer_end:u32}
impl OhciTransferDescriptor{
 pub fn new(direction:OhciDirection,data_toggle:OhciDataToggle,buffer_start:u64,buffer_end:u64,next_td:u64,interrupt_delay:u8,rounding:bool)->Result<Self,UsbError>{
  if interrupt_delay>7||buffer_start>u32::MAX as u64||buffer_end>u32::MAX as u64||next_td>u32::MAX as u64{return Err(UsbError::InvalidAddress)}
  if next_td&0xf!=0{return Err(UsbError::InvalidAddress)}
  let dir=match direction{OhciDirection::Setup=>TD_DIRECTION_SETUP,OhciDirection::Out=>TD_DIRECTION_OUT,OhciDirection::In=>TD_DIRECTION_IN,OhciDirection::FromTd=>TD_DIRECTION_FROM_ED};
  let toggle=match data_toggle{OhciDataToggle::Data0=>TD_TOGGLE_DATA0,OhciDataToggle::Data1=>TD_TOGGLE_DATA1,OhciDataToggle::FromEndpoint=>TD_TOGGLE_FROM_ED};
  let control=TD_CC_NOT_ACCESSED|dir|((interrupt_delay as u32)<<21)|toggle|if rounding{TD_ROUNDING}else{0};
  Ok(Self{control,current_buffer:buffer_start as u32,next_td:next_td as u32,buffer_end:buffer_end as u32})
 }
}
#[derive(Clone,Copy)]
#[repr(C,align(256))]
pub struct OhciHcca{pub interrupt_table:[u32;32],pub frame_number:u16,pub padding:u16,pub done_head:u32,pub reserved:[u8;120]}
impl OhciHcca{pub const fn new()->Self{Self{interrupt_table:[0;32],frame_number:0,padding:0,done_head:0,reserved:[0;120]}}}
impl Default for OhciHcca{fn default()->Self{Self::new()}}

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct OhciPortStatus{pub connected:bool,pub enabled:bool,pub suspended:bool,pub over_current:bool,pub reset:bool,pub powered:bool,pub low_speed:bool,pub connection_changed:bool}
impl OhciPortStatus{
 pub fn parse(value:u32)->Self{Self{connected:value&1!=0,enabled:value&(1<<1)!=0,suspended:value&(1<<2)!=0,over_current:value&(1<<3)!=0,reset:value&(1<<4)!=0,powered:value&(1<<8)!=0,low_speed:value&(1<<9)!=0,connection_changed:value&(1<<16)!=0}}
}

#[cfg(test)]
mod descriptor_tests{
 use super::*;
 #[test]fn builds_ohci_endpoint_and_transfer_descriptors(){
  let ed=OhciEndpointDescriptor::new(5,2,OhciDirection::In,true,64).unwrap();
  assert_eq!(ed.control&0x7f,5);assert_ne!(ed.control&ED_SKIP,0);
  let td=OhciTransferDescriptor::new(OhciDirection::In,OhciDataToggle::FromEndpoint,0x1000,0x103f,0x2000,7,true).unwrap();
  assert_ne!(td.control&TD_ROUNDING,0);assert_eq!(td.current_buffer,0x1000);
 }
 #[test]fn validates_ohci_dma_alignment(){
  assert_eq!(OhciEndpointDescriptor::new(128,0,OhciDirection::Out,false,64),Err(UsbError::MalformedDescriptor));
  assert_eq!(OhciTransferDescriptor::new(OhciDirection::Out,OhciDataToggle::FromEndpoint,0x1001,0x103f,0x2001,0,false),Err(UsbError::InvalidAddress));
 }
 #[test]fn hcca_has_required_256_byte_alignment(){
  assert_eq!(core::mem::align_of::<OhciHcca>(),256);assert_eq!(core::mem::size_of::<OhciHcca>(),256);
 }
}
