//! USB bus addressing, endpoints, and control-transfer setup packets.
use super::UsbError;
#[derive(Clone,Copy,Debug,Eq,PartialEq)]#[repr(u8)]
pub enum UsbSpeed{Low=1,Full=2,High=3,Super=4,SuperPlus=5}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]#[repr(u8)]
pub enum TransferType{Control=0,Isochronous=1,Bulk=2,Interrupt=3}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]pub struct DeviceAddress(u8);
impl DeviceAddress{
 pub const DEFAULT:Self=Self(0);
 pub fn new(v:u8)->Result<Self,UsbError>{if v<=127{Ok(Self(v))}else{Err(UsbError::InvalidAddress)}}
 pub const fn get(self)->u8{self.0}
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]pub struct EndpointAddress(u8);
impl EndpointAddress{
 pub const fn from_raw(v:u8)->Self{Self(v)}
 pub const fn number(self)->u8{self.0&15}
 pub const fn is_in(self)->bool{self.0&128!=0}
 pub const fn raw(self)->u8{self.0}
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]pub struct Endpoint{
 pub address:EndpointAddress,pub transfer_type:TransferType,pub max_packet_size:u16,pub interval:u8,
}
impl Endpoint{
 pub fn from_descriptor(b:&[u8])->Result<Self,UsbError>{
  if b.len()<7||b[0]<7||b[1]!=5{return Err(UsbError::MalformedDescriptor)}
  let raw=u16::from_le_bytes([b[4],b[5]]);
  let transfer= b[3]&3;
  let transfer_type=match transfer{0=>return Err(UsbError::MalformedDescriptor),1=>TransferType::Isochronous,2=>TransferType::Bulk,_=>TransferType::Interrupt};
  let max_packet_size=raw&0x07ff;
  let multiplier=(raw>>11)&3;
  if max_packet_size==0||raw&0xe000!=0||multiplier==3||b[2]&0x0f==0||b[2]&0x70!=0
   ||(transfer!=1&&b[3]&0xfc!=0)||((transfer!=1&&transfer!=3)&&multiplier!=0){
   return Err(UsbError::MalformedDescriptor)
  }
  Ok(Self{address:EndpointAddress::from_raw(b[2]),transfer_type,max_packet_size,interval:b[6]})
 }
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]#[repr(C)]
pub struct SetupPacket{pub request_type:u8,pub request:u8,pub value:u16,pub index:u16,pub length:u16}
impl SetupPacket{
 pub const fn new(request_type:u8,request:u8,value:u16,index:u16,length:u16)->Self{Self{request_type,request,value,index,length}}
 pub const fn get_descriptor(kind:u8,index:u8,length:u16)->Self{Self::new(0x80,6,((kind as u16)<<8)|index as u16,0,length)}
 pub const fn set_address(address:DeviceAddress)->Self{Self::new(0,5,address.get() as u16,0,0)}
 pub fn encode(self)->[u8;8]{let v=self.value.to_le_bytes();let i=self.index.to_le_bytes();let l=self.length.to_le_bytes();[self.request_type,self.request,v[0],v[1],i[0],i[1],l[0],l[1]]}
}

#[cfg(test)]
mod endpoint_validation_tests{
 use super::*;
 #[test]fn parses_valid_bulk_endpoint(){
  let endpoint=Endpoint::from_descriptor(&[7,5,0x81,2,0,2,0]).unwrap();
  assert_eq!(endpoint.address.number(),1);assert!(endpoint.address.is_in());
  assert_eq!(endpoint.transfer_type,TransferType::Bulk);assert_eq!(endpoint.max_packet_size,512);
 }
 #[test]fn rejects_control_endpoint_and_reserved_address_bits(){
  assert_eq!(Endpoint::from_descriptor(&[7,5,0x81,0,8,0,0]),Err(UsbError::MalformedDescriptor));
  assert_eq!(Endpoint::from_descriptor(&[7,5,0x91,2,0,2,0]),Err(UsbError::MalformedDescriptor));
 }
 #[test]fn rejects_reserved_packet_size_and_invalid_high_bandwidth_multiplier(){
  assert_eq!(Endpoint::from_descriptor(&[7,5,0x81,2,0,0x20,0]),Err(UsbError::MalformedDescriptor));
  assert_eq!(Endpoint::from_descriptor(&[7,5,0x81,3,0,0x18,1]),Err(UsbError::MalformedDescriptor));
 }
}
