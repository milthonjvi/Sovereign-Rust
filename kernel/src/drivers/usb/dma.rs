//! USB DMA contracts shared by host-controller drivers and the memory subsystem.
//! This module validates DMA addresses; it does not allocate or map physical memory.
use super::UsbError;

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct DmaConstraints{
 pub address_bits:u8,
 pub alignment:usize,
 /// Zero means no boundary restriction.
 pub boundary:usize,
 /// Zero means no explicit transfer-size limit.
 pub max_length:usize,
 pub require_coherent:bool,
}
impl DmaConstraints{
 pub const fn legacy_32bit(alignment:usize)->Self{
  Self{address_bits:32,alignment,boundary:0,max_length:0,require_coherent:true}
 }
 pub const fn xhci_64bit(alignment:usize)->Self{
  Self{address_bits:64,alignment,boundary:0,max_length:0,require_coherent:true}
 }
 pub const fn with_boundary(mut self,boundary:usize)->Self{self.boundary=boundary;self}
 pub const fn with_max_length(mut self,max_length:usize)->Self{self.max_length=max_length;self}
 pub fn validate(self,address:u64,length:usize,coherent:bool)->Result<(),UsbError>{
  if self.address_bits==0||self.address_bits>64||self.alignment==0||!self.alignment.is_power_of_two()
   ||(self.boundary!=0&&!self.boundary.is_power_of_two()){
   return Err(UsbError::MalformedDescriptor)
  }
  if length==0||address&((self.alignment as u64)-1)!=0{
   return Err(UsbError::InvalidAddress)
  }
  if self.max_length!=0&&length>self.max_length{return Err(UsbError::NoResources)}
  let end=address.checked_add((length-1)as u64).ok_or(UsbError::InvalidAddress)?;
  let max_address=if self.address_bits==64{u64::MAX}else{(1u64<<self.address_bits)-1};
  if end>max_address{return Err(UsbError::InvalidAddress)}
  if self.boundary!=0{
   let mask=(self.boundary as u64)-1;
   if (address&!mask)!=(end&!mask){return Err(UsbError::InvalidAddress)}
  }
  if self.require_coherent&&!coherent{return Err(UsbError::Unsupported)}
  Ok(())
 }
}

/// # Safety
/// Implementors must expose a pinned DMA mapping for the entire lifetime of the
/// buffer. dma_address must be the device-visible bus address, not a virtual
/// pointer or an unverified CPU physical address. The mapping must satisfy the
/// platform's cache-coherency and ownership rules.
pub unsafe trait UsbDmaBuffer{
 fn dma_address(&self)->u64;
 fn len(&self)->usize;
 fn is_coherent(&self)->bool;
 fn as_mut_slice(&mut self)->&mut[u8];
 fn is_empty(&self)->bool{self.len()==0}
}
pub trait UsbDmaAllocator{
 type Buffer:UsbDmaBuffer;
 fn allocate(&mut self,length:usize,constraints:DmaConstraints)->Result<Self::Buffer,UsbError>;
}
pub fn allocate_validated<A:UsbDmaAllocator>(
 allocator:&mut A,length:usize,constraints:DmaConstraints,
)->Result<A::Buffer,UsbError>{
 let buffer=allocator.allocate(length,constraints)?;
 if buffer.len()<length{return Err(UsbError::BufferTooShort)}
 constraints.validate(buffer.dma_address(),length,buffer.is_coherent())?;
 Ok(buffer)
}

#[cfg(test)]
mod tests{
 use super::*;
 #[test]fn validates_legacy_dma_address_width_and_alignment(){
  let c=DmaConstraints::legacy_32bit(16);
  assert_eq!(c.validate(0x1000,64,true),Ok(()));
  assert_eq!(c.validate(0x1001,64,true),Err(UsbError::InvalidAddress));
  assert_eq!(c.validate(0x1_0000_0000,64,true),Err(UsbError::InvalidAddress));
 }
 #[test]fn enforces_boundary_and_maximum_transfer_size(){
  let c=DmaConstraints::legacy_32bit(1).with_boundary(4096).with_max_length(1024);
  assert_eq!(c.validate(0x1f00,0x100,true),Ok(()));
  assert_eq!(c.validate(0x1ff0,32,true),Err(UsbError::InvalidAddress));
  assert_eq!(c.validate(0x1000,2048,true),Err(UsbError::NoResources));
 }
 #[test]fn rejects_noncoherent_regions_when_required(){
  assert_eq!(DmaConstraints::xhci_64bit(64).validate(0x1000,128,false),Err(UsbError::Unsupported));
 }
 #[test]fn permits_full_64bit_address_space_without_overflow(){
  assert_eq!(DmaConstraints::xhci_64bit(16).validate(0xffff_ffff_ffff_fff0,16,true),Ok(()));
  assert_eq!(DmaConstraints::xhci_64bit(16).validate(0xffff_ffff_ffff_fff0,17,true),Err(UsbError::InvalidAddress));
 }
}

#[cfg(test)]
mod allocator_contract_tests{
 use super::*;
 struct MockBuffer{bytes:[u8;128],address:u64,coherent:bool}
 unsafe impl UsbDmaBuffer for MockBuffer{
  fn dma_address(&self)->u64{self.address}
  fn len(&self)->usize{self.bytes.len()}
  fn is_coherent(&self)->bool{self.coherent}
  fn as_mut_slice(&mut self)->&mut[u8]{&mut self.bytes}
 }
 struct MockAllocator{address:u64,coherent:bool}
 impl UsbDmaAllocator for MockAllocator{
  type Buffer=MockBuffer;
  fn allocate(&mut self,_:usize,_:DmaConstraints)->Result<Self::Buffer,UsbError>{
   Ok(MockBuffer{bytes:[0;128],address:self.address,coherent:self.coherent})
  }
 }
 #[test]fn validates_allocator_output_before_exposing_dma_buffer(){
  let mut allocator=MockAllocator{address:0x2000,coherent:true};
  let mut buffer=allocate_validated(&mut allocator,64,DmaConstraints::legacy_32bit(64)).unwrap();
  buffer.as_mut_slice()[0]=0x5a;assert_eq!(buffer.as_mut_slice()[0],0x5a);
  let mut invalid=MockAllocator{address:0x1_0000_0000,coherent:true};
  assert!(matches!(allocate_validated(&mut invalid,64,DmaConstraints::legacy_32bit(64)),Err(UsbError::InvalidAddress)));
 }
}
