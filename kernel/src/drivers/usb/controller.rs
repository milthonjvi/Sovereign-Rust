//! Host-controller-independent USB transfer contract.
use super::{transfer::{DeviceAddress,Endpoint,SetupPacket,UsbSpeed},UsbError};
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub enum ControllerKind{Uhci,Ohci,Ehci,Xhci}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub enum UsbRevision{V1_0,V1_1,V2_0,V3_0,V3_1,V3_2,V4_0}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct PortStatus{pub connected:bool,pub enabled:bool,pub suspended:bool,pub over_current:bool,pub speed:Option<UsbSpeed>}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub enum TransferStatus{Pending,Success,Stall,NoDevice,TimedOut,ControllerError}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct TransferResult{pub status:TransferStatus,pub bytes_transferred:usize}
/// Implemented by each host-controller driver. Buffers passed to these methods
/// must be DMA-safe according to the controller implementation's contract.
pub trait HostController{
 fn kind(&self)->ControllerKind;
 fn port_count(&self)->u8;
 fn port_status(&self,port:u8)->Result<PortStatus,UsbError>;
 fn control_transfer(&mut self,address:DeviceAddress,setup:SetupPacket,data:&mut[u8])->Result<TransferResult,UsbError>;
 fn bulk_transfer(&mut self,address:DeviceAddress,endpoint:Endpoint,data:&mut[u8])->Result<TransferResult,UsbError>;
 fn interrupt_transfer(&mut self,address:DeviceAddress,endpoint:Endpoint,data:&mut[u8])->Result<TransferResult,UsbError>;
 fn reset_port(&mut self,port:u8)->Result<(),UsbError>;
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct ControllerCompatibility{pub kind:ControllerKind,pub revisions:&'static[UsbRevision]}
pub const CONTROLLER_COMPATIBILITY:&[ControllerCompatibility]=&[
 ControllerCompatibility{kind:ControllerKind::Uhci,revisions:&[UsbRevision::V1_0,UsbRevision::V1_1]},
 ControllerCompatibility{kind:ControllerKind::Ohci,revisions:&[UsbRevision::V1_0,UsbRevision::V1_1]},
 ControllerCompatibility{kind:ControllerKind::Ehci,revisions:&[UsbRevision::V2_0]},
 ControllerCompatibility{kind:ControllerKind::Xhci,revisions:&[UsbRevision::V1_0,UsbRevision::V1_1,UsbRevision::V2_0,UsbRevision::V3_0,UsbRevision::V3_1,UsbRevision::V3_2]},
];
