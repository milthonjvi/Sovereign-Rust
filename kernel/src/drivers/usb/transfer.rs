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
  let transfer_type=match b[3]&3{0=>TransferType::Control,1=>TransferType::Isochronous,2=>TransferType::Bulk,_=>TransferType::Interrupt};
  let max_packet_size=u16::from_le_bytes([b[4],b[5]])&0x7ff;
  if max_packet_size==0{return Err(UsbError::MalformedDescriptor)}
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
