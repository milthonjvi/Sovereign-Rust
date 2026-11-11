//! USB Communications Device Class (CDC) ACM support primitives.
use super::{descriptors::{DescriptorIter,ParsedConfiguration},transfer::{Endpoint,EndpointAddress,SetupPacket,TransferType},UsbError};

pub const USB_CLASS_COMMUNICATIONS:u8=0x02;
pub const USB_CLASS_CDC_DATA:u8=0x0a;
pub const CDC_SUBCLASS_ACM:u8=0x02;
pub const CDC_PROTOCOL_NONE:u8=0x00;
pub const CDC_PROTOCOL_AT:u8=0x01;
pub const CDC_CS_INTERFACE:u8=0x24;
pub const CDC_HEADER_SUBTYPE:u8=0x00;
pub const CDC_CALL_MANAGEMENT_SUBTYPE:u8=0x01;
pub const CDC_ACM_SUBTYPE:u8=0x02;
pub const CDC_UNION_SUBTYPE:u8=0x06;
pub const CDC_SET_COMM_FEATURE:u8=0x02;
pub const CDC_GET_COMM_FEATURE:u8=0x03;
pub const CDC_CLEAR_COMM_FEATURE:u8=0x04;
pub const CDC_SET_LINE_CODING:u8=0x20;
pub const CDC_GET_LINE_CODING:u8=0x21;
pub const CDC_SET_CONTROL_LINE_STATE:u8=0x22;
pub const CDC_SEND_BREAK:u8=0x23;

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct CdcAcmFunctionalDescriptors{
 pub control_interface:u8,pub data_interface:Option<u8>,pub capabilities:u8,
 pub call_management_capabilities:u8,pub call_management_data_interface:Option<u8>,
 pub country_code_count:u8,
}
impl CdcAcmFunctionalDescriptors{
 pub fn parse(configuration:&[u8],control_interface:u8)->Result<Self,UsbError>{
  let mut current=None;
  let mut class_control=false;
  let mut result=Self{control_interface,data_interface:None,capabilities:0,
   call_management_capabilities:0,call_management_data_interface:None,country_code_count:0};
  let mut found=false;
  for item in DescriptorIter::new(configuration){
   let d=item?;
   if d.descriptor_type==4{
    if d.bytes.len()<9{return Err(UsbError::MalformedDescriptor)}
    current=Some(d.bytes[2]);
    class_control=d.bytes[5]==USB_CLASS_COMMUNICATIONS&&d.bytes[6]==CDC_SUBCLASS_ACM;
    continue
   }
   if current!=Some(control_interface)||!class_control||d.descriptor_type!=CDC_CS_INTERFACE{continue}
   if d.bytes.len()<3{return Err(UsbError::MalformedDescriptor)}
   match d.bytes[2]{
    CDC_HEADER_SUBTYPE=>{if d.bytes.len()<5{return Err(UsbError::MalformedDescriptor)}found=true;}
    CDC_CALL_MANAGEMENT_SUBTYPE=>{
     if d.bytes.len()<5{return Err(UsbError::MalformedDescriptor)}
     result.call_management_capabilities=d.bytes[3];
     result.call_management_data_interface=Some(d.bytes[4]);
    }
    CDC_ACM_SUBTYPE=>{
     if d.bytes.len()<4{return Err(UsbError::MalformedDescriptor)}
     result.capabilities=d.bytes[3];found=true;
    }
    CDC_UNION_SUBTYPE=>{
     if d.bytes.len()<5||d.bytes[3]!=control_interface{return Err(UsbError::MalformedDescriptor)}
     result.data_interface=Some(d.bytes[4]);found=true;
    }
    _=>{}
   }
  }
  if !found||result.data_interface.is_none(){return Err(UsbError::MalformedDescriptor)}
  Ok(result)
 }
}

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct CdcAcmInterface{
 pub control_interface:u8,pub data_interface:u8,pub capabilities:u8,
 pub bulk_in:Endpoint,pub bulk_out:Endpoint,pub notification_in:Option<Endpoint>,
}
pub fn find_acm_interface(configuration:&[u8],parsed:&ParsedConfiguration)->Result<CdcAcmInterface,UsbError>{
 for iface in parsed.interfaces[..parsed.interface_len as usize].iter().flatten(){
  let d=iface.descriptor;
  if d.class!=USB_CLASS_COMMUNICATIONS||d.subclass!=CDC_SUBCLASS_ACM{continue}
  let functional=CdcAcmFunctionalDescriptors::parse(configuration,d.number)?;
  let data_number=functional.data_interface.ok_or(UsbError::MalformedDescriptor)?;
  let data=parsed.interfaces[..parsed.interface_len as usize].iter().flatten()
   .find(|i|i.descriptor.number==data_number&&i.descriptor.class==USB_CLASS_CDC_DATA)
   .ok_or(UsbError::MalformedDescriptor)?;
  let mut bulk_in=None;let mut bulk_out=None;let mut notification=None;
  let start=iface.first_endpoint as usize;let end=start+iface.endpoint_count as usize;
  for ep in parsed.endpoints[start..end].iter().flatten(){
   if ep.address&0x80!=0&&ep.attributes&3==TransferType::Interrupt as u8{
    notification=Some(to_endpoint(*ep,TransferType::Interrupt)?);
   }
  }
  let start=data.first_endpoint as usize;let end=start+data.endpoint_count as usize;
  for ep in parsed.endpoints[start..end].iter().flatten(){
   if ep.attributes&3!=TransferType::Bulk as u8{continue}
   let endpoint=to_endpoint(*ep,TransferType::Bulk)?;
   if ep.address&0x80!=0{bulk_in=Some(endpoint)}else{bulk_out=Some(endpoint)}
  }
  return Ok(CdcAcmInterface{control_interface:d.number,data_interface:data_number,
   capabilities:functional.capabilities,bulk_in:bulk_in.ok_or(UsbError::MalformedDescriptor)?,
   bulk_out:bulk_out.ok_or(UsbError::MalformedDescriptor)?,notification_in:notification});
 }
 Err(UsbError::Unsupported)
}
fn to_endpoint(ep:super::descriptors::ParsedEndpoint,kind:TransferType)->Result<Endpoint,UsbError>{
 if ep.address&0x0f==0||ep.max_packet_size==0{return Err(UsbError::MalformedDescriptor)}
 Ok(Endpoint{address:EndpointAddress::from_raw(ep.address),transfer_type:kind,
  max_packet_size:ep.max_packet_size,interval:ep.interval})
}

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct CdcLineCoding{pub baud_rate:u32,pub stop_bits:u8,pub parity:u8,pub data_bits:u8}
impl CdcLineCoding{
 pub const fn new(baud_rate:u32,data_bits:u8)->Self{Self{baud_rate,stop_bits:0,parity:0,data_bits}}
 pub fn parse(bytes:&[u8])->Result<Self,UsbError>{
  if bytes.len()<7{return Err(UsbError::BufferTooShort)}
  let baud_rate=u32::from_le_bytes([bytes[0],bytes[1],bytes[2],bytes[3]]);
  let value=Self{baud_rate,stop_bits:bytes[4],parity:bytes[5],data_bits:bytes[6]};
  value.validate()?;Ok(value)
 }
 pub fn validate(self)->Result<(),UsbError>{
  if self.baud_rate==0||self.stop_bits>2||self.parity>4||(!(5..=8).contains(&self.data_bits)&&self.data_bits!=16){
   return Err(UsbError::MalformedDescriptor)
  }
  Ok(())
 }
 pub fn encode(self)->Result<[u8;7],UsbError>{
  self.validate()?;let baud=self.baud_rate.to_le_bytes();
  Ok([baud[0],baud[1],baud[2],baud[3],self.stop_bits,self.parity,self.data_bits])
 }
}
pub const fn set_line_coding(interface:u8)->SetupPacket{
 SetupPacket::new(0x21,CDC_SET_LINE_CODING,0,interface as u16,7)
}
pub const fn get_line_coding(interface:u8)->SetupPacket{
 SetupPacket::new(0xa1,CDC_GET_LINE_CODING,0,interface as u16,7)
}
pub const fn set_control_line_state(interface:u8,dtr:bool,rts:bool)->SetupPacket{
 SetupPacket::new(0x21,CDC_SET_CONTROL_LINE_STATE,(dtr as u16)|((rts as u16)<<1),interface as u16,0)
}
pub const fn send_break(interface:u8,duration_ms:u16)->SetupPacket{
 SetupPacket::new(0x21,CDC_SEND_BREAK,duration_ms,interface as u16,0)
}

#[cfg(test)]
mod tests{
 use super::*;
 #[test]fn roundtrips_valid_line_coding(){
  let coding=CdcLineCoding{baud_rate:115200,stop_bits:0,parity:0,data_bits:8};
  assert_eq!(CdcLineCoding::parse(&coding.encode().unwrap()).unwrap(),coding);
 }
 #[test]fn rejects_invalid_line_coding(){
  assert_eq!(CdcLineCoding::parse(&[0,0,0,0,0,0,8]),Err(UsbError::MalformedDescriptor));
  assert_eq!(CdcLineCoding::parse(&[0,0,0,0,0,0,9]),Err(UsbError::MalformedDescriptor));
 }
 #[test]fn builds_cdc_control_requests(){
  let set=set_line_coding(3);assert_eq!(set.request,CDC_SET_LINE_CODING);assert_eq!(set.length,7);
  let state=set_control_line_state(3,true,true);assert_eq!(state.value,3);
  let brk=send_break(3,250);assert_eq!(brk.value,250);
 }
 #[test]fn finds_union_mapping_for_control_interface(){
  let descriptors=[9,2,42,0,2,1,0,0x80,50,
   9,4,0,0,1,2,2,1,0,5,0x24,0,0x10,0x01,
   5,0x24,1,2,1,5,0x24,6,0,1,
   9,4,1,0,2,0x0a,0,0,0];
  let d=CdcAcmFunctionalDescriptors::parse(&descriptors,0).unwrap();
  assert_eq!(d.data_interface,Some(1));assert_eq!(d.capabilities,2);
 }
}


use super::{control::execute_control,controller::{HostController,TransferStatus},transfer::DeviceAddress};
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct CdcTransferReport{pub bytes_transferred:usize,pub short_packet:bool}
pub struct CdcAcmPort{pub address:DeviceAddress,pub interface:CdcAcmInterface}
impl CdcAcmPort{
 pub const fn new(address:DeviceAddress,interface:CdcAcmInterface)->Self{Self{address,interface}}
 pub fn set_line_coding<C:HostController>(&self,controller:&mut C,coding:CdcLineCoding)->Result<(),UsbError>{
  let mut bytes=coding.encode()?;
  let report=execute_control(controller,self.address,set_line_coding(self.interface.control_interface),&mut bytes)?;
  if report.bytes_transferred!=7{return Err(UsbError::TransferFailed)}Ok(())
 }
 pub fn get_line_coding<C:HostController>(&self,controller:&mut C)->Result<CdcLineCoding,UsbError>{
  let mut bytes=[0u8;7];
  let report=execute_control(controller,self.address,get_line_coding(self.interface.control_interface),&mut bytes)?;
  if report.bytes_transferred!=7{return Err(UsbError::TransferFailed)}
  CdcLineCoding::parse(&bytes)
 }
 pub fn set_control_lines<C:HostController>(&self,controller:&mut C,dtr:bool,rts:bool)->Result<(),UsbError>{
  let mut empty=[];
  execute_control(controller,self.address,set_control_line_state(self.interface.control_interface,dtr,rts),&mut empty)?;
  Ok(())
 }
 pub fn send_break<C:HostController>(&self,controller:&mut C,duration_ms:u16)->Result<(),UsbError>{
  let mut empty=[];
  execute_control(controller,self.address,send_break(self.interface.control_interface,duration_ms),&mut empty)?;
  Ok(())
 }
 pub fn read<C:HostController>(&self,controller:&mut C,buffer:&mut[u8])->Result<CdcTransferReport,UsbError>{
  let result=controller.bulk_transfer(self.address,self.interface.bulk_in,buffer)?;
  match result.status{
   TransferStatus::Success=>{},
   TransferStatus::TimedOut=>return Err(UsbError::Timeout),
   TransferStatus::NoDevice=>return Err(UsbError::ControllerUnavailable),
   TransferStatus::Stall=>return Err(UsbError::Unsupported),
   TransferStatus::Pending|TransferStatus::ControllerError=>return Err(UsbError::TransferFailed),
  }
  if result.bytes_transferred>buffer.len(){return Err(UsbError::TransferFailed)}
  Ok(CdcTransferReport{bytes_transferred:result.bytes_transferred,short_packet:result.bytes_transferred<buffer.len()})
 }
 pub fn write<C:HostController>(&self,controller:&mut C,buffer:&mut[u8])->Result<CdcTransferReport,UsbError>{
  let result=controller.bulk_transfer(self.address,self.interface.bulk_out,buffer)?;
  match result.status{
   TransferStatus::Success=>{},
   TransferStatus::TimedOut=>return Err(UsbError::Timeout),
   TransferStatus::NoDevice=>return Err(UsbError::ControllerUnavailable),
   TransferStatus::Stall=>return Err(UsbError::Unsupported),
   TransferStatus::Pending|TransferStatus::ControllerError=>return Err(UsbError::TransferFailed),
  }
  if result.bytes_transferred>buffer.len(){return Err(UsbError::TransferFailed)}
  if result.bytes_transferred!=buffer.len(){return Err(UsbError::TransferFailed)}
  Ok(CdcTransferReport{bytes_transferred:result.bytes_transferred,short_packet:false})
 }
}
