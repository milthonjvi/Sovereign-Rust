//! USB 2.0 and SuperSpeed hub descriptor parsing.
use super::{UsbError,transfer::{SetupPacket,UsbSpeed}};
pub const HUB_DESCRIPTOR:u8=0x29;
pub const HUB_DESCRIPTOR_SS:u8=0x2a;
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct HubDescriptor{pub ports:u8,pub characteristics:u16,pub power_on_to_good:u8,pub controller_current:u8}
impl HubDescriptor{
 pub fn parse(b:&[u8])->Result<Self,UsbError>{
  if b.len()<7||b[1]!=HUB_DESCRIPTOR||b[2]==0{return Err(UsbError::MalformedDescriptor)}
  let bitmap_bytes=((b[2] as usize+1)+7)/8;
  let required=7+bitmap_bytes*2;
  if b.len()<required||b[0] as usize>b.len(){return Err(UsbError::BufferTooShort)}
  if b[0] as usize!=required{return Err(UsbError::MalformedDescriptor)}
  Ok(Self{ports:b[2],characteristics:u16::from_le_bytes([b[3],b[4]]),power_on_to_good:b[5],controller_current:b[6]})
 }
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct SuperSpeedHubDescriptor{pub ports:u8,pub characteristics:u16,pub power_on_to_good:u8,pub controller_current:u8,pub header_decode_latency:u8,pub hub_delay:u16}
impl SuperSpeedHubDescriptor{
 pub fn parse(b:&[u8])->Result<Self,UsbError>{
  if b.len()<10||b[1]!=HUB_DESCRIPTOR_SS||b[2]==0{return Err(UsbError::MalformedDescriptor)}
  let bitmap_bytes=((b[2] as usize+1)+7)/8;
  let required=10+bitmap_bytes;
  if b.len()<required||b[0] as usize>b.len(){return Err(UsbError::BufferTooShort)}
  if b[0] as usize!=required{return Err(UsbError::MalformedDescriptor)}
  Ok(Self{ports:b[2],characteristics:u16::from_le_bytes([b[3],b[4]]),power_on_to_good:b[5],controller_current:b[6],header_decode_latency:b[7],hub_delay:u16::from_le_bytes([b[8],b[9]])})
 }
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
#[repr(u16)]
pub enum HubPortFeature{Connection=0,Enable=1,Suspend=2,OverCurrent=3,Reset=4,PortLinkState=5,BHPortReset=28,Power=8,LowSpeed=9,ConnectionChange=16,EnableChange=17,SuspendChange=18,OverCurrentChange=19,ResetChange=20}
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
 /// USB 2.0 hub port power bit.
 pub const fn powered(self)->bool{self.status&Self::POWER!=0}
 /// USB 2.0 speed flags; SuperSpeed uses speed_superspeed().
 pub const fn low_speed(self)->bool{self.status&Self::LOW_SPEED!=0}
 pub const fn high_speed(self)->bool{self.status&Self::HIGH_SPEED!=0}
 pub const fn has_change(self)->bool{self.change!=0}
 /// USB 2.0 port-speed interpretation. Do not use for SuperSpeed hubs.
 pub const fn speed_usb2(self)->Option<UsbSpeed>{
  if !self.connected(){None}else if self.low_speed(){Some(UsbSpeed::Low)}
  else if self.high_speed(){Some(UsbSpeed::High)}else{Some(UsbSpeed::Full)}
 }
 /// SuperSpeed port-speed field (wPortStatus bits 10..13).
 pub const fn speed_superspeed(self)->Option<UsbSpeed>{
  if !self.connected(){return None}
  match (self.status>>10)&0x0f{1=>Some(UsbSpeed::Full),2=>Some(UsbSpeed::Low),3=>Some(UsbSpeed::High),4=>Some(UsbSpeed::Super),5=>Some(UsbSpeed::SuperPlus),_=>None}
 }
 /// SuperSpeed port power bit (wPortStatus bit 9).
 pub const fn powered_superspeed(self)->bool{self.status&(1<<9)!=0}
 /// SuperSpeed port link state (wPortStatus bits 5..8).
 pub const fn superspeed_link_state(self)->u8{((self.status>>5)&0x0f)as u8}
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub enum HubPortAction{PowerOn,PowerOff,Reset,WarmReset,Enable,Suspend,Resume,WarmResume,ClearConnectionChange,ClearEnableChange,ClearSuspendChange,ClearOverCurrentChange,ClearResetChange}
impl HubPortAction{
 pub const fn feature(self)->u16{
  match self{
   Self::PowerOn|Self::PowerOff=>8,Self::Reset=>4,Self::WarmReset=>28,Self::Enable=>1,Self::Suspend|Self::Resume=>2,Self::WarmResume=>5,
   Self::ClearConnectionChange=>16,Self::ClearEnableChange=>17,Self::ClearSuspendChange=>18,
   Self::ClearOverCurrentChange=>19,Self::ClearResetChange=>20,
  }
 }
 pub const fn request(self)->u8{
  match self{
   Self::PowerOn|Self::Reset|Self::WarmReset|Self::WarmResume|Self::Enable|Self::Suspend=>HUB_SET_FEATURE,
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

#[cfg(test)]
mod descriptor_length_tests{
 use super::*;
 #[test]fn rejects_truncated_usb2_hub_bitmaps(){
  assert_eq!(HubDescriptor::parse(&[9,0x29,4,0,0,10,0]),Err(UsbError::BufferTooShort));
 }
 #[test]fn accepts_usb2_hub_descriptor_with_complete_bitmaps(){
  let d=HubDescriptor::parse(&[9,0x29,4,0,0,10,0,0,0]).unwrap();
  assert_eq!(d.ports,4);
 }
 #[test]fn accepts_superspeed_hub_descriptor_with_removable_bitmap(){
  let d=SuperSpeedHubDescriptor::parse(&[11,0x2a,4,0,0,10,0,1,0,0,0]).unwrap();
  assert_eq!(d.ports,4);assert_eq!(d.header_decode_latency,1);
 }
}


/// Decodes the interrupt-IN bitmap returned by a hub status endpoint.
/// Bit zero reports a hub-level change; bits 1..=N correspond to ports.
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct HubChangeBitmap{bits:[u32;4],pub port_count:u8}
impl HubChangeBitmap{
 pub fn parse(bytes:&[u8],port_count:u8)->Result<Self,UsbError>{
  if port_count==0||port_count>127{return Err(UsbError::MalformedDescriptor)}
  let required=((port_count as usize+1)+7)/8;
  if bytes.len()<required{return Err(UsbError::BufferTooShort)}
  let mut bits=[0u32;4];
  for (i,b) in bytes[..required].iter().enumerate(){
   let word=i/4;let shift=(i%4)*8;bits[word]|=(*b as u32)<<shift;
  }
  Ok(Self{bits,port_count})
 }
 pub const fn hub_changed(self)->bool{self.bits[0]&1!=0}
 pub fn port_changed(self,port:u8)->bool{
  if port==0||port>self.port_count{return false}
  let bit=port as usize;self.bits[bit/32]&(1u32<<(bit%32))!=0
 }
}
#[cfg(test)]
mod change_bitmap_tests{
 use super::*;
 #[test]fn maps_hub_and_port_change_bits(){
  let map=HubChangeBitmap::parse(&[0b0001_0101],4).unwrap();
  assert!(map.hub_changed());assert!(map.port_changed(2));assert!(map.port_changed(4));
  assert!(!map.port_changed(1));assert!(!map.port_changed(5));
 }
 #[test]fn requires_full_bitmap_for_all_ports(){
  assert_eq!(HubChangeBitmap::parse(&[0],8),Err(UsbError::BufferTooShort));
 }
}


/// Non-blocking downstream-port lifecycle policy. Timing and status polling are
/// supplied by the caller; no polling loop or MMIO is performed here.
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub enum HubPortState{Unpowered,PowerGoodWait,AwaitConnection,Debouncing,ResetRequired,Resetting,ClearResetChange,Ready,Disconnected,OverCurrent,OverCurrentLatched}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct HubPortMachine{pub port:u8,pub state:HubPortState,elapsed_ms:u32,power_good_ms:u32,superspeed:bool}
impl HubPortMachine{
 pub const fn new(port:u8,power_on_to_good_2ms:u8)->Self{Self{port,state:HubPortState::Unpowered,elapsed_ms:0,power_good_ms:power_on_to_good_2ms as u32*2,superspeed:false}}
 pub const fn new_superspeed(port:u8,power_on_to_good_2ms:u8)->Self{Self{port,state:HubPortState::Unpowered,elapsed_ms:0,power_good_ms:power_on_to_good_2ms as u32*2,superspeed:true}}
 pub fn start(&mut self)->Result<HubPortAction,UsbError>{if self.port==0||self.state!=HubPortState::Unpowered{return Err(UsbError::Unsupported)}self.state=HubPortState::PowerGoodWait;self.elapsed_ms=0;Ok(HubPortAction::PowerOn)}
 pub fn elapse_ms(&mut self,ms:u32){self.elapsed_ms=self.elapsed_ms.saturating_add(ms);if self.state==HubPortState::PowerGoodWait&&self.elapsed_ms>=self.power_good_ms{self.state=HubPortState::AwaitConnection;self.elapsed_ms=0}else if self.state==HubPortState::Debouncing&&self.elapsed_ms>=100{self.state=HubPortState::ResetRequired;self.elapsed_ms=0}}
 pub fn observe(&mut self,status:HubPortStatus){
  if status.status&HubPortStatus::OVER_CURRENT!=0{if self.state!=HubPortState::OverCurrentLatched{self.state=HubPortState::OverCurrent;}return}
  match self.state{
   HubPortState::AwaitConnection=>if status.connected(){self.state=HubPortState::Debouncing;self.elapsed_ms=0},
   HubPortState::Debouncing=>if !status.connected(){self.state=HubPortState::AwaitConnection;self.elapsed_ms=0},
   HubPortState::Resetting=>if !status.connected(){self.state=HubPortState::Disconnected}else if status.status&HubPortStatus::RESET==0&&status.enabled(){self.state=HubPortState::ClearResetChange},
   HubPortState::Disconnected=>if status.connected(){self.state=HubPortState::Debouncing;self.elapsed_ms=0},
   HubPortState::ClearResetChange=>if !status.connected(){self.state=HubPortState::Disconnected},
   HubPortState::Ready=>if !status.connected(){self.state=HubPortState::Disconnected}else if !status.enabled(){self.state=HubPortState::ResetRequired},
   _=>{}
  }
 }
 pub fn take_action(&mut self)->Option<HubPortAction>{match self.state{
  HubPortState::ResetRequired=>{self.state=HubPortState::Resetting;Some(if self.superspeed{HubPortAction::WarmReset}else{HubPortAction::Reset})},
  HubPortState::ClearResetChange=>{self.state=HubPortState::Ready;Some(HubPortAction::ClearResetChange)},
  HubPortState::OverCurrent=>{self.state=HubPortState::OverCurrentLatched;Some(HubPortAction::PowerOff)},_=>None}}
 /// Call when the corresponding class request failed so the action can be retried.
 pub fn action_failed(&mut self,action:HubPortAction){
  match action{
   HubPortAction::PowerOn if self.state==HubPortState::PowerGoodWait=>self.state=HubPortState::Unpowered,
   HubPortAction::Reset|HubPortAction::WarmReset if self.state==HubPortState::Resetting=>self.state=HubPortState::ResetRequired,
   HubPortAction::ClearResetChange if self.state==HubPortState::Ready=>self.state=HubPortState::ClearResetChange,
   HubPortAction::PowerOff if self.state==HubPortState::OverCurrentLatched=>self.state=HubPortState::OverCurrent,
   _=>{}
  }
 }
 pub const fn ready(&self)->bool{matches!(self.state,HubPortState::Ready)}
}
#[cfg(test)]mod port_machine_tests{
 use super::*;
 #[test]fn port_lifecycle_is_nonblocking(){
  let mut p=HubPortMachine::new(2,10);assert_eq!(p.start(),Ok(HubPortAction::PowerOn));
  p.elapse_ms(20);assert_eq!(p.state,HubPortState::AwaitConnection);
  p.observe(HubPortStatus{status:HubPortStatus::CONNECTION,change:0});p.elapse_ms(100);
  assert_eq!(p.take_action(),Some(HubPortAction::Reset));
  p.observe(HubPortStatus{status:HubPortStatus::CONNECTION|HubPortStatus::ENABLE,change:HubPortStatus::C_RESET});
  assert_eq!(p.take_action(),Some(HubPortAction::ClearResetChange));assert!(p.ready());
 }
 #[test]fn overcurrent_requests_power_off_once(){let mut p=HubPortMachine::new(1,0);p.start().unwrap();let status=HubPortStatus{status:HubPortStatus::OVER_CURRENT,change:0};p.observe(status);assert_eq!(p.take_action(),Some(HubPortAction::PowerOff));p.observe(status);assert_eq!(p.take_action(),None);}
}

#[cfg(test)]
mod superspeed_hub_reset_tests{
 use super::*;
 #[test]fn superspeed_port_machine_uses_warm_reset_feature(){
  let mut port=HubPortMachine::new_superspeed(1,0);
  port.start().unwrap();port.elapse_ms(0);
  port.observe(HubPortStatus{status:HubPortStatus::CONNECTION,change:0});
  port.elapse_ms(100);
  assert_eq!(port.take_action(),Some(HubPortAction::WarmReset));
  let request=port_action(1,HubPortAction::WarmReset);
  assert_eq!(request.request,HUB_SET_FEATURE);assert_eq!(request.value,28);
 }
}

#[cfg(test)]
mod hub_speed_semantics_tests{
 use super::*;
 #[test]fn decodes_usb2_and_superspeed_fields_separately(){
  let usb2=HubPortStatus{status:HubPortStatus::CONNECTION|HubPortStatus::HIGH_SPEED,change:0};
  assert_eq!(usb2.speed_usb2(),Some(UsbSpeed::High));
  let ss=HubPortStatus{status:HubPortStatus::CONNECTION|(4<<10)|(3<<5),change:0};
  assert_eq!(ss.speed_superspeed(),Some(UsbSpeed::Super));assert_eq!(ss.superspeed_link_state(),3);
 }
}

#[cfg(test)]
mod superspeed_resume_tests{
 use super::*;
 #[test]fn resumes_super_speed_port_by_selecting_u0(){
  let request=port_action(2,HubPortAction::WarmResume);
  assert_eq!(request.request,HUB_SET_FEATURE);assert_eq!(request.value,5);
  assert_eq!(request.index,2);
 }
}
