//! Legacy PCI Configuration Mechanism #1 (x86 CF8/CFC).
//!
//! Only segment zero is supported by this transport. PCIe extended space and
//! nonzero segments require an ACPI MCFG-backed ECAM transport.
use core::sync::atomic::{AtomicBool,Ordering};
use crate::arch::x86_64::{control,io};
use super::config::{ConfigAccess,ConfigError,PciAddress,PCI_CONFIG_SIZE};

const CONFIG_ADDRESS:u16=0x0cf8;
const CONFIG_DATA:u16=0x0cfc;
static CONFIG_LOCK:AtomicBool=AtomicBool::new(false);

/// Serialize CF8/CFC transactions across CPUs and prevent local IRQ handlers
/// from interrupting a transaction and changing the shared address register.
struct TransactionGuard{restore_interrupts:bool}
impl TransactionGuard{
 fn acquire()->Self{
  // SAFETY: this guard restores the caller's previous IF state in Drop.
  let flags=control::read_rflags();
  let restore_interrupts=flags&(1<<9)!=0;
  // SAFETY: preserving the previous interrupt state is handled by this guard.
  unsafe{control::disable_interrupts()};
  while CONFIG_LOCK.compare_exchange_weak(false,true,Ordering::Acquire,Ordering::Relaxed).is_err(){control::pause();}
  Self{restore_interrupts}
 }
}
impl Drop for TransactionGuard{fn drop(&mut self){CONFIG_LOCK.store(false,Ordering::Release);if self.restore_interrupts{
  // SAFETY: IF was set on entry; the caller's interrupt state is restored.
  unsafe{control::enable_interrupts()};
 }}}

/// Hardware implementation of PCI Configuration Mechanism #1.
#[derive(Clone,Copy,Debug,Default)]pub struct Mechanism1;
impl Mechanism1{
 /// Encode a CF8 configuration-address register value.
 pub const fn address_value(address:PciAddress,register:u16)->Result<u32,ConfigError>{
  if address.segment!=0{return Err(ConfigError::UnsupportedSegment)}
  if address.device>=32||address.function>=8{return Err(ConfigError::InvalidAddress)}
  if register&3!=0{return Err(ConfigError::UnalignedRegister)}
  if register>=PCI_CONFIG_SIZE{return Err(ConfigError::OutOfRange)}
  Ok(0x8000_0000|((address.bus as u32)<<16)|((address.device as u32)<<11)|((address.function as u32)<<8)|(register as u32&0xfc))
 }
}
impl ConfigAccess for Mechanism1{
 fn read_u32(&self,address:PciAddress,register:u16)->Result<u32,ConfigError>{let selector=Self::address_value(address,register)?;let _guard=TransactionGuard::acquire();
  // SAFETY: CF8/CFC are the defined legacy PCI config ports; transaction lock
  // serializes selector/data pairs and this method is only for ring-0 kernel use.
  unsafe{io::out32(CONFIG_ADDRESS,selector);Ok(io::in32(CONFIG_DATA))}
 }
 fn write_u32(&self,address:PciAddress,register:u16,value:u32)->Result<(),ConfigError>{let selector=Self::address_value(address,register)?;let _guard=TransactionGuard::acquire();
  // SAFETY: CF8/CFC are the defined legacy PCI config ports; transaction lock
  // serializes selector/data pairs and this method is only for ring-0 kernel use.
  unsafe{io::out32(CONFIG_ADDRESS,selector);io::out32(CONFIG_DATA,value)}Ok(())
 }
}

#[cfg(test)]mod tests{use super::*;#[test]fn selector_encodes_bdf_and_register(){let a=PciAddress{segment:0,bus:0x12,device:0x1b,function:5};assert_eq!(Mechanism1::address_value(a,0x34),Ok(0x8012_da34));}#[test]fn rejects_nonzero_segment(){let a=PciAddress{segment:1,bus:0,device:0,function:0};assert_eq!(Mechanism1::address_value(a,0),Err(ConfigError::UnsupportedSegment));}#[test]fn rejects_bad_register(){let a=PciAddress{segment:0,bus:0,device:0,function:0};assert_eq!(Mechanism1::address_value(a,3),Err(ConfigError::UnalignedRegister));assert_eq!(Mechanism1::address_value(a,256),Err(ConfigError::OutOfRange));}}
