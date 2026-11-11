//! USB HID class support primitives for boot keyboards and mice.
use super::{UsbError,controller::{HostController,TransferStatus},descriptors::ParsedConfiguration,transfer::{DeviceAddress,Endpoint,EndpointAddress,SetupPacket,TransferType}};

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
 /// Locates this interface's HID class descriptor in the raw configuration blob.
 /// ParsedConfiguration intentionally stores the standard interface/endpoint
 /// tree; callers that need class-specific descriptors can retain this raw blob.
 pub fn find_in_configuration(
  configuration:&[u8],interface_number:u8,alternate_setting:u8,
 )->Result<Self,UsbError>{
  if configuration.len()<9||configuration[0]!=9||configuration[1]!=2{
   return Err(UsbError::MalformedDescriptor)
  }
  let total=u16::from_le_bytes([configuration[2],configuration[3]]) as usize;
  if total<9{return Err(UsbError::MalformedDescriptor)}
  if configuration.len()<total{return Err(UsbError::BufferTooShort)}
  let mut offset=9usize;
  let mut target=false;
  while offset<total{
   if total-offset<2{return Err(UsbError::MalformedDescriptor)}
   let length=configuration[offset] as usize;
   if length<2||length>total-offset{return Err(UsbError::MalformedDescriptor)}
   let descriptor_type=configuration[offset+1];
   if descriptor_type==4{
    if length<9{return Err(UsbError::MalformedDescriptor)}
    target=configuration[offset+2]==interface_number
     &&configuration[offset+3]==alternate_setting
     &&configuration[offset+5]==HID_CLASS;
   }else if target&&descriptor_type==HID_DESCRIPTOR_TYPE{
    return Self::parse(&configuration[offset..offset+length]);
   }
   offset+=length;
  }
  Err(UsbError::Unsupported)
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

/// Standard GET_DESCRIPTOR request for a HID Report descriptor.
pub const fn get_report_descriptor(interface:u8,length:u16)->SetupPacket{
 SetupPacket::new(0x81,6,(HID_REPORT_DESCRIPTOR_TYPE as u16)<<8,interface as u16,length)
}


/// Executes SET_PROTOCOL and validates the HID protocol selector.
pub fn execute_set_protocol<C:HostController>(
 controller:&mut C,address:DeviceAddress,interface:u8,protocol:u16,
)->Result<(),UsbError>{
 if protocol!=HID_PROTOCOL_BOOT&&protocol!=HID_PROTOCOL_REPORT{return Err(UsbError::Unsupported)}
 let setup=set_protocol(interface,protocol);
 super::control::execute_control(controller,address,setup,&mut[])?;
 Ok(())
}

/// Executes SET_IDLE for one report ID (zero selects all reports).
pub fn execute_set_idle<C:HostController>(
 controller:&mut C,address:DeviceAddress,interface:u8,duration:u8,report_id:u8,
)->Result<(),UsbError>{
 let setup=set_idle(interface,duration,report_id);
 super::control::execute_control(controller,address,setup,&mut[])?;
 Ok(())
}

/// Retrieves one HID Input, Output or Feature report. A short IN transfer is
/// returned as-is; the caller must validate its report layout and ID.
pub fn execute_get_report<C:HostController>(
 controller:&mut C,address:DeviceAddress,interface:u8,report_type:u8,
 report_id:u8,buffer:&mut[u8],
)->Result<usize,UsbError>{
 if !(1..=3).contains(&report_type){return Err(UsbError::Unsupported)}
 if buffer.is_empty()||buffer.len()>u16::MAX as usize{return Err(UsbError::BufferTooShort)}
 let setup=get_report(interface,report_type,report_id,buffer.len() as u16);
 let result=super::control::execute_control(controller,address,setup,buffer)?;
 Ok(result.bytes_transferred)
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
 #[test]fn locates_hid_class_descriptor_in_configuration_by_interface_and_alternate(){
  let configuration=[
   9,2,27,0,1,1,0,0x80,50,
   9,4,3,1,1,HID_CLASS,0,0,0,
   9,HID_DESCRIPTOR_TYPE,0x11,0x01,0,1,HID_REPORT_DESCRIPTOR_TYPE,63,0,
  ];
  let descriptor=HidDescriptor::find_in_configuration(&configuration,3,1).unwrap();
  assert_eq!(descriptor.report_length,63);
  assert_eq!(descriptor.hid_release,0x0111);
  assert_eq!(HidDescriptor::find_in_configuration(&configuration,3,0),Err(UsbError::Unsupported));
 }
 #[test]fn rejects_truncated_raw_configuration_hid_descriptor(){
  let configuration=[9,2,27,0,1,1,0,0x80,50,9,4,3,1,1,HID_CLASS,0,0,0,9,HID_DESCRIPTOR_TYPE];
  assert_eq!(HidDescriptor::find_in_configuration(&configuration,3,1),Err(UsbError::BufferTooShort));
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
impl BootHidInterface{
 pub const fn endpoint(self)->Endpoint{
  Endpoint{address:EndpointAddress::from_raw(self.interrupt_in_address),
   transfer_type:TransferType::Interrupt,max_packet_size:self.max_packet_size,
   interval:self.interval}
 }
 /// Reads one bounded raw report. Input owns semantic decoding and event publication.
 pub fn read_report<C:HostController>(
  self,controller:&mut C,address:DeviceAddress,buffer:&mut[u8],
 )->Result<usize,UsbError>{
  if buffer.is_empty()||buffer.len()>self.max_packet_size as usize{return Err(UsbError::BufferTooShort)}
  let result=controller.interrupt_transfer(address,self.endpoint(),buffer)?;
  match result.status{
   TransferStatus::Success=>{},
   TransferStatus::TimedOut=>return Err(UsbError::Timeout),
   TransferStatus::NoDevice=>return Err(UsbError::ControllerUnavailable),
   TransferStatus::Stall=>return Err(UsbError::Unsupported),
   TransferStatus::Pending|TransferStatus::ControllerError=>return Err(UsbError::TransferFailed),
  }
  if result.bytes_transferred>buffer.len(){return Err(UsbError::TransferFailed)}
  Ok(result.bytes_transferred)
 }
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
  if d.class!=HID_CLASS||d.subclass!=HID_SUBCLASS_BOOT||d.protocol!=protocol{continue}
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
 fn poll<C:HostController,const N:usize>(&self,controller:&mut C,data:&mut[u8;N])->Result<usize,UsbError>{
  self.interface.read_report(controller,self.device,data)
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


// Generic HID report-descriptor accounting. It deliberately parses item
// structure rather than assuming boot-keyboard layouts, and uses fixed storage.
pub const MAX_HID_REPORT_IDS:usize=16;
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct HidReportLayout{pub report_id:u8,pub input_bits:u32,pub output_bits:u32,pub feature_bits:u32}
impl HidReportLayout{
 pub const EMPTY:Self=Self{report_id:0,input_bits:0,output_bits:0,feature_bits:0};
 pub const fn input_bytes(self)->u32{self.input_bits.saturating_add(7)/8+(self.report_id!=0)as u32}
 pub const fn output_bytes(self)->u32{self.output_bits.saturating_add(7)/8+(self.report_id!=0)as u32}
 pub const fn feature_bytes(self)->u32{self.feature_bits.saturating_add(7)/8+(self.report_id!=0)as u32}
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
struct HidGlobalState{report_size:u32,report_count:u32,report_id:u8}
impl HidGlobalState{const EMPTY:Self=Self{report_size:0,report_count:0,report_id:0};}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct HidReportDescriptor{reports:[Option<HidReportLayout>;MAX_HID_REPORT_IDS],len:u8}
impl HidReportDescriptor{
 pub fn parse(bytes:&[u8])->Result<Self,UsbError>{
  let mut out=Self{reports:[None;MAX_HID_REPORT_IDS],len:0};
  let mut state=HidGlobalState::EMPTY;
  let mut stack=[HidGlobalState::EMPTY;4];let mut depth=0usize;let mut offset=0usize;
  let mut uses_unnumbered=false;let mut uses_numbered=false;
  while offset<bytes.len(){
   let prefix=bytes[offset];offset+=1;
   if prefix==0xfe{
    if bytes.len().saturating_sub(offset)<2{return Err(UsbError::MalformedDescriptor)}
    let size=bytes[offset] as usize;offset+=2;
    if size>bytes.len().saturating_sub(offset){return Err(UsbError::MalformedDescriptor)}
    offset+=size;continue
   }
   let size=match prefix&3{0=>0,1=>1,2=>2,_=>4};
   if size>bytes.len().saturating_sub(offset){return Err(UsbError::MalformedDescriptor)}
   let mut value=0u32;
   for i in 0..size{value|=(bytes[offset+i] as u32)<<(i*8);}
   offset+=size;
   let item_type=(prefix>>2)&3;let tag=prefix>>4;
   if item_type==1{
    match tag{
     7=>state.report_size=value,
     8=>{
      if value==0||value>255{return Err(UsbError::MalformedDescriptor)}
      state.report_id=value as u8;
     }
     9=>state.report_count=value,
     10=>{
      if depth>=stack.len(){return Err(UsbError::NoResources)}
      stack[depth]=state;depth+=1;
     }
     11=>{
      if depth==0{return Err(UsbError::MalformedDescriptor)}
      depth-=1;state=stack[depth];
     }
     _=>{}
    }
   }
   if item_type==0&&(tag==8||tag==9||tag==11){
    // HID report streams use either implicit report ID zero or explicit
    // non-zero IDs; mixing the two makes the wire layout ambiguous.
    if state.report_id==0{
     if uses_numbered{return Err(UsbError::MalformedDescriptor)}
     uses_unnumbered=true;
    }else{
     if uses_unnumbered{return Err(UsbError::MalformedDescriptor)}
     uses_numbered=true;
    }
    if state.report_size==0||state.report_count==0{return Err(UsbError::MalformedDescriptor)}
    let bits=state.report_size.checked_mul(state.report_count).ok_or(UsbError::NoResources)?;
    let index=out.find_or_insert(state.report_id)?;
    let layout=out.reports[index].as_mut().ok_or(UsbError::MalformedDescriptor)?;
    let target=match tag{8=>&mut layout.input_bits,9=>&mut layout.output_bits,_=>&mut layout.feature_bits};
    *target=target.checked_add(bits).ok_or(UsbError::NoResources)?;
   }
  }
  if depth!=0||out.len==0{return Err(UsbError::MalformedDescriptor)}
  Ok(out)
 }
 fn find_or_insert(&mut self,id:u8)->Result<usize,UsbError>{
  for i in 0..self.len as usize{if self.reports[i].map(|r|r.report_id)==Some(id){return Ok(i)}}
  if self.len as usize>=MAX_HID_REPORT_IDS{return Err(UsbError::NoResources)}
  let i=self.len as usize;self.reports[i]=Some(HidReportLayout{report_id:id,..HidReportLayout::EMPTY});self.len+=1;Ok(i)
 }
 pub fn report(&self,id:u8)->Option<HidReportLayout>{
  self.reports[..self.len as usize].iter().flatten().find(|r|r.report_id==id).copied()
 }
 pub fn iter(&self)->impl Iterator<Item=HidReportLayout>+'_{
  self.reports[..self.len as usize].iter().filter_map(|r|*r)
 }
 pub const fn len(&self)->usize{self.len as usize}
}

#[cfg(test)]
mod report_descriptor_tests{
 use super::*;
 #[test]fn accounts_input_output_and_feature_reports(){
  // Report Size=8, Report Count=3, then Input; Count=2, then Output.
  let descriptor=[0x75,8,0x95,3,0x81,2,0x95,2,0x91,2];
  let parsed=HidReportDescriptor::parse(&descriptor).unwrap();
  let report=parsed.report(0).unwrap();
  assert_eq!(report.input_bits,24);assert_eq!(report.output_bits,16);
  assert_eq!(report.input_bytes(),3);assert_eq!(report.output_bytes(),2);
 }
 #[test]fn separates_report_ids_and_includes_id_prefix_byte(){
  let descriptor=[0x85,1,0x75,8,0x95,2,0x81,2,0x85,2,0x95,4,0x91,2];
  let parsed=HidReportDescriptor::parse(&descriptor).unwrap();
  assert_eq!(parsed.report(1).unwrap().input_bytes(),3);
  assert_eq!(parsed.report(2).unwrap().output_bytes(),5);
 }
 #[test]fn rejects_mixing_implicit_and_explicit_report_ids(){
  let mixed=[0x75,8,0x95,1,0x81,2,0x85,1,0x81,2];
  assert_eq!(HidReportDescriptor::parse(&mixed),Err(UsbError::MalformedDescriptor));
 }
 #[test]fn validates_global_push_pop_and_malformed_streams(){
  let descriptor=[0x75,8,0xa4,0x95,2,0xb4,0x95,1,0x81,2];
  assert_eq!(HidReportDescriptor::parse(&descriptor).unwrap().report(0).unwrap().input_bits,8);
  assert_eq!(HidReportDescriptor::parse(&[0xb4]),Err(UsbError::MalformedDescriptor));
  assert_eq!(HidReportDescriptor::parse(&[0x75]),Err(UsbError::MalformedDescriptor));
 }
}

#[cfg(test)]
mod input_handoff_tests{
 use super::*;
 #[test]fn builds_report_descriptor_request_for_selected_interface(){
  let request=get_report_descriptor(5,128);
  assert_eq!(request.request_type,0x81);assert_eq!(request.request,6);
  assert_eq!(request.value,0x2200);assert_eq!(request.index,5);
  assert_eq!(request.length,128);
 }
 #[test]fn boot_interface_exposes_normalized_interrupt_endpoint(){
  let interface=BootHidInterface{interface_number:3,alternate_setting:1,kind:BootHidKind::Mouse,
   interrupt_in_address:0x83,max_packet_size:16,interval:4};
  let endpoint=interface.endpoint();
  assert_eq!(endpoint.address.raw(),0x83);assert!(endpoint.address.is_in());
  assert_eq!(endpoint.address.number(),3);assert_eq!(endpoint.transfer_type,TransferType::Interrupt);
  assert_eq!(endpoint.max_packet_size,16);assert_eq!(endpoint.interval,4);
 }
}


#[cfg(test)]
mod class_request_execution_tests{
 use super::*;
 use crate::drivers::usb::controller::{ControllerKind,PortStatus,TransferResult};
 use crate::drivers::usb::transfer::{UsbSpeed};
 struct MockHost{last:Option<SetupPacket>,response:[u8;8],length:usize}
 impl HostController for MockHost{
  fn kind(&self)->ControllerKind{ControllerKind::Xhci}
  fn port_count(&self)->u8{1}
  fn port_status(&self,_:u8)->Result<PortStatus,UsbError>{Ok(PortStatus{connected:true,enabled:true,suspended:false,over_current:false,speed:Some(UsbSpeed::High)})}
  fn control_transfer(&mut self,_:DeviceAddress,setup:SetupPacket,data:&mut[u8])->Result<TransferResult,UsbError>{
   self.last=Some(setup);
   if setup.request_type&0x80!=0{
    let n=self.length.min(data.len());data[..n].copy_from_slice(&self.response[..n]);
    Ok(TransferResult{status:TransferStatus::Success,bytes_transferred:n})
   }else{Ok(TransferResult{status:TransferStatus::Success,bytes_transferred:0})}
  }
  fn bulk_transfer(&mut self,_:DeviceAddress,_:Endpoint,_:&mut[u8])->Result<TransferResult,UsbError>{Err(UsbError::Unsupported)}
  fn interrupt_transfer(&mut self,_:DeviceAddress,_:Endpoint,_:&mut[u8])->Result<TransferResult,UsbError>{Err(UsbError::Unsupported)}
  fn reset_port(&mut self,_:u8)->Result<(),UsbError>{Ok(())}
 }
 fn host()->MockHost{MockHost{last:None,response:[1,2,3,4,0,0,0,0],length:4}}
 #[test]fn executes_set_protocol_and_idle_without_data_stage(){
  let mut h=host();let a=DeviceAddress::new(7).unwrap();
  execute_set_protocol(&mut h,a,3,HID_PROTOCOL_BOOT).unwrap();
  let p=h.last.unwrap();assert_eq!(p.request_type,0x21);assert_eq!(p.request,HID_SET_PROTOCOL);assert_eq!(p.index,3);assert_eq!(p.value,0);
  execute_set_idle(&mut h,a,3,20,2).unwrap();
  let p=h.last.unwrap();assert_eq!(p.request,HID_SET_IDLE);assert_eq!(p.value,0x1402);
  assert_eq!(execute_set_protocol(&mut h,a,3,2),Err(UsbError::Unsupported));
 }
 #[test]fn executes_get_report_and_returns_actual_short_in_length(){
  let mut h=host();let mut data=[0u8;8];
  assert_eq!(execute_get_report(&mut h,DeviceAddress::new(2).unwrap(),4,1,7,&mut data),Ok(4));
  assert_eq!(&data[..4],&[1,2,3,4]);
  let p=h.last.unwrap();assert_eq!(p.request_type,0xa1);assert_eq!(p.request,HID_GET_REPORT);
  assert_eq!(p.value,0x0107);assert_eq!(p.index,4);assert_eq!(p.length,8);
  assert_eq!(execute_get_report(&mut h,DeviceAddress::new(2).unwrap(),4,0,0,&mut data),Err(UsbError::Unsupported));
 }
}
