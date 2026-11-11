//! USB Mass Storage Bulk-Only Transport (BOT) and core SCSI command framing.
use super::{control::execute_control,controller::{HostController,TransferStatus},descriptors::ParsedConfiguration,
 transfer::{DeviceAddress,Endpoint,EndpointAddress,SetupPacket,TransferType},UsbError};

pub const USB_CLASS_MASS_STORAGE:u8=8;
pub const MSC_SUBCLASS_SCSI:u8=6;
pub const MSC_PROTOCOL_BULK_ONLY:u8=0x50;
pub const CBW_SIGNATURE:u32=0x43425355;
pub const CSW_SIGNATURE:u32=0x53425355;
pub const CBW_LENGTH:usize=31;
pub const CSW_LENGTH:usize=13;

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct MassStorageInterface{pub interface_number:u8,pub bulk_in:Endpoint,pub bulk_out:Endpoint}
pub fn find_mass_storage_interface(config:&ParsedConfiguration)->Option<MassStorageInterface>{
 for iface in config.interfaces[..config.interface_len as usize].iter().flatten(){
  let d=iface.descriptor;
  if d.class!=USB_CLASS_MASS_STORAGE||d.subclass!=MSC_SUBCLASS_SCSI||d.protocol!=MSC_PROTOCOL_BULK_ONLY{continue}
  let mut input=None;let mut output=None;
  let start=iface.first_endpoint as usize;let end=start+iface.endpoint_count as usize;
  for ep in config.endpoints[start..end].iter().flatten(){
   if ep.attributes&3!=TransferType::Bulk as u8{continue}
   let endpoint=Endpoint{address:EndpointAddress::from_raw(ep.address),transfer_type:TransferType::Bulk,max_packet_size:ep.max_packet_size,interval:ep.interval};
   if ep.address&0x80!=0{input=Some(endpoint)}else{output=Some(endpoint)}
  }
  if let (Some(bulk_in),Some(bulk_out))=(input,output){return Some(MassStorageInterface{interface_number:d.number,bulk_in,bulk_out})}
 }
 None
}

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub enum DataDirection{None,In,Out}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct ScsiCommand{pub bytes:[u8;16],pub length:u8}
impl ScsiCommand{
 pub const fn test_unit_ready()->Self{let mut b=[0;16];b[0]=0x00;Self{bytes:b,length:6}}
 pub const fn inquiry(allocation_length:u8)->Self{let mut b=[0;16];b[0]=0x12;b[4]=allocation_length;Self{bytes:b,length:6}}
 pub const fn request_sense(allocation_length:u8)->Self{let mut b=[0;16];b[0]=0x03;b[4]=allocation_length;Self{bytes:b,length:6}}
 pub const fn read_capacity_10()->Self{let mut b=[0;16];b[0]=0x25;Self{bytes:b,length:10}}
 pub fn read_10(lba:u32,blocks:u16)->Self{let mut b=[0;16];b[0]=0x28;b[2..6].copy_from_slice(&lba.to_be_bytes());b[7..9].copy_from_slice(&blocks.to_be_bytes());Self{bytes:b,length:10}}
 pub fn write_10(lba:u32,blocks:u16)->Self{let mut b=[0;16];b[0]=0x2a;b[2..6].copy_from_slice(&lba.to_be_bytes());b[7..9].copy_from_slice(&blocks.to_be_bytes());Self{bytes:b,length:10}}
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct CommandStatusWrapper{pub tag:u32,pub residue:u32,pub status:u8}
impl CommandStatusWrapper{
 pub fn parse(bytes:&[u8],expected_tag:u32)->Result<Self,UsbError>{
  if bytes.len()<CSW_LENGTH{return Err(UsbError::BufferTooShort)}
  let signature=u32::from_le_bytes([bytes[0],bytes[1],bytes[2],bytes[3]]);
  let tag=u32::from_le_bytes([bytes[4],bytes[5],bytes[6],bytes[7]]);
  let residue=u32::from_le_bytes([bytes[8],bytes[9],bytes[10],bytes[11]]);
  if signature!=CSW_SIGNATURE||tag!=expected_tag||bytes[12]>2{return Err(UsbError::TransferFailed)}
  Ok(Self{tag,residue,status:bytes[12]})
 }
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct BotResult{pub tag:u32,pub bytes_transferred:u32,pub residue:u32,pub command_failed:bool}
pub struct BulkOnlyTransport{pub device:DeviceAddress,pub interface:MassStorageInterface,pub lun:u8,next_tag:u32}
impl BulkOnlyTransport{
 pub const fn new(device:DeviceAddress,interface:MassStorageInterface)->Self{Self{device,interface,lun:0,next_tag:1}}
 pub fn select_lun(&mut self,lun:u8)->Result<(),UsbError>{if lun>15{return Err(UsbError::MalformedDescriptor)}self.lun=lun;Ok(())}
 pub fn max_lun<C:HostController>(&self,controller:&mut C)->Result<u8,UsbError>{
  let mut data=[0u8;1];
  let report=match execute_control(controller,self.device,get_max_lun_request(self.interface.interface_number),&mut data){
   Ok(report)=>report,Err(UsbError::Unsupported)=>return Ok(0),Err(error)=>return Err(error),
  };
  if report.bytes_transferred!=1{return Err(UsbError::TransferFailed)}
  parse_max_lun(&data)
 }
 /// BOT reset recovery order: class reset, clear bulk-IN halt, clear bulk-OUT halt.
 pub fn recover<C:HostController>(&self,controller:&mut C)->Result<(),UsbError>{
  let mut empty=[];
  execute_control(controller,self.device,bulk_only_reset_request(self.interface.interface_number),&mut empty)?;
  execute_control(controller,self.device,clear_endpoint_halt_request(self.interface.bulk_in.address.raw()),&mut empty)?;
  execute_control(controller,self.device,clear_endpoint_halt_request(self.interface.bulk_out.address.raw()),&mut empty)?;
  Ok(())
 }
 pub fn read_blocks<C:HostController>(&mut self,controller:&mut C,lba:u32,blocks:u16,block_size:u32,data:&mut[u8])->Result<BotResult,UsbError>{
  let expected=(blocks as usize).checked_mul(block_size as usize).ok_or(UsbError::NoResources)?;
  if expected!=data.len()||blocks==0{return Err(UsbError::BufferTooShort)}
  self.execute(controller,ScsiCommand::read_10(lba,blocks),DataDirection::In,data)
 }
 pub fn write_blocks<C:HostController>(&mut self,controller:&mut C,lba:u32,blocks:u16,block_size:u32,data:&mut[u8])->Result<BotResult,UsbError>{
  let expected=(blocks as usize).checked_mul(block_size as usize).ok_or(UsbError::NoResources)?;
  if expected!=data.len()||blocks==0{return Err(UsbError::BufferTooShort)}
  self.execute(controller,ScsiCommand::write_10(lba,blocks),DataDirection::Out,data)
 }
 pub fn execute<C:HostController>(&mut self,controller:&mut C,command:ScsiCommand,direction:DataDirection,data:&mut[u8])->Result<BotResult,UsbError>{
  if command.length==0||command.length>16{return Err(UsbError::MalformedDescriptor)}
  if (direction==DataDirection::None)!=data.is_empty(){return Err(UsbError::MalformedDescriptor)}
  let length=u32::try_from(data.len()).map_err(|_|UsbError::NoResources)?;
  let tag=self.next_tag;self.next_tag=self.next_tag.wrapping_add(1);if self.next_tag==0{self.next_tag=1;}
  let mut cbw=[0u8;CBW_LENGTH];
  cbw[0..4].copy_from_slice(&CBW_SIGNATURE.to_le_bytes());
  cbw[4..8].copy_from_slice(&tag.to_le_bytes());
  cbw[8..12].copy_from_slice(&length.to_le_bytes());
  cbw[12]=if direction==DataDirection::In{0x80}else{0};
  cbw[13]=self.lun;cbw[14]=command.length;cbw[15..15+command.length as usize].copy_from_slice(&command.bytes[..command.length as usize]);
  let cbw_result=controller.bulk_transfer(self.device,self.interface.bulk_out,&mut cbw)?;
  require_exact(cbw_result,CBW_LENGTH)?;
  let mut transferred=0u32;
  if length!=0{
   let result=match direction{
    DataDirection::In=>controller.bulk_transfer(self.device,self.interface.bulk_in,data)?,
    DataDirection::Out=>controller.bulk_transfer(self.device,self.interface.bulk_out,data)?,
    DataDirection::None=>return Err(UsbError::MalformedDescriptor),
   };
   require_success(result)?;
   if result.bytes_transferred>data.len(){return Err(UsbError::TransferFailed)}
   if direction==DataDirection::Out&&result.bytes_transferred!=data.len(){return Err(UsbError::TransferFailed)}
   transferred=result.bytes_transferred as u32;
  }
  let mut csw=[0u8;CSW_LENGTH];
  let csw_result=controller.bulk_transfer(self.device,self.interface.bulk_in,&mut csw)?;
  require_exact(csw_result,CSW_LENGTH)?;
  let status=CommandStatusWrapper::parse(&csw,tag)?;
  if status.residue>length||status.residue!=length.saturating_sub(transferred){return Err(UsbError::TransferFailed)}
  if status.status==2{return Err(UsbError::TransferFailed)}
  Ok(BotResult{tag,bytes_transferred:transferred,residue:status.residue,command_failed:status.status==1})
 }
}
fn require_success(result:super::controller::TransferResult)->Result<(),UsbError>{
 match result.status{
  TransferStatus::Success=>Ok(()),TransferStatus::TimedOut=>Err(UsbError::Timeout),
  TransferStatus::NoDevice=>Err(UsbError::ControllerUnavailable),TransferStatus::Stall=>Err(UsbError::Unsupported),
  TransferStatus::Pending|TransferStatus::ControllerError=>Err(UsbError::TransferFailed),
 }
}
fn require_exact(result:super::controller::TransferResult,expected:usize)->Result<(),UsbError>{
 require_success(result)?;if result.bytes_transferred!=expected{Err(UsbError::TransferFailed)}else{Ok(())}
}

#[cfg(test)]
mod tests{
 use super::*;
 #[test]fn creates_scsi_read_write_cdbs(){
  let read=ScsiCommand::read_10(0x12345678,0x0102);
  assert_eq!(read.length,10);assert_eq!(&read.bytes[2..6],&[0x12,0x34,0x56,0x78]);assert_eq!(&read.bytes[7..9],&[1,2]);
  let write=ScsiCommand::write_10(7,1);assert_eq!(write.bytes[0],0x2a);
 }
 #[test]fn parses_and_checks_bot_command_status(){
  let tag=0x12345678;let mut csw=[0u8;CSW_LENGTH];
  csw[0..4].copy_from_slice(&CSW_SIGNATURE.to_le_bytes());csw[4..8].copy_from_slice(&tag.to_le_bytes());
  csw[8..12].copy_from_slice(&0u32.to_le_bytes());csw[12]=0;
  assert_eq!(CommandStatusWrapper::parse(&csw,tag).unwrap(),CommandStatusWrapper{tag,residue:0,status:0});
  assert_eq!(CommandStatusWrapper::parse(&csw,tag+1),Err(UsbError::TransferFailed));
 }
 #[test]fn rejects_invalid_bot_status_and_signature(){
  let mut csw=[0u8;CSW_LENGTH];csw[0..4].copy_from_slice(&CSW_SIGNATURE.to_le_bytes());
  csw[12]=3;assert_eq!(CommandStatusWrapper::parse(&csw,0),Err(UsbError::TransferFailed));
 }
 #[test]fn builds_six_byte_scsi_commands(){
  assert_eq!(ScsiCommand::test_unit_ready().length,6);
  assert_eq!(ScsiCommand::inquiry(36).bytes[4],36);
  assert_eq!(ScsiCommand::request_sense(18).bytes[0],3);
  assert_eq!(ScsiCommand::read_capacity_10().bytes[0],0x25);
 }
}

pub const MSC_GET_MAX_LUN:u8=0xfe;
pub const MSC_BULK_ONLY_RESET:u8=0xff;
pub const USB_CLEAR_FEATURE:u8=1;
pub const USB_FEATURE_ENDPOINT_HALT:u16=0;
pub const fn get_max_lun_request(interface:u8)->SetupPacket{SetupPacket::new(0xa1,MSC_GET_MAX_LUN,0,interface as u16,1)}
pub const fn bulk_only_reset_request(interface:u8)->SetupPacket{SetupPacket::new(0x21,MSC_BULK_ONLY_RESET,0,interface as u16,0)}
pub const fn clear_endpoint_halt_request(endpoint:u8)->SetupPacket{SetupPacket::new(0x02,USB_CLEAR_FEATURE,USB_FEATURE_ENDPOINT_HALT,endpoint as u16,0)}
pub fn parse_max_lun(bytes:&[u8])->Result<u8,UsbError>{if bytes.is_empty(){return Err(UsbError::BufferTooShort)}if bytes[0]>15{return Err(UsbError::MalformedDescriptor)}Ok(bytes[0])}

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct ScsiCapacity10{pub last_lba:u32,pub block_size:u32}
impl ScsiCapacity10{
 pub fn parse(bytes:&[u8])->Result<Self,UsbError>{
  if bytes.len()<8{return Err(UsbError::BufferTooShort)}
  let last_lba=u32::from_be_bytes([bytes[0],bytes[1],bytes[2],bytes[3]]);
  let block_size=u32::from_be_bytes([bytes[4],bytes[5],bytes[6],bytes[7]]);
  if block_size==0{return Err(UsbError::MalformedDescriptor)}
  Ok(Self{last_lba,block_size})
 }
 pub const fn block_count(self)->u64{self.last_lba as u64+1}
}

#[cfg(test)]mod recovery_and_capacity_tests{
 use super::*;
 #[test]fn builds_bot_recovery_and_lun_requests(){
  let reset=bulk_only_reset_request(4);assert_eq!(reset.request_type,0x21);assert_eq!(reset.request,0xff);assert_eq!(reset.index,4);
  let clear=clear_endpoint_halt_request(0x81);assert_eq!(clear.request_type,2);assert_eq!(clear.index,0x81);
  assert_eq!(parse_max_lun(&[3]),Ok(3));assert_eq!(parse_max_lun(&[16]),Err(UsbError::MalformedDescriptor));
 }
 #[test]fn parses_scsi_capacity_without_overflow(){
  let c=ScsiCapacity10::parse(&[0,0,0,99,0,0,2,0]).unwrap();
  assert_eq!(c.block_count(),100);assert_eq!(c.block_size,512);
  assert_eq!(ScsiCapacity10::parse(&[0;8]),Err(UsbError::MalformedDescriptor));
 }
}

#[cfg(test)]mod lun_selection_tests{
 use super::*;
 #[test]fn accepts_only_valid_lun_ids(){
  let interface=MassStorageInterface{interface_number:0,
   bulk_in:Endpoint{address:EndpointAddress::from_raw(0x81),transfer_type:TransferType::Bulk,max_packet_size:512,interval:0},
   bulk_out:Endpoint{address:EndpointAddress::from_raw(0x02),transfer_type:TransferType::Bulk,max_packet_size:512,interval:0}};
  let mut bot=BulkOnlyTransport::new(DeviceAddress::DEFAULT,interface);
  assert_eq!(bot.select_lun(4),Ok(()));assert_eq!(bot.lun,4);
  assert_eq!(bot.select_lun(16),Err(UsbError::MalformedDescriptor));
 }
}
