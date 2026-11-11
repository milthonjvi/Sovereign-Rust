//! USB 2.0 and SuperSpeed hub descriptor parsing.
use super::{UsbError,transfer::SetupPacket};
pub const HUB_DESCRIPTOR:u8=0x29;
pub const HUB_DESCRIPTOR_SS:u8=0x2a;
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct HubDescriptor{pub ports:u8,pub characteristics:u16,pub power_on_to_good:u8,pub controller_current:u8}
impl HubDescriptor{
 pub fn parse(b:&[u8])->Result<Self,UsbError>{
  if b.len()<7||b[0]<7||b[1]!=HUB_DESCRIPTOR{return Err(UsbError::MalformedDescriptor)}
  Ok(Self{ports:b[2],characteristics:u16::from_le_bytes([b[3],b[4]]),power_on_to_good:b[5],controller_current:b[6]})
 }
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct SuperSpeedHubDescriptor{pub ports:u8,pub characteristics:u16,pub power_on_to_good:u8,pub controller_current:u8,pub header_decode_latency:u8,pub hub_delay:u16}
impl SuperSpeedHubDescriptor{
 pub fn parse(b:&[u8])->Result<Self,UsbError>{
  if b.len()<10||b[0]<10||b[1]!=HUB_DESCRIPTOR_SS{return Err(UsbError::MalformedDescriptor)}
  Ok(Self{ports:b[2],characteristics:u16::from_le_bytes([b[3],b[4]]),power_on_to_good:b[5],controller_current:b[6],header_decode_latency:b[7],hub_delay:u16::from_le_bytes([b[8],b[9]])})
 }
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
#[repr(u16)]
pub enum HubPortFeature{Connection=0,Enable=1,Suspend=2,OverCurrent=3,Reset=4,Power=8,LowSpeed=9,ConnectionChange=16,EnableChange=17,SuspendChange=18,OverCurrentChange=19,ResetChange=20}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct HubPort{pub number:u8,pub connected:bool,pub enabled:bool,pub powered:bool,pub speed_bits:u8}


// USB hub class requests (USB 2.0 and SuperSpeed use the same request codes).
pub const HUB_GET_STATUS:u8=0;
pub const HUB_CLEAR_FEATURE:u8=1;
pub const HUB_SET_FEATURE:u8=3;
pub const HUB_GET_DESCRIPTOR:u8=6;
pub const HUB_PORT_STATUS_LENGTH:u16=4;
pub const HUB_PORT_REQUEST_TYPE_IN:u8=0xa3;  // device-to-host, class, other
pub const HUB_PORT_REQUEST_TYPE_OUT:u8=0x23; // host-to-device, class, other
pub const HUB_DEVICE_REQUEST_TYPE_IN:u8=0xa0;
pub const HUB_DEVICE_REQUEST_TYPE_OUT:u8=0x20;

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct HubPortStatus{
 pub status:u16,pub change:u16,
}
impl HubPortStatus{
 pub const CONNECTION:u16=1<<0;
 pub const ENABLE:u16=1<<1;
 pub const SUSPEND:u16=1<<2;
 pub const OVER_CURRENT:u16=1<<3;
 pub const RESET:u16=1<<4;
 pub const POWER:u16=1<<8;
 pub const LOW_SPEED:u16=1<<9;
 pub const HIGH_SPEED:u16=1<<10;
 pub const TEST:u16=1<<11;
 pub const INDICATOR:u16=1<<12;
 pub const C_CONNECTION:u16=1<<0;
 pub const C_ENABLE:u16=1<<1;
 pub const C_SUSPEND:u16=1<<2;
 pub const C_OVER_CURRENT:u16=1<<3;
 pub const C_RESET:u16=1<<4;
 pub fn parse(bytes:&[u8])->Result<Self,UsbError>{
  if bytes.len()<4{return Err(UsbError::BufferTooShort)}
  Ok(Self{status:u16::from_le_bytes([bytes[0],bytes[1]]),
   change:u16::from_le_bytes([bytes[2],bytes[3]])})
 }
 pub const fn connected(self)->bool{self.status&Self::CONNECTION!=0}
 pub const fn enabled(self)->bool{self.status&Self::ENABLE!=0}
 pub const fn powered(self)->bool{self.status&Self::POWER!=0}
 pub const fn low_speed(self)->bool{self.status&Self::LOW_SPEED!=0}
 pub const fn high_speed(self)->bool{self.status&Self::HIGH_SPEED!=0}
 pub const fn has_change(self)->bool{self.change!=0}
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub enum HubPortAction{PowerOn,PowerOff,Reset,Enable,Suspend,Resume,ClearConnectionChange,ClearEnableChange,ClearSuspendChange,ClearOverCurrentChange,ClearResetChange}
impl HubPortAction{
 pub const fn feature(self)->u16{
  match self{
   Self::PowerOn|Self::PowerOff=>8,Self::Reset=>4,Self::Enable=>1,Self::Suspend|Self::Resume=>2,
   Self::ClearConnectionChange=>16,Self::ClearEnableChange=>17,Self::ClearSuspendChange=>18,
   Self::ClearOverCurrentChange=>19,Self::ClearResetChange=>20,
  }
 }
 pub const fn request(self)->u8{
  match self{
   Self::PowerOn|Self::Reset|Self::Enable|Self::Suspend=>HUB_SET_FEATURE,
   Self::PowerOff|Self::Resume|Self::ClearConnectionChange|Self::ClearEnableChange|
   Self::ClearSuspendChange|Self::ClearOverCurrentChange|Self::ClearResetChange=>HUB_CLEAR_FEATURE,
  }
 }
 pub const fn request_type(self)->u8{HUB_PORT_REQUEST_TYPE_OUT}
}
/// Builds a class GET_STATUS request for a downstream port (port numbers start at 1).
pub const fn get_port_status(port:u8)->SetupPacket{
 SetupPacket::new(HUB_PORT_REQUEST_TYPE_IN,HUB_GET_STATUS,0,port as u16,HUB_PORT_STATUS_LENGTH)
}
/// Builds a hub class request that changes a downstream port feature.
pub const fn port_action(port:u8,action:HubPortAction)->SetupPacket{
 SetupPacket::new(action.request_type(),action.request(),action.feature(),port as u16,0)
}

#[cfg(test)]
mod request_tests{
 use super::*;
 #[test]fn port_status_decodes_connection_speed_and_changes(){
  let s=HubPortStatus::parse(&[1,6,1,0]).unwrap();
  assert!(s.connected());assert!(s.enabled());assert!(s.high_speed());
  assert!(s.has_change());
 }
 #[test]fn port_requests_use_class_other_recipient_and_one_based_port(){
  let get=get_port_status(3);assert_eq!(get.request_type,0xa3);
  assert_eq!(get.request,0);assert_eq!(get.index,3);assert_eq!(get.length,4);
  let reset=port_action(2,HubPortAction::Reset);
  assert_eq!(reset.request_type,0x23);assert_eq!(reset.request,3);
  assert_eq!(reset.value,4);assert_eq!(reset.index,2);
 }
}
