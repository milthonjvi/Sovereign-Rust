//! Fixed-capacity USB device topology and cascading disconnect handling.
//! This module owns only USB bookkeeping; controller discovery, DMA and IRQ
//! services remain with their respective kernel subsystems.
use super::{DeviceState,UsbDevice,UsbError,enumeration::DeviceAddressPool,transfer::DeviceAddress};

pub const MAX_USB_DEVICES:usize=127;
pub const MAX_USB_HUB_DEPTH:u8=7;

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct UsbTopologyNode{
 pub device:UsbDevice,
 /// None means the device is connected directly to a host-controller root port.
 pub parent_hub:Option<DeviceAddress>,
 /// USB port numbers are one-based, including root-hub ports.
 pub port:u8,
 pub is_hub:bool,
 pub depth:u8,
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct UsbTopology{nodes:[Option<UsbTopologyNode>;128],len:u8}
impl UsbTopology{
 pub const fn new()->Self{Self{nodes:[None;128],len:0}}
 pub const fn len(&self)->usize{self.len as usize}
 pub const fn is_empty(&self)->bool{self.len==0}
 pub fn get(&self,address:DeviceAddress)->Option<&UsbTopologyNode>{
  self.nodes.get(address.get() as usize)?.as_ref()
 }
 /// Adds a device after successful enumeration. A child may only be attached
 /// below a registered hub, and duplicate parent/port pairs are rejected.
 pub fn attach(&mut self,node:UsbTopologyNode)->Result<(),UsbError>{
  let address=node.device.address.get() as usize;
  if address==0||address>127||node.port==0{return Err(UsbError::InvalidAddress)}
  if self.nodes[address].is_some(){return Err(UsbError::InvalidAddress)}
  if self.len as usize>=MAX_USB_DEVICES{return Err(UsbError::NoResources)}
  match node.parent_hub{
   Some(parent)=>{
    let p=self.get(parent).ok_or(UsbError::InvalidAddress)?;
    if !p.is_hub||node.depth!=p.depth.saturating_add(1)||node.depth>MAX_USB_HUB_DEPTH{
     return Err(UsbError::MalformedDescriptor)
    }
   }
   None=>{if node.depth!=0{return Err(UsbError::MalformedDescriptor)}}
  }
  for existing in self.nodes.iter().flatten(){
   if existing.parent_hub==node.parent_hub&&existing.port==node.port{
    return Err(UsbError::InvalidAddress)
   }
  }
  self.nodes[address]=Some(node);self.len+=1;Ok(())
 }
 /// Marks the device and all descendants disconnected, removes them from the
 /// topology, and returns the number of released USB addresses.
 pub fn detach(&mut self,address:DeviceAddress,pool:&mut DeviceAddressPool)->Result<usize,UsbError>{
  let index=address.get() as usize;
  if index==0||index>127||self.nodes[index].is_none(){return Err(UsbError::InvalidAddress)}
  let mut removed=[false;128];removed[index]=true;
  // Fixed-point traversal handles arbitrarily ordered entries without recursion.
  loop{
   let mut changed=false;
   for i in 1..128{
    if removed[i]{continue}
    if let Some(node)=self.nodes[i]{
     if let Some(parent)=node.parent_hub{
      if removed[parent.get() as usize]{removed[i]=true;changed=true;}
     }
    }
   }
   if !changed{break}
  }
  let mut count=0;
  for i in 1..128{
   if removed[i]{
    if let Some(mut node)=self.nodes[i].take(){
     node.device.state=DeviceState::Disconnected;
     let _=pool.release(node.device.address);
     self.len-=1;count+=1;
    }
   }
  }
  Ok(count)
 }
 pub fn find_child(&self,parent:Option<DeviceAddress>,port:u8)->Option<&UsbTopologyNode>{
  self.nodes.iter().flatten().find(|n|n.parent_hub==parent&&n.port==port)
 }
 pub fn iter(&self)->impl Iterator<Item=&UsbTopologyNode>{
  self.nodes.iter().filter_map(|node|node.as_ref())
 }
}
impl Default for UsbTopology{fn default()->Self{Self::new()}}

#[cfg(test)]
mod tests{
 use super::*;
 use crate::drivers::usb::transfer::UsbSpeed;
 fn device(address:DeviceAddress)->UsbDevice{
  UsbDevice{address,speed:UsbSpeed::High,state:DeviceState::Configured,
   vendor_id:1,product_id:2,class:0,subclass:0,protocol:0}
 }
 #[test]fn attaches_root_hub_and_nested_device(){
  let mut pool=DeviceAddressPool::new();
  let root=pool.allocate().unwrap();let child=pool.allocate().unwrap();
  let mut topology=UsbTopology::new();
  topology.attach(UsbTopologyNode{device:device(root),parent_hub:None,port:1,is_hub:true,depth:0}).unwrap();
  topology.attach(UsbTopologyNode{device:device(child),parent_hub:Some(root),port:3,is_hub:false,depth:1}).unwrap();
  assert_eq!(topology.len(),2);assert!(topology.find_child(Some(root),3).is_some());
 }
 #[test]fn detaching_hub_releases_entire_descendant_tree(){
  let mut pool=DeviceAddressPool::new();
  let root=pool.allocate().unwrap();let child=pool.allocate().unwrap();let leaf=pool.allocate().unwrap();
  let mut topology=UsbTopology::new();
  topology.attach(UsbTopologyNode{device:device(root),parent_hub:None,port:1,is_hub:true,depth:0}).unwrap();
  topology.attach(UsbTopologyNode{device:device(child),parent_hub:Some(root),port:2,is_hub:true,depth:1}).unwrap();
  topology.attach(UsbTopologyNode{device:device(leaf),parent_hub:Some(child),port:4,is_hub:false,depth:2}).unwrap();
  assert_eq!(topology.detach(root,&mut pool).unwrap(),3);
  assert!(topology.is_empty());assert!(!pool.is_allocated(root));
  assert!(!pool.is_allocated(child));assert!(!pool.is_allocated(leaf));
 }
 #[test]fn rejects_devices_attached_to_non_hub(){
  let mut pool=DeviceAddressPool::new();let parent=pool.allocate().unwrap();let child=pool.allocate().unwrap();
  let mut topology=UsbTopology::new();
  topology.attach(UsbTopologyNode{device:device(parent),parent_hub:None,port:1,is_hub:false,depth:0}).unwrap();
  assert_eq!(topology.attach(UsbTopologyNode{device:device(child),parent_hub:Some(parent),port:1,is_hub:false,depth:1}),Err(UsbError::MalformedDescriptor));
 }
}
