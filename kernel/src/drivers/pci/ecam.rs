//! PCIe ECAM access-window discovery from the ACPI MCFG table.
use super::config::{ConfigError,EcamWindow,PciAddress};
use crate::drivers::acpi::{checksum_valid,AcpiError};
pub const MCFG_HEADER_SIZE:usize=44;pub const MCFG_ENTRY_SIZE:usize=16;
#[derive(Clone,Copy,Debug,Eq,PartialEq)]pub enum McfgError{Acpi(AcpiError),InvalidSignature,InvalidLength,InvalidEntry,OutputTooSmall}
impl From<AcpiError> for McfgError{fn from(v:AcpiError)->Self{Self::Acpi(v)}}
fn u16le(b:&[u8],i:usize)->u16{u16::from_le_bytes([b[i],b[i+1]])}fn u32le(b:&[u8],i:usize)->u32{u32::from_le_bytes([b[i],b[i+1],b[i+2],b[i+3]])}fn u64le(b:&[u8],i:usize)->u64{u64::from_le_bytes([b[i],b[i+1],b[i+2],b[i+3],b[i+4],b[i+5],b[i+6],b[i+7]])}
/// Parse validated MCFG bytes into caller-owned storage; never dereferences physical addresses.
pub fn parse_mcfg(table:&[u8],out:&mut[EcamWindow])->Result<usize,McfgError>{if table.len()<44{return Err(McfgError::Acpi(AcpiError::BufferTooSmall))}if &table[..4]!=b"MCFG"{return Err(McfgError::InvalidSignature)}let len=u32le(table,4)as usize;if len<44||len>table.len()||(len-44)%16!=0{return Err(McfgError::InvalidLength)}if !checksum_valid(&table[..len]){return Err(McfgError::Acpi(AcpiError::InvalidChecksum))}if table[36..44].iter().any(|v|*v!=0){return Err(McfgError::InvalidEntry)}let count=(len-44)/16;if count>out.len(){return Err(McfgError::OutputTooSmall)}for(i,e)in table[44..len].chunks_exact(16).enumerate(){let base=u64le(e,0);let start=e[10];let end=e[11];if base&0xfffff!=0||start>end||u32le(e,12)!=0{return Err(McfgError::InvalidEntry)}out[i]=EcamWindow{segment:u16le(e,8),start_bus:start,end_bus:end,physical_base:base}}Ok(count)}
pub fn physical_address(w:EcamWindow,a:PciAddress,r:u16)->Result<u64,ConfigError>{w.physical_base.checked_add(w.offset(a,r).ok_or(ConfigError::OutOfRange)?).ok_or(ConfigError::OutOfRange)}
