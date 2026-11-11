//! USB HID class support primitives for boot keyboards and mice.
use super::{UsbError,descriptors::ParsedConfiguration,transfer::{SetupPacket,TransferType}};

pub const HID_CLASS:u8=3;
pub const HID_SUBCLASS_BOOT:u8=1;
pub const HID_PROTOCOL_KEYBOARD:u8=1;
pub const HID_PROTOCOL_MOUSE:u8=2;
pub const HID_DESCRIPTOR_TYPE:u8=0x21;
pub const HID_REPORT_DESCRIPTOR_TYPE:u8=0x22;
pub const HID_SET_PROTOCOL:u8=0x0b;
pub const HID_SET_IDLE:u8=0x0a;
pub const HID_GET_REPORT:u8=0x01;
pub const HID_PROTOCOL_BOOT:u16=0;
pub const HID_PROTOCOL_REPORT:u16=1;

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct HidDescriptor{pub hid_release:u16,pub country_code:u8,pub subordinate_count:u8,pub report_length:u16}
impl HidDescriptor{
 pub fn parse(bytes:&[u8])->Result<Self,UsbError>{
  if bytes.len()<9||bytes[0]<9||bytes[1]!=HID_DESCRIPTOR_TYPE{return Err(UsbError::MalformedDescriptor)}
  if bytes[0] as usize>bytes.len(){return Err(UsbError::BufferTooShort)}
  let count=bytes[5] as usize;
  if count==0||(bytes[0] as usize)<6+3*count||bytes.len()<6+3*count{return Err(UsbError::MalformedDescriptor)}
  let mut report_length=0;
  for i in 0..count{
   let off=6+i*3;
   if bytes[off]==HID_REPORT_DESCRIPTOR_TYPE{
    report_length=u16::from_le_bytes([bytes[off+1],bytes[off+2]]);
    break;
   }
  }
  if report_length==0{return Err(UsbError::MalformedDescriptor)}
  Ok(Self{hid_release:u16::from_le_bytes([bytes[2],bytes[3]]),country_code:bytes[4],subordinate_count:count as u8,report_length})
 }
}
pub const fn set_protocol(interface:u8,protocol:u16)->SetupPacket{
 SetupPacket::new(0x21,HID_SET_PROTOCOL,protocol,interface as u16,0)
}
pub const fn set_idle(interface:u8,duration:u8,report_id:u8)->SetupPacket{
 SetupPacket::new(0x21,HID_SET_IDLE,((duration as u16)<<8)|report_id as u16,interface as u16,0)
}
pub const fn get_report(interface:u8,report_type:u8,report_id:u8,length:u16)->SetupPacket{
 SetupPacket::new(0xa1,HID_GET_REPORT,((report_type as u16)<<8)|report_id as u16,interface as u16,length)
}

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct BootKeyboardReport{pub modifiers:u8,pub keys:[u8;6]}
impl BootKeyboardReport{
 pub const LENGTH:usize=8;
 pub fn parse(bytes:&[u8])->Result<Self,UsbError>{
  if bytes.len()<Self::LENGTH{return Err(UsbError::BufferTooShort)}
  if bytes[2..8].iter().any(|key|*key==1){return Err(UsbError::MalformedDescriptor)}
  let mut keys=[0;6];keys.copy_from_slice(&bytes[2..8]);
  Ok(Self{modifiers:bytes[0],keys})
 }
 pub const fn modifier_down(self,bit:u8)->bool{bit<8&&self.modifiers&(1<<bit)!=0}
 pub fn key_down(self,usage:u8)->bool{usage!=0&&self.keys.contains(&usage)}
 pub fn pressed_since(self,previous:Self,output:&mut[u8;6])->usize{
  let mut n=0;
  for key in self.keys{
   if key!=0&&!previous.keys.contains(&key)&&n<output.len(){output[n]=key;n+=1;}
  }
  n
 }
 pub fn released_since(self,previous:Self,output:&mut[u8;6])->usize{
  let mut n=0;
  for key in previous.keys{
   if key!=0&&!self.keys.contains(&key)&&n<output.len(){output[n]=key;n+=1;}
  }
  n
 }
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct BootMouseReport{pub buttons:u8,pub x:i8,pub y:i8}
impl BootMouseReport{
 pub const LENGTH:usize=3;
 pub fn parse(bytes:&[u8])->Result<Self,UsbError>{
  if bytes.len()<Self::LENGTH{return Err(UsbError::BufferTooShort)}
  Ok(Self{buttons:bytes[0],x:bytes[1] as i8,y:bytes[2] as i8})
 }
 pub const fn button_down(self,button:u8)->bool{button>=1&&button<=8&&self.buttons&(1<<(button-1))!=0}
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct BootMouseReportWheel{pub base:BootMouseReport,pub wheel:i8}
impl BootMouseReportWheel{
 pub fn parse(bytes:&[u8])->Result<Self,UsbError>{
  if bytes.len()<4{return Err(UsbError::BufferTooShort)}
  Ok(Self{base:BootMouseReport::parse(bytes)?,wheel:bytes[3] as i8})
 }
}

#[cfg(test)]
mod tests{
 use super::*;
 #[test]fn parses_hid_report_descriptor_reference(){
  let d=HidDescriptor::parse(&[9,0x21,0x11,0x01,0,1,0x22,63,0]).unwrap();
  assert_eq!(d.hid_release,0x0111);assert_eq!(d.report_length,63);
 }
 #[test]fn builds_boot_protocol_request(){
  let p=set_protocol(2,HID_PROTOCOL_BOOT);
  assert_eq!(p.request_type,0x21);assert_eq!(p.request,HID_SET_PROTOCOL);
  assert_eq!(p.index,2);assert_eq!(p.value,0);
 }
 #[test]fn detects_keyboard_key_edges(){
  let old=BootKeyboardReport::parse(&[0,0,4,0,0,0,0,0]).unwrap();
  let new=BootKeyboardReport::parse(&[2,0,4,5,0,0,0,0]).unwrap();
  let mut keys=[0;6];assert_eq!(new.pressed_since(old,&mut keys),1);assert_eq!(keys[0],5);
  assert_eq!(new.released_since(old,&mut keys),0);assert!(new.modifier_down(1));
 }
 #[test]fn decodes_signed_mouse_motion(){
  let m=BootMouseReport::parse(&[1,0xfe,3]).unwrap();
  assert!(m.button_down(1));assert_eq!(m.x,-2);assert_eq!(m.y,3);
 }
 #[test]fn rejects_keyboard_rollover_usage(){
  assert_eq!(BootKeyboardReport::parse(&[0,0,1,0,0,0,0,0]),Err(UsbError::MalformedDescriptor));
 }
}


#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub enum BootHidKind{Keyboard,Mouse}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct BootHidInterface{
 pub interface_number:u8,pub alternate_setting:u8,pub kind:BootHidKind,
 pub interrupt_in_address:u8,pub max_packet_size:u16,pub interval:u8,
}
/// Finds a boot-protocol HID interface and its interrupt-IN endpoint in a
/// parsed configuration. It does not change the active alternate setting.
pub fn find_boot_interface(config:&ParsedConfiguration,kind:BootHidKind)->Option<BootHidInterface>{
 let (protocol,expected_packet)=match kind{
  BootHidKind::Keyboard=>(HID_PROTOCOL_KEYBOARD,8),
  BootHidKind::Mouse=>(HID_PROTOCOL_MOUSE,3),
 };
 for iface in config.interfaces[..config.interface_len as usize].iter().flatten(){
  let d=iface.descriptor;
  if d.class!=HID_CLASS||d.subclass!=HID_SUBCLASS_BOOT||d.protocol!=protocol{return continue}
  let start=iface.first_endpoint as usize;
  let end=start+iface.endpoint_count as usize;
  for endpoint in config.endpoints[start..end].iter().flatten(){
   if endpoint.address&0x80!=0&&(endpoint.attributes&3)==TransferType::Interrupt as u8
    &&endpoint.max_packet_size>=expected_packet{
    return Some(BootHidInterface{interface_number:d.number,alternate_setting:d.alternate_setting,
     kind,interrupt_in_address:endpoint.address,max_packet_size:endpoint.max_packet_size,
     interval:endpoint.interval});
   }
  }
 }
 None
}


#[cfg(test)]
mod interface_tests{
 use super::*;
 use super::super::descriptors::{ConfigurationDescriptor,InterfaceDescriptor,ParsedEndpoint,ParsedInterface,MAX_ENDPOINTS,MAX_INTERFACES};
 #[test]fn locates_boot_keyboard_interrupt_in_endpoint(){
  let mut config=ParsedConfiguration{
   descriptor:ConfigurationDescriptor{total_length:25,interface_count:1,configuration_value:1,configuration_index:0,attributes:0x80,max_power:50},
   interfaces:[None;MAX_INTERFACES],endpoints:[None;MAX_ENDPOINTS],interface_len:1,endpoint_len:1,
  };
  config.interfaces[0]=Some(ParsedInterface{descriptor:InterfaceDescriptor{number:2,alternate_setting:0,endpoint_count:1,class:HID_CLASS,subclass:HID_SUBCLASS_BOOT,protocol:HID_PROTOCOL_KEYBOARD,string_index:0},first_endpoint:0,endpoint_count:1});
  config.endpoints[0]=Some(ParsedEndpoint{address:0x81,attributes:3,max_packet_size:8,interval:10,transactions_per_microframe:1,max_burst:0,companion_attributes:0,bytes_per_interval:0,has_ss_companion:false});
  let found=find_boot_interface(&config,BootHidKind::Keyboard).unwrap();
  assert_eq!(found.interface_number,2);assert_eq!(found.interrupt_in_address,0x81);
  assert_eq!(found.max_packet_size,8);
 }
 #[test]fn rejects_keyboard_without_interrupt_in_endpoint(){
  let mut config=ParsedConfiguration{
   descriptor:ConfigurationDescriptor{total_length:25,interface_count:1,configuration_value:1,configuration_index:0,attributes:0x80,max_power:50},
   interfaces:[None;MAX_INTERFACES],endpoints:[None;MAX_ENDPOINTS],interface_len:1,endpoint_len:1,
  };
  config.interfaces[0]=Some(ParsedInterface{descriptor:InterfaceDescriptor{number:0,alternate_setting:0,endpoint_count:1,class:HID_CLASS,subclass:HID_SUBCLASS_BOOT,protocol:HID_PROTOCOL_KEYBOARD,string_index:0},first_endpoint:0,endpoint_count:1});
  config.endpoints[0]=Some(ParsedEndpoint{address:0x01,attributes:3,max_packet_size:8,interval:10,transactions_per_microframe:1,max_burst:0,companion_attributes:0,bytes_per_interval:0,has_ss_companion:false});
  assert_eq!(find_boot_interface(&config,BootHidKind::Keyboard),None);
 }
}


use super::controller::{HostController,TransferStatus};
use super::transfer::{DeviceAddress,Endpoint,EndpointAddress,TransferType};

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct KeyboardChanges{
 pub report:BootKeyboardReport,pub pressed:[u8;6],pub pressed_count:u8,
 pub released:[u8;6],pub released_count:u8,
 pub modifiers_pressed:u8,pub modifiers_released:u8,
}
pub struct BootHidPoller{device:DeviceAddress,pub interface:BootHidInterface,previous_keyboard:Option<BootKeyboardReport>}
impl BootHidPoller{
 pub const fn new(device:DeviceAddress,interface:BootHidInterface)->Self{
  Self{device,interface,previous_keyboard:None}
 }
 fn endpoint(&self)->Endpoint{
  Endpoint{address:EndpointAddress::from_raw(self.interface.interrupt_in_address),
   transfer_type:TransferType::Interrupt,max_packet_size:self.interface.max_packet_size,
   interval:self.interface.interval}
 }
 fn poll<C:HostController,const N:usize>(&self,controller:&mut C,data:&mut[u8;N])->Result<usize,UsbError>{
  let result=controller.interrupt_transfer(self.device,self.endpoint(),data)?;
  match result.status{
   TransferStatus::Success=>{},
   TransferStatus::TimedOut=>return Err(UsbError::Timeout),
   TransferStatus::NoDevice=>return Err(UsbError::ControllerUnavailable),
   TransferStatus::Stall=>return Err(UsbError::Unsupported),
   TransferStatus::Pending|TransferStatus::ControllerError=>return Err(UsbError::TransferFailed),
  }
  if result.bytes_transferred>N||result.bytes_transferred>self.interface.max_packet_size as usize{
   return Err(UsbError::TransferFailed)
  }
  Ok(result.bytes_transferred)
 }
 pub fn poll_keyboard<C:HostController>(&mut self,controller:&mut C)->Result<KeyboardChanges,UsbError>{
  if self.interface.kind!=BootHidKind::Keyboard{return Err(UsbError::Unsupported)}
  let mut bytes=[0u8;8];let count=self.poll(controller,&mut bytes)?;
  if count<BootKeyboardReport::LENGTH{return Err(UsbError::BufferTooShort)}
  let report=BootKeyboardReport::parse(&bytes[..count])?;
  let previous=self.previous_keyboard.unwrap_or(BootKeyboardReport{modifiers:0,keys:[0;6]});
  let mut pressed=[0;6];let mut released=[0;6];
  let pressed_count=report.pressed_since(previous,&mut pressed) as u8;
  let released_count=report.released_since(previous,&mut released) as u8;
  let changes=KeyboardChanges{report,pressed,pressed_count,released,released_count,
   modifiers_pressed:report.modifiers&!previous.modifiers,
   modifiers_released:previous.modifiers&!report.modifiers};
  self.previous_keyboard=Some(report);Ok(changes)
 }
 pub fn poll_mouse<C:HostController>(&self,controller:&mut C)->Result<BootMouseReport,UsbError>{
  if self.interface.kind!=BootHidKind::Mouse{return Err(UsbError::Unsupported)}
  let mut bytes=[0u8;8];let count=self.poll(controller,&mut bytes)?;
  if count<BootMouseReport::LENGTH{return Err(UsbError::BufferTooShort)}
  BootMouseReport::parse(&bytes[..count])
 }
}
