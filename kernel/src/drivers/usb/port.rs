//! Non-blocking USB downstream-port power, debounce, reset and enumeration policy.
//! The caller performs returned actions through a root-hub or external-hub
//! controller and supplies elapsed time; this module never busy-waits.
use super::{UsbError,hub::{HubPortAction,port_action},transfer::SetupPacket};

pub const USB_CONNECT_DEBOUNCE_MS:u16=100;
pub const USB_RESET_RECOVERY_MS:u16=10;

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct PortObservation{
 pub connected:bool,pub enabled:bool,pub reset_asserted:bool,
 pub connection_changed:bool,pub reset_changed:bool,
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub enum PortPhase{Unpowered,Powering,Debouncing,Resetting,ResetRecovery,Enumerating,Active,Disconnected,Failed}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub enum PortAction{PowerOn,PowerOff,ClearConnectionChange,Reset,ClearResetChange,BeginEnumeration,DisconnectDevice}
impl PortAction{
 /// Converts an actionable state-machine output into an external-hub class
 /// request. Root-hub actions must instead use the host-controller interface.
 pub fn hub_request(self,port:u8)->Option<SetupPacket>{
  let action=match self{
   Self::PowerOn=>HubPortAction::PowerOn,
   Self::PowerOff=>HubPortAction::PowerOff,
   Self::Reset=>HubPortAction::Reset,
   Self::ClearConnectionChange=>HubPortAction::ClearConnectionChange,
   Self::ClearResetChange=>HubPortAction::ClearResetChange,
   Self::BeginEnumeration|Self::DisconnectDevice=>return None,
  };
  Some(port_action(port,action))
 }
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct UsbPortMachine{
 pub port:u8,pub phase:PortPhase,power_good_ms:u16,elapsed_ms:u16,stable_ms:u16,
}
impl UsbPortMachine{
 /// The parameter is the hub descriptor's bPwrOn2PwrGood field (units of 2 ms).
 pub const fn new(port:u8,power_on_to_good_2ms:u8)->Self{
  Self{port,phase:PortPhase::Unpowered,power_good_ms:(power_on_to_good_2ms as u16)*2,elapsed_ms:0,stable_ms:0}
 }
 pub fn start(&mut self)->Result<PortAction,UsbError>{
  if self.port==0||self.phase!=PortPhase::Unpowered{return Err(UsbError::Unsupported)}
  self.phase=PortPhase::Powering;self.elapsed_ms=0;Ok(PortAction::PowerOn)
 }
 pub fn power_off(&mut self)->Result<PortAction,UsbError>{
  if self.port==0||self.phase==PortPhase::Unpowered{return Err(UsbError::Unsupported)}
  self.phase=PortPhase::Unpowered;self.elapsed_ms=0;self.stable_ms=0;Ok(PortAction::PowerOff)
 }
 /// Advances a state machine without blocking. Actions are requests for the
 /// caller to execute; state transitions never imply hardware success by themselves.
 pub fn advance(&mut self,elapsed_ms:u16,observation:PortObservation)->Option<PortAction>{
  match self.phase{
   PortPhase::Unpowered=>None,
   PortPhase::Powering=>{
    self.elapsed_ms=self.elapsed_ms.saturating_add(elapsed_ms);
    if self.elapsed_ms>=self.power_good_ms{
     self.phase=PortPhase::Debouncing;self.elapsed_ms=0;self.stable_ms=0;
     if observation.connection_changed{return Some(PortAction::ClearConnectionChange)}
    }
    None
   }
   PortPhase::Debouncing=>{
    if observation.connection_changed{return Some(PortAction::ClearConnectionChange)}
    if !observation.connected{self.stable_ms=0;return None}
    self.stable_ms=self.stable_ms.saturating_add(elapsed_ms);
    if self.stable_ms>=USB_CONNECT_DEBOUNCE_MS{
     self.phase=PortPhase::Resetting;self.elapsed_ms=0;return Some(PortAction::Reset)
    }
    None
   }
   PortPhase::Resetting=>{
    if !observation.connected{
     self.phase=PortPhase::Disconnected;self.elapsed_ms=0;return Some(PortAction::DisconnectDevice)
    }
    if !observation.reset_asserted&&observation.enabled{
     self.phase=PortPhase::ResetRecovery;self.elapsed_ms=0;
     if observation.reset_changed{return Some(PortAction::ClearResetChange)}
    }
    None
   }
   PortPhase::ResetRecovery=>{
    if !observation.connected{
     self.phase=PortPhase::Disconnected;self.elapsed_ms=0;return Some(PortAction::DisconnectDevice)
    }
    self.elapsed_ms=self.elapsed_ms.saturating_add(elapsed_ms);
    if self.elapsed_ms>=USB_RESET_RECOVERY_MS{
     self.phase=PortPhase::Enumerating;return Some(PortAction::BeginEnumeration)
    }
    None
   }
   PortPhase::Enumerating=>None,
   PortPhase::Active=>{
    if !observation.connected{
     self.phase=PortPhase::Disconnected;return Some(PortAction::DisconnectDevice)
    }
    None
   }
   PortPhase::Disconnected=>{
    if observation.connected{
     self.phase=PortPhase::Debouncing;self.stable_ms=0;self.elapsed_ms=0;
     if observation.connection_changed{return Some(PortAction::ClearConnectionChange)}
    }
    None
   }
   PortPhase::Failed=>None
  }
 }
 pub fn retry(&mut self)->Result<(),UsbError>{
  if self.phase!=PortPhase::Failed{return Err(UsbError::Unsupported)}
  self.phase=PortPhase::Debouncing;self.elapsed_ms=0;self.stable_ms=0;Ok(())
 }
 pub fn enumeration_complete(&mut self,success:bool)->Result<(),UsbError>{
  if self.phase!=PortPhase::Enumerating{return Err(UsbError::Unsupported)}
  self.phase=if success{PortPhase::Active}else{PortPhase::Failed};
  self.elapsed_ms=0;self.stable_ms=0;Ok(())
 }
}
#[cfg(test)]
mod tests{
 use super::*;
 const EMPTY:PortObservation=PortObservation{connected:false,enabled:false,reset_asserted:false,connection_changed:false,reset_changed:false};
 #[test]fn applies_power_debounce_reset_and_recovery_without_busy_wait(){
  let mut p=UsbPortMachine::new(2,10);assert_eq!(p.start(),Ok(PortAction::PowerOn));
  p.advance(19,EMPTY);assert_eq!(p.phase,PortPhase::Powering);
  p.advance(1,EMPTY);assert_eq!(p.phase,PortPhase::Debouncing);
  let connected=PortObservation{connected:true,..EMPTY};
  p.advance(50,connected);assert_eq!(p.phase,PortPhase::Debouncing);
  assert_eq!(p.advance(50,connected),Some(PortAction::Reset));
  assert_eq!(p.phase,PortPhase::Resetting);
  let reset_done=PortObservation{connected:true,enabled:true,reset_changed:true,..EMPTY};
  assert_eq!(p.advance(0,reset_done),Some(PortAction::ClearResetChange));
  assert_eq!(p.phase,PortPhase::ResetRecovery);
  p.advance(9,reset_done);assert_eq!(p.phase,PortPhase::ResetRecovery);
  assert_eq!(p.advance(1,reset_done),Some(PortAction::BeginEnumeration));
  p.enumeration_complete(true).unwrap();assert_eq!(p.phase,PortPhase::Active);
 }
 #[test]fn disconnect_during_reset_aborts_enumeration(){
  let mut p=UsbPortMachine::new(1,0);p.start().unwrap();p.advance(0,EMPTY);
  let connected=PortObservation{connected:true,..EMPTY};
  p.advance(100,connected);assert_eq!(p.phase,PortPhase::Resetting);
  assert_eq!(p.advance(1,EMPTY),Some(PortAction::DisconnectDevice));
  assert_eq!(p.phase,PortPhase::Disconnected);
 }
 #[test]fn rejects_invalid_port_and_illegal_enumeration_transition(){
  let mut invalid=UsbPortMachine::new(0,0);assert_eq!(invalid.start(),Err(UsbError::Unsupported));
  let mut p=UsbPortMachine::new(1,0);assert_eq!(p.enumeration_complete(true),Err(UsbError::Unsupported));
 }
}

#[cfg(test)]
mod retry_tests{
 use super::*;
 #[test]fn failed_port_requires_explicit_retry(){
  let mut p=UsbPortMachine::new(1,0);p.start().unwrap();
  let empty=PortObservation{connected:false,enabled:false,reset_asserted:false,connection_changed:false,reset_changed:false};
  let connected=PortObservation{connected:true,..empty};
  p.advance(0,connected);p.advance(100,connected);
  let reset_done=PortObservation{connected:true,enabled:true,reset_asserted:false,connection_changed:false,reset_changed:false};
  p.advance(0,reset_done);p.advance(10,reset_done);
  p.enumeration_complete(false).unwrap();assert_eq!(p.phase,PortPhase::Failed);
  p.advance(100,reset_done);assert_eq!(p.phase,PortPhase::Failed);
  p.retry().unwrap();assert_eq!(p.phase,PortPhase::Debouncing);
 }
}
