//! Allocation-free USB descriptor parsing.
use super::UsbError;
pub const DEVICE_DESCRIPTOR: u8 = 1;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceDescriptor {
 pub usb_release:u16,pub device_class:u8,pub device_subclass:u8,pub device_protocol:u8,
 pub max_packet_size_0:u8,pub vendor_id:u16,pub product_id:u16,pub device_release:u16,
 pub manufacturer_index:u8,pub product_index:u8,pub serial_index:u8,pub configurations:u8,
}
impl DeviceDescriptor {
 pub const LENGTH:usize=18;
 pub fn parse(b:&[u8])->Result<Self,UsbError>{
  if b.len()<18{return Err(UsbError::BufferTooShort)}
  if b[0]!=18||b[1]!=DEVICE_DESCRIPTOR{return Err(UsbError::MalformedDescriptor)}
  let w=|i:usize|u16::from_le_bytes([b[i],b[i+1]]);
  let d=Self{usb_release:w(2),device_class:b[4],device_subclass:b[5],device_protocol:b[6],
   max_packet_size_0:b[7],vendor_id:w(8),product_id:w(10),device_release:w(12),
   manufacturer_index:b[14],product_index:b[15],serial_index:b[16],configurations:b[17]};
  if d.max_packet_size_0==0||d.configurations==0{return Err(UsbError::MalformedDescriptor)}
  Ok(d)
 }
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct Descriptor<'a>{pub descriptor_type:u8,pub bytes:&'a[u8]}
pub struct DescriptorIter<'a>{bytes:&'a[u8],offset:usize,failed:bool}
impl<'a> DescriptorIter<'a>{pub const fn new(bytes:&'a[u8])->Self{Self{bytes,offset:0,failed:false}}}
impl<'a> Iterator for DescriptorIter<'a>{
 type Item=Result<Descriptor<'a>,UsbError>;
 fn next(&mut self)->Option<Self::Item>{
  if self.failed||self.offset==self.bytes.len(){return None}
  if self.bytes.len()-self.offset<2{self.failed=true;return Some(Err(UsbError::MalformedDescriptor))}
  let n=self.bytes[self.offset] as usize;
  if n<2||n>self.bytes.len()-self.offset{self.failed=true;return Some(Err(UsbError::MalformedDescriptor))}
  let s=self.offset;self.offset+=n;
  Some(Ok(Descriptor{descriptor_type:self.bytes[s+1],bytes:&self.bytes[s..s+n]}))
 }
}
