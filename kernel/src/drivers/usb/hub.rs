//! USB 2.0 and SuperSpeed hub descriptor parsing.
use super::UsbError;
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
