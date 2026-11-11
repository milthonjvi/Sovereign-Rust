//! Synchronous control-transfer execution and completion validation.
use super::{controller::{HostController,TransferResult,TransferStatus},transfer::{DeviceAddress,EndpointAddress,SetupPacket},UsbError};
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
/// Builds the standard CLEAR_FEATURE(ENDPOINT_HALT) request.
/// Endpoint zero is never valid for this request.
pub fn clear_endpoint_halt_request(endpoint:EndpointAddress)->Result<SetupPacket,UsbError>{
 let raw=endpoint.raw();
 if endpoint.number()==0||raw&0x70!=0{return Err(UsbError::MalformedDescriptor)}
 Ok(SetupPacket::new(0x02,1,0,raw as u16,0))
}

/// Clears a halted non-control endpoint. The host-controller implementation
/// must also restore its endpoint data-toggle state to DATA0 as required by USB.
pub fn execute_clear_endpoint_halt<C:HostController>(
 controller:&mut C,address:DeviceAddress,endpoint:EndpointAddress,
)->Result<(),UsbError>{
 let setup=clear_endpoint_halt_request(endpoint)?;
 execute_control(controller,address,setup,&mut[])?;
 Ok(())
}

pub fn execute_control<C:HostController>(controller:&mut C,address:DeviceAddress,setup:SetupPacket,data:&mut[u8])->Result<ControlTransferReport,UsbError>{
 let len=setup.length as usize;
 if len>data.len(){return Err(UsbError::BufferTooShort)}
 let result=controller.control_transfer(address,setup,&mut data[..len])?;
 validate_control_result(setup,len,result)
}
#[cfg(test)]mod endpoint_recovery_tests{
 use super::*;
 use crate::drivers::usb::transfer::EndpointAddress;
 #[test]fn builds_clear_halt_for_valid_nonzero_endpoint(){
  let setup=clear_endpoint_halt_request(EndpointAddress::from_raw(0x83)).unwrap();
  assert_eq!(setup.request_type,0x02);assert_eq!(setup.request,1);
  assert_eq!(setup.value,0);assert_eq!(setup.index,0x83);assert_eq!(setup.length,0);
 }
 #[test]fn rejects_endpoint_zero_and_reserved_address_bits(){
  assert_eq!(clear_endpoint_halt_request(EndpointAddress::from_raw(0x00)),Err(UsbError::MalformedDescriptor));
  assert_eq!(clear_endpoint_halt_request(EndpointAddress::from_raw(0x91)),Err(UsbError::MalformedDescriptor));
 }
}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn accepts_short_in(){let s=SetupPacket::get_descriptor(1,0,18);let r=validate_control_result(s,18,TransferResult{status:TransferStatus::Success,bytes_transferred:8}).unwrap();assert!(r.short_packet);assert_eq!(r.bytes_transferred,8);}
 #[test]fn rejects_short_out(){let s=SetupPacket::new(0x21,9,1,0,8);assert_eq!(validate_control_result(s,8,TransferResult{status:TransferStatus::Success,bytes_transferred:4}),Err(UsbError::TransferFailed));}
 #[test]fn rejects_overreported_bytes(){let s=SetupPacket::get_descriptor(1,0,8);assert_eq!(validate_control_result(s,8,TransferResult{status:TransferStatus::Success,bytes_transferred:9}),Err(UsbError::TransferFailed));}
 #[test]fn maps_controller_errors(){let s=SetupPacket::get_descriptor(1,0,8);assert_eq!(validate_control_result(s,8,TransferResult{status:TransferStatus::TimedOut,bytes_transferred:0}),Err(UsbError::Timeout));assert_eq!(validate_control_result(s,8,TransferResult{status:TransferStatus::NoDevice,bytes_transferred:0}),Err(UsbError::ControllerUnavailable));}
}
