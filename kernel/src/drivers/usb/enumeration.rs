//! Allocation-free USB device-address allocation and enumeration state machine.
//! The caller executes each setup packet through a HostController and feeds the
//! returned data/status stage to complete(). Timing and port reset remain the
//! responsibility of the host-controller/port manager.
use super::{descriptors::{ConfigurationDescriptor,DeviceDescriptor,ParsedConfiguration},transfer::{DeviceAddress,SetupPacket},UsbError};

pub const GET_DESCRIPTOR:u8=6;
pub const SET_ADDRESS:u8=5;
pub const SET_CONFIGURATION:u8=9;
pub const DESC_DEVICE:u8=1;
pub const DESC_CONFIGURATION:u8=2;

/// USB addresses 1..=127. Address zero is reserved for enumeration.
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct DeviceAddressPool{used:u128}
impl DeviceAddressPool{
 pub const fn new()->Self{Self{used:1}}
 pub fn allocate(&mut self)->Result<DeviceAddress,UsbError>{
  let free=(!self.used)&(!1u128);
  if free==0{return Err(UsbError::NoResources)}
  let n=free.trailing_zeros() as u8;
  self.used|=1u128<<n;
  DeviceAddress::new(n)
 }
 pub fn release(&mut self,address:DeviceAddress)->Result<(),UsbError>{
  let n=address.get();
  if n==0||self.used&(1u128<<n)==0{return Err(UsbError::InvalidAddress)}
  self.used&=!(1u128<<n);Ok(())
 }
 pub fn is_allocated(&self,address:DeviceAddress)->bool{
  self.used&(1u128<<address.get())!=0
 }
}
impl Default for DeviceAddressPool{fn default()->Self{Self::new()}}

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub enum EnumerationStage{
 ReadInitialDeviceDescriptor,SetAddress,AddressSettle,ReadDeviceDescriptor,
 ReadConfigurationHeader,ReadConfiguration,SetConfiguration,Complete,Failed,
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct EnumerationSession{
 stage:EnumerationStage,address:Option<DeviceAddress>,configuration_value:u8,
 configuration_length:u16,device:Option<DeviceDescriptor>,configuration:Option<ParsedConfiguration>,
}
impl EnumerationSession{
 pub const fn new()->Self{Self{stage:EnumerationStage::ReadInitialDeviceDescriptor,
  address:None,configuration_value:0,configuration_length:0,device:None,configuration:None}}
 pub const fn stage(&self)->EnumerationStage{self.stage}
 pub const fn address(&self)->Option<DeviceAddress>{self.address}
 pub const fn device_descriptor(&self)->Option<DeviceDescriptor>{self.device}
 pub const fn configuration_length(&self)->u16{self.configuration_length}
 pub const fn configuration(&self)->Option<ParsedConfiguration>{self.configuration}
 /// Returns the next control request. None means enumeration has completed or failed.
 pub fn request(&self)->Option<SetupPacket>{
  match self.stage{
   EnumerationStage::ReadInitialDeviceDescriptor=>Some(SetupPacket::get_descriptor(DESC_DEVICE,0,8)),
   EnumerationStage::SetAddress=>Some(SetupPacket::set_address(self.address?)),
   EnumerationStage::AddressSettle=>None,
   EnumerationStage::ReadDeviceDescriptor=>Some(SetupPacket::get_descriptor(DESC_DEVICE,0,18)),
   EnumerationStage::ReadConfigurationHeader=>Some(SetupPacket::get_descriptor(DESC_CONFIGURATION,0,9)),
   EnumerationStage::ReadConfiguration=>Some(SetupPacket::get_descriptor(DESC_CONFIGURATION,0,self.configuration_length)),
   EnumerationStage::SetConfiguration=>Some(SetupPacket::new(0,SET_CONFIGURATION,self.configuration_value as u16,0,0)),
   EnumerationStage::Complete|EnumerationStage::Failed=>None,
  }
 }
 /// Consume data from a successful control transfer. For no-data status stages,
 /// pass an empty slice. On error the session enters Failed.
 pub fn complete(&mut self,data:&[u8],pool:&mut DeviceAddressPool)->Result<(),UsbError>{
  let result=(||{
   match self.stage{
    EnumerationStage::ReadInitialDeviceDescriptor=>{
     if data.len()<8||data[0]!=18||data[1]!=DESC_DEVICE||!matches!(data[7],8|16|32|64|9){return Err(UsbError::MalformedDescriptor)}
     self.address=Some(pool.allocate()?);self.stage=EnumerationStage::SetAddress;
    }
    EnumerationStage::SetAddress=>{
     if !data.is_empty(){return Err(UsbError::MalformedDescriptor)}
     self.stage=EnumerationStage::AddressSettle;
    }
    EnumerationStage::AddressSettle=>return Err(UsbError::Unsupported),
    EnumerationStage::ReadDeviceDescriptor=>{
     let d=DeviceDescriptor::parse(data)?;self.device=Some(d);self.stage=EnumerationStage::ReadConfigurationHeader;
    }
    EnumerationStage::ReadConfigurationHeader=>{
     let d=ConfigurationDescriptor::parse(data)?;
     self.configuration_value=d.configuration_value;self.configuration_length=d.total_length;
     self.stage=EnumerationStage::ReadConfiguration;
    }
    EnumerationStage::ReadConfiguration=>{
     if data.len()<self.configuration_length as usize{return Err(UsbError::BufferTooShort)}
     let parsed=ParsedConfiguration::parse(data)?;
     let d=parsed.descriptor;
     if d.configuration_value!=self.configuration_value||d.total_length!=self.configuration_length{
      return Err(UsbError::MalformedDescriptor)
     }
     self.configuration=Some(parsed);
     self.stage=EnumerationStage::SetConfiguration;
    }
    EnumerationStage::SetConfiguration=>{
     if !data.is_empty(){return Err(UsbError::MalformedDescriptor)}
     self.stage=EnumerationStage::Complete;
    }
    EnumerationStage::Complete|EnumerationStage::Failed=>return Err(UsbError::Unsupported),
   }
   Ok(())
  })();
  if result.is_err(){if let Some(address)=self.address.take(){let _=pool.release(address);}self.stage=EnumerationStage::Failed;}
  result
 }
 /// Advances the mandatory USB address-settle delay after SET_ADDRESS.
 /// The USB specification requires at least 2 ms before the next request.
 pub fn elapse_ms(&mut self,elapsed_ms:u16)->Result<(),UsbError>{
  if self.stage!=EnumerationStage::AddressSettle{return Err(UsbError::Unsupported)}
  if elapsed_ms<2{return Ok(())}
  self.stage=EnumerationStage::ReadDeviceDescriptor;Ok(())
 }
 /// Releases the address after a failed or disconnected enumeration.
 pub fn release_address(&mut self,pool:&mut DeviceAddressPool)->Result<(),UsbError>{
  if let Some(address)=self.address.take(){pool.release(address)?;}
  self.stage=EnumerationStage::Failed;Ok(())
 }
}
impl Default for EnumerationSession{fn default()->Self{Self::new()}}

#[cfg(test)]
mod tests{
 use super::*;
 #[test]fn address_pool_allocates_all_nonzero_addresses_and_reuses_released(){
  let mut pool=DeviceAddressPool::new();
  let first=pool.allocate().unwrap();assert_eq!(first.get(),1);
  let mut last=first;
  for _ in 1..127{last=pool.allocate().unwrap();}
  assert_eq!(last.get(),127);assert_eq!(pool.allocate(),Err(UsbError::NoResources));
  pool.release(first).unwrap();assert_eq!(pool.allocate().unwrap(),first);
 }
 #[test]fn enumeration_requests_follow_usb_control_sequence(){
  let mut pool=DeviceAddressPool::new();let mut s=EnumerationSession::new();
  assert_eq!(s.request().unwrap().request,GET_DESCRIPTOR);
  s.complete(&[18,1,0,2,0,0,0,64],&mut pool).unwrap();
  assert_eq!(s.stage(),EnumerationStage::SetAddress);
  assert_eq!(s.request().unwrap().request,SET_ADDRESS);
  s.complete(&[],&mut pool).unwrap();
  assert_eq!(s.stage(),EnumerationStage::AddressSettle);
  assert_eq!(s.request(),None);
  s.elapse_ms(1).unwrap();assert_eq!(s.stage(),EnumerationStage::AddressSettle);
  s.elapse_ms(1).unwrap();assert_eq!(s.stage(),EnumerationStage::ReadDeviceDescriptor);
  assert_eq!(s.request().unwrap().length,18);
 }
 #[test]fn failed_enumeration_releases_allocated_address(){
  let mut pool=DeviceAddressPool::new();let mut s=EnumerationSession::new();
  s.complete(&[18,1,0,2,0,0,0,64],&mut pool).unwrap();
  let a=s.address().unwrap();s.release_address(&mut pool).unwrap();
  assert!(!pool.is_allocated(a));
 }
}
