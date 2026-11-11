//! Synchronous control-transfer execution and completion validation.
use super::{controller::{HostController,TransferResult,TransferStatus},transfer::{DeviceAddress,SetupPacket},UsbError};
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct ControlTransferReport{pub bytes_transferred:usize,pub short_packet:bool}
pub fn validate_control_result(setup:SetupPacket,capacity:usize,result:TransferResult)->Result<ControlTransferReport,UsbError>{
 if setup.length as usize>capacity{return Err(UsbError::BufferTooShort)}
 match result.status{
  TransferStatus::Success=>{},
  TransferStatus::Pending=>return Err(UsbError::TransferFailed),
  TransferStatus::Stall=>return Err(UsbError::Unsupported),
  TransferStatus::NoDevice=>return Err(UsbError::ControllerUnavailable),
  TransferStatus::TimedOut=>return Err(UsbError::Timeout),
  TransferStatus::ControllerError=>return Err(UsbError::TransferFailed),
 }
 let n=result.bytes_transferred;
 if n>setup.length as usize||n>capacity{return Err(UsbError::TransferFailed)}
 if setup.request_type&0x80==0&&n!=setup.length as usize{return Err(UsbError::TransferFailed)}
 if setup.length==0&&n!=0{return Err(UsbError::TransferFailed)}
 Ok(ControlTransferReport{bytes_transferred:n,short_packet:setup.request_type&0x80!=0&&n<setup.length as usize})
}
pub fn execute_control<C:HostController>(controller:&mut C,address:DeviceAddress,setup:SetupPacket,data:&mut[u8])->Result<ControlTransferReport,UsbError>{
 let len=setup.length as usize;
 if len>data.len(){return Err(UsbError::BufferTooShort)}
 let result=controller.control_transfer(address,setup,&mut data[..len])?;
 validate_control_result(setup,len,result)
}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn accepts_short_in(){let s=SetupPacket::get_descriptor(1,0,18);let r=validate_control_result(s,18,TransferResult{status:TransferStatus::Success,bytes_transferred:8}).unwrap();assert!(r.short_packet);assert_eq!(r.bytes_transferred,8);}
 #[test]fn rejects_short_out(){let s=SetupPacket::new(0x21,9,1,0,8);assert_eq!(validate_control_result(s,8,TransferResult{status:TransferStatus::Success,bytes_transferred:4}),Err(UsbError::TransferFailed));}
 #[test]fn rejects_overreported_bytes(){let s=SetupPacket::get_descriptor(1,0,8);assert_eq!(validate_control_result(s,8,TransferResult{status:TransferStatus::Success,bytes_transferred:9}),Err(UsbError::TransferFailed));}
 #[test]fn maps_controller_errors(){let s=SetupPacket::get_descriptor(1,0,8);assert_eq!(validate_control_result(s,8,TransferResult{status:TransferStatus::TimedOut,bytes_transferred:0}),Err(UsbError::Timeout));assert_eq!(validate_control_result(s,8,TransferResult{status:TransferStatus::NoDevice,bytes_transferred:0}),Err(UsbError::ControllerUnavailable));}
}
