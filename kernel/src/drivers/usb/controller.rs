//! Host-controller-independent USB transfer contract.
use super::{transfer::{DeviceAddress,Endpoint,SetupPacket,UsbSpeed},UsbError};
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub enum ControllerKind{Uhci,Ohci,Ehci,Xhci}
impl ControllerKind{
 /// PCI class 0x0c, subclass 0x03, programming-interface mapping.
 pub const fn from_pci(class:u8,subclass:u8,programming_interface:u8)->Option<Self>{
  if class!=0x0c||subclass!=0x03{return None}
  match programming_interface{0x00=>Some(Self::Uhci),0x10=>Some(Self::Ohci),0x20=>Some(Self::Ehci),0x30=>Some(Self::Xhci),_=>None}
 }
}
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

#[cfg(test)]
mod tests{
 use super::*;
 #[test]fn pci_programming_interfaces_select_legacy_and_modern_hosts(){
  assert_eq!(ControllerKind::from_pci(0x0c,0x03,0x00),Some(ControllerKind::Uhci));
  assert_eq!(ControllerKind::from_pci(0x0c,0x03,0x10),Some(ControllerKind::Ohci));
  assert_eq!(ControllerKind::from_pci(0x0c,0x03,0x20),Some(ControllerKind::Ehci));
  assert_eq!(ControllerKind::from_pci(0x0c,0x03,0x30),Some(ControllerKind::Xhci));
  assert_eq!(ControllerKind::from_pci(0x02,0x00,0x30),None);
  assert_eq!(ControllerKind::from_pci(0x0c,0x03,0x80),None);
 }
 #[test]fn xhci_compatibility_includes_usb1_and_usb2_fallback(){
  let entry=CONTROLLER_COMPATIBILITY.iter().find(|v|v.kind==ControllerKind::Xhci).unwrap();
  assert!(entry.revisions.contains(&UsbRevision::V1_1));
  assert!(entry.revisions.contains(&UsbRevision::V2_0));
  assert!(entry.revisions.contains(&UsbRevision::V3_2));
  assert!(!entry.revisions.contains(&UsbRevision::V4_0));
 }
}
