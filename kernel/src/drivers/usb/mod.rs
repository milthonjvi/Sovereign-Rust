//! USB bus subsystem spanning legacy USB 1.x through USB 3.x host controllers.
//! USB4 tunnelling is usable when platform firmware exposes a compatible xHCI
//! controller; native USB4 router management is a separate future subsystem.
pub mod controller;
pub mod descriptors;
pub mod enumeration;
pub mod ehci;
pub mod hub;
pub mod ohci;
pub mod transfer;
pub mod uhci;
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
