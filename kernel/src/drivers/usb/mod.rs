//! USB bus subsystem. Protocol handling is independent of host-controller hardware.
//! xHCI is the first controller target; additional controller families can reuse
//! the same device, descriptor, endpoint, and transfer model.
pub mod descriptors;
pub mod transfer;
pub mod xhci;
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub enum UsbError{BufferTooShort,MalformedDescriptor,InvalidAddress,Unsupported,NoResources,ControllerUnavailable,TransferFailed,Timeout}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub enum DeviceState{Attached,Addressed,Configured,Suspended,Disconnected}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct UsbDevice{
 pub address:transfer::DeviceAddress,pub speed:transfer::UsbSpeed,pub state:DeviceState,
 pub vendor_id:u16,pub product_id:u16,pub class:u8,pub subclass:u8,pub protocol:u8,
}
impl UsbDevice{
 pub fn from_descriptor(address:transfer::DeviceAddress,speed:transfer::UsbSpeed,descriptor:descriptors::DeviceDescriptor)->Self{
  Self{address,speed,state:DeviceState::Attached,vendor_id:descriptor.vendor_id,product_id:descriptor.product_id,class:descriptor.device_class,subclass:descriptor.device_subclass,protocol:descriptor.device_protocol}
 }
}
