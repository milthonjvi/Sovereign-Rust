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


// Standard configuration, interface, endpoint and SuperSpeed companion descriptors.
pub const CONFIGURATION_DESCRIPTOR:u8=2;
pub const INTERFACE_DESCRIPTOR:u8=4;
pub const ENDPOINT_DESCRIPTOR:u8=5;
pub const SS_ENDPOINT_COMPANION_DESCRIPTOR:u8=0x30;
pub const MAX_INTERFACES:usize=64;
pub const MAX_ENDPOINTS:usize=128;

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct ConfigurationDescriptor{
 pub total_length:u16,pub interface_count:u8,pub configuration_value:u8,
 pub configuration_index:u8,pub attributes:u8,pub max_power:u8,
}
impl ConfigurationDescriptor{
 pub const LENGTH:usize=9;
 pub fn parse(b:&[u8])->Result<Self,UsbError>{
  if b.len()<Self::LENGTH{return Err(UsbError::BufferTooShort)}
  if b[0]<9||b[1]!=CONFIGURATION_DESCRIPTOR{return Err(UsbError::MalformedDescriptor)}
  let d=Self{total_length:u16::from_le_bytes([b[2],b[3]]),interface_count:b[4],
   configuration_value:b[5],configuration_index:b[6],attributes:b[7],max_power:b[8]};
  if d.total_length<9||d.interface_count==0||d.configuration_value==0||d.attributes&0x80==0{
   return Err(UsbError::MalformedDescriptor)
  }
  Ok(d)
 }
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct InterfaceDescriptor{
 pub number:u8,pub alternate_setting:u8,pub endpoint_count:u8,
 pub class:u8,pub subclass:u8,pub protocol:u8,pub string_index:u8,
}
impl InterfaceDescriptor{
 fn parse(b:&[u8])->Result<Self,UsbError>{
  if b.len()<9||b[0]<9||b[1]!=INTERFACE_DESCRIPTOR{return Err(UsbError::MalformedDescriptor)}
  Ok(Self{number:b[2],alternate_setting:b[3],endpoint_count:b[4],class:b[5],
   subclass:b[6],protocol:b[7],string_index:b[8]})
 }
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct ParsedEndpoint{
 pub address:u8,pub attributes:u8,pub max_packet_size:u16,pub interval:u8,
 pub transactions_per_microframe:u8,pub max_burst:u8,pub companion_attributes:u8,
 pub bytes_per_interval:u16,pub has_ss_companion:bool,
}
impl ParsedEndpoint{
 fn parse(b:&[u8])->Result<Self,UsbError>{
  if b.len()<7||b[0]<7||b[1]!=ENDPOINT_DESCRIPTOR{return Err(UsbError::MalformedDescriptor)}
  let raw=u16::from_le_bytes([b[4],b[5]]);
  let packet=raw&0x07ff;
  let mult=((raw>>11)&3) as u8;
  let transfer=b[3]&3;
  if packet==0||raw&0xe000!=0||mult==3||b[2]&0x0f==0||b[2]&0x70!=0
   ||transfer==0||(transfer==1&&b[3]&0xc0!=0)||(transfer!=1&&b[3]&0xfc!=0)
   ||((transfer!=1&&transfer!=3)&&mult!=0){return Err(UsbError::MalformedDescriptor)}
  Ok(Self{address:b[2],attributes:b[3],max_packet_size:packet,interval:b[6],
   transactions_per_microframe:mult+1,max_burst:0,companion_attributes:0,
   bytes_per_interval:0,has_ss_companion:false})
 }
 fn apply_companion(&mut self,b:&[u8])->Result<(),UsbError>{
  if b.len()<6||b[0]!=6||b[1]!=SS_ENDPOINT_COMPANION_DESCRIPTOR||self.has_ss_companion||b[2]>15{
   return Err(UsbError::MalformedDescriptor)
  }
  let transfer=self.attributes&3;
  let attributes=b[3];
  let bytes_per_interval=u16::from_le_bytes([b[4],b[5]]);
  match transfer{
   2=>{if attributes&0xe0!=0||bytes_per_interval!=0{return Err(UsbError::MalformedDescriptor)}},
   1=>{if attributes&0xfc!=0||attributes&3>2||bytes_per_interval==0{return Err(UsbError::MalformedDescriptor)}},
   3=>{if attributes!=0||bytes_per_interval==0{return Err(UsbError::MalformedDescriptor)}},
   _=>return Err(UsbError::MalformedDescriptor),
  }
  self.max_burst=b[2];self.companion_attributes=attributes;
  self.bytes_per_interval=bytes_per_interval;self.has_ss_companion=true;Ok(())
 }
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct ParsedInterface{pub descriptor:InterfaceDescriptor,pub first_endpoint:u8,pub endpoint_count:u8}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct ParsedConfiguration{
 pub descriptor:ConfigurationDescriptor,
 pub interfaces:[Option<ParsedInterface>;MAX_INTERFACES],
 pub endpoints:[Option<ParsedEndpoint>;MAX_ENDPOINTS],
 pub interface_len:u8,pub endpoint_len:u8,
}
impl ParsedConfiguration{
 /// Parses a complete configuration blob without allocation. The input must
 /// contain exactly the descriptor chain declared by wTotalLength.
 pub fn parse(bytes:&[u8])->Result<Self,UsbError>{
  let descriptor=ConfigurationDescriptor::parse(bytes)?;
  let total=descriptor.total_length as usize;
  if bytes.len()<total{return Err(UsbError::BufferTooShort)}
  let mut out=Self{descriptor,interfaces:[None;MAX_INTERFACES],endpoints:[None;MAX_ENDPOINTS],interface_len:0,endpoint_len:0};
  let mut current:Option<usize>=None;
  let mut first=true;
  for item in DescriptorIter::new(&bytes[..total]){
   let item=item?;
   if first{
    if item.descriptor_type!=CONFIGURATION_DESCRIPTOR||item.bytes[0]<9{return Err(UsbError::MalformedDescriptor)}
    first=false;
    continue
   }
   match item.descriptor_type{
    CONFIGURATION_DESCRIPTOR=>return Err(UsbError::MalformedDescriptor),
    INTERFACE_DESCRIPTOR=>{
     if out.interface_len as usize>=MAX_INTERFACES{return Err(UsbError::NoResources)}
     let d=InterfaceDescriptor::parse(item.bytes)?;
     // One entry per alternate setting; repeated interface numbers are valid.
     let i=out.interface_len as usize;
     out.interfaces[i]=Some(ParsedInterface{descriptor:d,first_endpoint:out.endpoint_len,endpoint_count:0});
     out.interface_len+=1;current=Some(i);
    }
    ENDPOINT_DESCRIPTOR=>{
     let i=current.ok_or(UsbError::MalformedDescriptor)?;
     if out.endpoint_len as usize>=MAX_ENDPOINTS{return Err(UsbError::NoResources)}
     let e=ParsedEndpoint::parse(item.bytes)?;
     let n=out.endpoint_len as usize;out.endpoints[n]=Some(e);out.endpoint_len+=1;
     let iface=out.interfaces[i].as_mut().ok_or(UsbError::MalformedDescriptor)?;
     iface.endpoint_count=iface.endpoint_count.saturating_add(1);
    }
    SS_ENDPOINT_COMPANION_DESCRIPTOR=>{
     if out.endpoint_len==0{return Err(UsbError::MalformedDescriptor)}
     let e=out.endpoints[out.endpoint_len as usize-1].as_mut().ok_or(UsbError::MalformedDescriptor)?;
     e.apply_companion(item.bytes)?;
    }
    _=>{}
   }
  }
  let mut unique_interfaces=[false;256];
  let mut unique_count=0u8;
  for iface in out.interfaces[..out.interface_len as usize].iter().flatten(){
   if iface.endpoint_count!=iface.descriptor.endpoint_count{return Err(UsbError::MalformedDescriptor)}
   let n=iface.descriptor.number as usize;
   if !unique_interfaces[n]{unique_interfaces[n]=true;unique_count=unique_count.saturating_add(1);}
  }
  // bNumInterfaces counts distinct interface numbers, not alternate settings.
  if unique_count!=descriptor.interface_count{return Err(UsbError::MalformedDescriptor)}
  // Alternate-setting numbers and endpoint addresses must be unique within
  // each interface alternate setting.
  for i in 0..out.interface_len as usize{
   let a=out.interfaces[i].ok_or(UsbError::MalformedDescriptor)?;
   for j in i+1..out.interface_len as usize{
    let b=out.interfaces[j].ok_or(UsbError::MalformedDescriptor)?;
    if a.descriptor.number==b.descriptor.number&&a.descriptor.alternate_setting==b.descriptor.alternate_setting{
     return Err(UsbError::MalformedDescriptor)
    }
   }
   let start=a.first_endpoint as usize;
   let end=start+a.endpoint_count as usize;
   for x in start..end{
    let ex=out.endpoints[x].ok_or(UsbError::MalformedDescriptor)?;
    for y in x+1..end{
     let ey=out.endpoints[y].ok_or(UsbError::MalformedDescriptor)?;
     if ex.address==ey.address{return Err(UsbError::MalformedDescriptor)}
    }
   }
  }
  Ok(out)
 }
}


#[cfg(test)]
mod configuration_tests{
 use super::*;
 #[test]fn parses_interface_endpoint_and_superspeed_companion(){
  let bytes:[u8;31]=[
   9,2,31,0,1,1,0,0x80,50,
   9,4,0,0,1,0xff,0,0,0,
   7,5,0x81,2,0,2,0,
   6,0x30,3,0,0,0,
  ];
  let c=ParsedConfiguration::parse(&bytes).unwrap();
  assert_eq!(c.interface_len,1);assert_eq!(c.endpoint_len,1);
  let e=c.endpoints[0].unwrap();
  assert_eq!(e.max_packet_size,512);assert_eq!(e.max_burst,3);
  assert!(e.has_ss_companion);assert_eq!(e.bytes_per_interval,0);
 }
 #[test]fn rejects_configuration_with_truncated_descriptor(){
  let bytes=[9,2,12,0,1,1,0,0x80,50,9,4,0];
  assert_eq!(ParsedConfiguration::parse(&bytes),Err(UsbError::BufferTooShort));
 }
 #[test]fn rejects_endpoint_multiplier_on_bulk_endpoint(){
  let bytes=[7,5,0x81,2,0,10,0];
  assert_eq!(ParsedEndpoint::parse(&bytes),Err(UsbError::MalformedDescriptor));
 }
}

#[cfg(test)]
mod alternate_setting_tests{
 use super::*;
 #[test]fn alternate_settings_do_not_inflate_interface_count(){
  let bytes:[u8;43]=[
   9,2,43,0,2,1,0,0x80,50,
   9,4,0,0,0,0xff,0,0,0,
   9,4,0,1,1,0xff,0,0,0,
   7,5,0x81,2,0,2,0,
   9,4,1,0,0,0xff,0,0,0,
  ];
  let parsed=ParsedConfiguration::parse(&bytes).unwrap();
  assert_eq!(parsed.interface_len,3);
 }
}

#[cfg(test)]
mod superspeed_companion_validation_tests{
 use super::*;
 #[test]fn validates_isochronous_companion_fields(){
  let mut endpoint=ParsedEndpoint::parse(&[7,5,0x81,1,0,2,1]).unwrap();
  endpoint.apply_companion(&[6,0x30,2,1,0,4]).unwrap();
  assert_eq!(endpoint.max_burst,2);assert_eq!(endpoint.bytes_per_interval,1024);
 }
 #[test]fn rejects_reserved_bulk_companion_bytes_and_excess_burst(){
  let mut endpoint=ParsedEndpoint::parse(&[7,5,0x81,2,0,2,0]).unwrap();
  assert_eq!(endpoint.apply_companion(&[6,0x30,1,0,0,1]),Err(UsbError::MalformedDescriptor));
  assert_eq!(endpoint.apply_companion(&[6,0x30,16,0,0,0]),Err(UsbError::MalformedDescriptor));
 }
 #[test]fn rejects_reserved_isochronous_endpoint_attribute_bits(){
  assert_eq!(ParsedEndpoint::parse(&[7,5,0x81,0x41,0,2,1]),Err(UsbError::MalformedDescriptor));
 }
}
