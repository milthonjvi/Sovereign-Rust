//! Generic HID interface discovery and transport metadata.
//!
//! Semantic report decoding and InputEvent publication belong to
//! drivers::input. This module only locates HID interfaces/endpoints in the
//! already validated USB configuration tree.
use super::{
 UsbError,
 descriptors::{ParsedConfiguration,MAX_ENDPOINTS},
 controller::{HostController,TransferStatus},
 transfer::{DeviceAddress,Endpoint,EndpointAddress,TransferType},
};

/// Executes one interrupt-IN report transaction without interpreting its payload.
/// The caller (Input) owns report state, decoding, scheduling cadence and event queues.
pub fn read_interrupt_report<C:HostController>(
 controller:&mut C,address:DeviceAddress,endpoint:Endpoint,buffer:&mut[u8],
)->Result<usize,UsbError>{
 if endpoint.transfer_type!=TransferType::Interrupt||!endpoint.address.is_in()
  ||endpoint.address.number()==0||endpoint.max_packet_size==0{
  return Err(UsbError::MalformedDescriptor)
 }
 if buffer.is_empty(){return Err(UsbError::BufferTooShort)}
 let result=controller.interrupt_transfer(address,endpoint,buffer)?;
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

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct HidInterfaceInfo{
 pub interface_number:u8,
 pub alternate_setting:u8,
 pub subclass:u8,
 pub protocol:u8,
 pub first_endpoint:u8,
 pub endpoint_count:u8,
}
impl HidInterfaceInfo{
 /// Returns the Nth interrupt-IN endpoint belonging to this interface.
 /// Multiple endpoints are retained instead of silently discarding all but one.
 pub fn interrupt_in_endpoint(
  self,configuration:&ParsedConfiguration,ordinal:usize,
 )->Option<Endpoint>{
  let start=self.first_endpoint as usize;
  let end=start.checked_add(self.endpoint_count as usize)?;
  if end>configuration.endpoint_len as usize||end>MAX_ENDPOINTS{return None}
  let mut found=0usize;
  for parsed in configuration.endpoints[start..end].iter().flatten(){
   if parsed.address&0x80==0||(parsed.attributes&3)!=TransferType::Interrupt as u8{continue}
   if found==ordinal{
    return Some(Endpoint{
     address:EndpointAddress::from_raw(parsed.address),
     transfer_type:TransferType::Interrupt,
     max_packet_size:parsed.max_packet_size,
     interval:parsed.interval,
    })
   }
   found+=1;
  }
  None
 }
 /// Reads a raw report from the selected interrupt-IN endpoint.
 pub fn read_report<C:HostController>(
  self,configuration:&ParsedConfiguration,ordinal:usize,controller:&mut C,
  address:DeviceAddress,buffer:&mut[u8],
 )->Result<usize,UsbError>{
  let endpoint=self.interrupt_in_endpoint(configuration,ordinal).ok_or(UsbError::Unsupported)?;
  read_interrupt_report(controller,address,endpoint,buffer)
 }
 /// Counts interrupt-IN endpoints without allocating.
 pub fn interrupt_in_count(self,configuration:&ParsedConfiguration)->usize{
  let start=self.first_endpoint as usize;
  let Some(end)=start.checked_add(self.endpoint_count as usize)else{return 0};
  if end>configuration.endpoint_len as usize||end>MAX_ENDPOINTS{return 0}
  configuration.endpoints[start..end].iter().flatten()
   .filter(|e|e.address&0x80!=0&&(e.attributes&3)==TransferType::Interrupt as u8)
   .count()
 }
}

/// Periodic endpoint timing expressed without losing the USB speed semantics.
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub enum HidServiceInterval{
 /// Full/Low Speed interrupt endpoints are scheduled in 1 ms frames.
 Frames(u8),
 /// High/SuperSpeed intervals are powers of two in 125 us microframes.
 Microframes(u16),
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct HidInterruptEndpoint{
 pub endpoint:Endpoint,
 pub speed:super::transfer::UsbSpeed,
 pub service_interval:HidServiceInterval,
 /// Transactions per service opportunity (High Speed high-bandwidth endpoints).
 pub transactions_per_interval:u8,
 /// SuperSpeed companion burst count, as encoded by bMaxBurst (zero for USB 2).
 pub max_burst:u8,
 /// Maximum aggregate payload per service interval when supplied by SS companion.
 pub bytes_per_interval:u16,
}
impl HidInterruptEndpoint{
 pub fn from_parsed(parsed:super::descriptors::ParsedEndpoint,speed:super::transfer::UsbSpeed)->Result<Self,UsbError>{
  if parsed.address&0x80==0||(parsed.attributes&3)!=TransferType::Interrupt as u8
   ||parsed.max_packet_size==0||parsed.interval==0{
   return Err(UsbError::MalformedDescriptor)
  }
  let service_interval=match speed{
   super::transfer::UsbSpeed::Low|super::transfer::UsbSpeed::Full=>{
    if parsed.interval<1{return Err(UsbError::MalformedDescriptor)}
    HidServiceInterval::Frames(parsed.interval)
   }
   super::transfer::UsbSpeed::High|super::transfer::UsbSpeed::Super|super::transfer::UsbSpeed::SuperPlus=>{
    if parsed.interval>16{return Err(UsbError::MalformedDescriptor)}
    HidServiceInterval::Microframes(1u16<<(parsed.interval-1))
   }
  };
  let transactions=if speed==super::transfer::UsbSpeed::High{
   parsed.transactions_per_microframe
  }else{1};
  if transactions==0||transactions>3{return Err(UsbError::MalformedDescriptor)}
  if (speed==super::transfer::UsbSpeed::Super||speed==super::transfer::UsbSpeed::SuperPlus)
   &&!parsed.has_ss_companion{return Err(UsbError::MalformedDescriptor)}
  let bytes=if parsed.has_ss_companion{parsed.bytes_per_interval}else{
   parsed.max_packet_size.saturating_mul(transactions as u16)
  };
  if bytes==0{return Err(UsbError::MalformedDescriptor)}
  Ok(Self{
   endpoint:Endpoint{address:EndpointAddress::from_raw(parsed.address),
    transfer_type:TransferType::Interrupt,max_packet_size:parsed.max_packet_size,interval:parsed.interval},
   speed,service_interval,transactions_per_interval:transactions,
   max_burst:parsed.max_burst,bytes_per_interval:bytes,
  })
 }
}

/// Finds one HID interface alternate setting, including non-boot devices.
pub fn find_interface(
 configuration:&ParsedConfiguration,number:u8,alternate:u8,
)->Result<Option<HidInterfaceInfo>,UsbError>{
 if configuration.interface_len as usize>configuration.interfaces.len()
  ||configuration.endpoint_len as usize>configuration.endpoints.len(){
  return Err(UsbError::MalformedDescriptor)
 }
 for interface in configuration.interfaces[..configuration.interface_len as usize].iter().flatten(){
  let descriptor=interface.descriptor;
  if descriptor.class!=3||descriptor.number!=number||descriptor.alternate_setting!=alternate{continue}
  let start=interface.first_endpoint as usize;
  let end=start.checked_add(interface.endpoint_count as usize).ok_or(UsbError::MalformedDescriptor)?;
  if end>configuration.endpoint_len as usize||end>MAX_ENDPOINTS{
   return Err(UsbError::MalformedDescriptor)
  }
  return Ok(Some(HidInterfaceInfo{
   interface_number:descriptor.number,
   alternate_setting:descriptor.alternate_setting,
   subclass:descriptor.subclass,
   protocol:descriptor.protocol,
   first_endpoint:interface.first_endpoint,
   endpoint_count:interface.endpoint_count,
  }))
 }
 Ok(None)
}

#[cfg(test)]
mod tests{
 use super::*;
 use crate::drivers::usb::descriptors::{
  ConfigurationDescriptor,InterfaceDescriptor,ParsedEndpoint,ParsedInterface,
  MAX_INTERFACES,
 };
 fn fixture()->ParsedConfiguration{
  let mut config=ParsedConfiguration{
   descriptor:ConfigurationDescriptor{total_length:32,interface_count:1,configuration_value:1,
    configuration_index:0,attributes:0x80,max_power:50},
   interfaces:[None;MAX_INTERFACES],endpoints:[None;MAX_ENDPOINTS],
   interface_len:1,endpoint_len:2,
  };
  config.interfaces[0]=Some(ParsedInterface{
   descriptor:InterfaceDescriptor{number:4,alternate_setting:2,endpoint_count:2,
    class:3,subclass:0,protocol:0,string_index:0},
   first_endpoint:0,endpoint_count:2,
  });
  config.endpoints[0]=Some(ParsedEndpoint{address:0x81,attributes:3,max_packet_size:32,
   interval:4,transactions_per_microframe:1,max_burst:0,companion_attributes:0,
   bytes_per_interval:0,has_ss_companion:false});
  config.endpoints[1]=Some(ParsedEndpoint{address:0x02,attributes:2,max_packet_size:64,
   interval:0,transactions_per_microframe:1,max_burst:0,companion_attributes:0,
   bytes_per_interval:0,has_ss_companion:false});
  config
 }
 #[test]fn discovers_report_protocol_hid_without_boot_assumptions(){
  let config=fixture();
  let info=find_interface(&config,4,2).unwrap().unwrap();
  assert_eq!(info.subclass,0);assert_eq!(info.protocol,0);
  assert_eq!(info.interrupt_in_count(&config),1);
  let endpoint=info.interrupt_in_endpoint(&config,0).unwrap();
  assert_eq!(endpoint.address.raw(),0x81);assert_eq!(endpoint.max_packet_size,32);
  assert_eq!(info.interrupt_in_endpoint(&config,1),None);
 }
 #[test]fn rejects_non_hid_and_missing_alternate_settings(){
  let config=fixture();
  assert_eq!(find_interface(&config,7,0).unwrap(),None);
  assert_eq!(find_interface(&config,4,1).unwrap(),None);
 }

 struct MockHost{status:TransferStatus,bytes:usize}
 impl HostController for MockHost{
  fn kind(&self)->super::super::controller::ControllerKind{super::super::controller::ControllerKind::Xhci}
  fn port_count(&self)->u8{1}
  fn port_status(&self,_:u8)->Result<super::super::controller::PortStatus,UsbError>{
   Ok(super::super::controller::PortStatus{connected:true,enabled:true,suspended:false,over_current:false,speed:Some(super::super::transfer::UsbSpeed::High)})
  }
  fn control_transfer(&mut self,_:DeviceAddress,_:super::super::transfer::SetupPacket,_:&mut[u8])->Result<super::super::controller::TransferResult,UsbError>{Err(UsbError::Unsupported)}
  fn bulk_transfer(&mut self,_:DeviceAddress,_:Endpoint,_:&mut[u8])->Result<super::super::controller::TransferResult,UsbError>{Err(UsbError::Unsupported)}
  fn interrupt_transfer(&mut self,_:DeviceAddress,_:Endpoint,data:&mut[u8])->Result<super::super::controller::TransferResult,UsbError>{
   if !data.is_empty(){data[0]=0xa5}
   Ok(super::super::controller::TransferResult{status:self.status,bytes_transferred:self.bytes})
  }
  fn reset_port(&mut self,_:u8)->Result<(),UsbError>{Ok(())}
 }
 #[test]fn raw_report_reader_validates_transfer_status_and_length(){
  let endpoint=Endpoint{address:EndpointAddress::from_raw(0x81),transfer_type:TransferType::Interrupt,max_packet_size:8,interval:1};
  let mut host=MockHost{status:TransferStatus::Success,bytes:2};let mut buffer=[0u8;8];
  assert_eq!(read_interrupt_report(&mut host,DeviceAddress::new(2).unwrap(),endpoint,&mut buffer),Ok(2));
  assert_eq!(buffer[0],0xa5);
  host.status=TransferStatus::TimedOut;
  assert_eq!(read_interrupt_report(&mut host,DeviceAddress::new(2).unwrap(),endpoint,&mut buffer),Err(UsbError::Timeout));
  host.status=TransferStatus::Success;host.bytes=9;
  assert_eq!(read_interrupt_report(&mut host,DeviceAddress::new(2).unwrap(),endpoint,&mut buffer),Err(UsbError::TransferFailed));
 }
 #[test]fn raw_report_reader_rejects_invalid_endpoint_but_allows_multi_packet_reports(){
  let mut host=MockHost{status:TransferStatus::Success,bytes:0};
  let out=Endpoint{address:EndpointAddress::from_raw(0x01),transfer_type:TransferType::Interrupt,max_packet_size:8,interval:1};
  assert_eq!(read_interrupt_report(&mut host,DeviceAddress::new(2).unwrap(),out,&mut[0;8]),Err(UsbError::MalformedDescriptor));
  let input=Endpoint{address:EndpointAddress::from_raw(0x81),..out};
  host.bytes=9;
  assert_eq!(read_interrupt_report(&mut host,DeviceAddress::new(2).unwrap(),input,&mut[0;9]),Ok(9));
 }
}
