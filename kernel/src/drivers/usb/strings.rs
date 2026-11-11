//! Allocation-free USB string descriptor parsing and UTF-8 conversion.
use super::UsbError;
pub const STRING_DESCRIPTOR:u8=3;
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct StringDescriptor<'a>{utf16le:&'a[u8]}
impl<'a> StringDescriptor<'a>{
 pub fn parse(bytes:&'a[u8])->Result<Self,UsbError>{
  if bytes.len()<2{return Err(UsbError::BufferTooShort)}
  if bytes[1]!=STRING_DESCRIPTOR||bytes[0]<2||bytes[0]&1!=0{
   return Err(UsbError::MalformedDescriptor)
  }
  if bytes[0] as usize>bytes.len(){return Err(UsbError::BufferTooShort)}
  Ok(Self{utf16le:&bytes[2..bytes[0] as usize]})
 }
 pub const fn code_units(self)->usize{self.utf16le.len()/2}
 /// Converts UTF-16LE to UTF-8 without allocation. Returns bytes written.
 pub fn write_utf8(self,out:&mut[u8])->Result<usize,UsbError>{
  let mut read=0;let mut written=0;
  while read<self.utf16le.len(){
   let first=u16::from_le_bytes([self.utf16le[read],self.utf16le[read+1]]);read+=2;
   let scalar=if (0xd800..=0xdbff).contains(&first){
    if read+1>=self.utf16le.len(){return Err(UsbError::MalformedDescriptor)}
    let second=u16::from_le_bytes([self.utf16le[read],self.utf16le[read+1]]);read+=2;
    if !(0xdc00..=0xdfff).contains(&second){return Err(UsbError::MalformedDescriptor)}
    0x10000+((((first as u32)-0xd800)<<10)|((second as u32)-0xdc00))
   }else if (0xdc00..=0xdfff).contains(&first){return Err(UsbError::MalformedDescriptor)}
   else{first as u32};
   let n=if scalar<=0x7f{1}else if scalar<=0x7ff{2}else if scalar<=0xffff{3}else{4};
   if written+n>out.len(){return Err(UsbError::NoResources)}
   match n{
    1=>out[written]=scalar as u8,
    2=>{out[written]=0xc0|((scalar>>6)as u8);out[written+1]=0x80|(scalar as u8&0x3f);}
    3=>{out[written]=0xe0|((scalar>>12)as u8);out[written+1]=0x80|((scalar>>6)as u8&0x3f);out[written+2]=0x80|(scalar as u8&0x3f);}
    _=>{out[written]=0xf0|((scalar>>18)as u8);out[written+1]=0x80|((scalar>>12)as u8&0x3f);out[written+2]=0x80|((scalar>>6)as u8&0x3f);out[written+3]=0x80|(scalar as u8&0x3f);}
   }
   written+=n;
  }
  Ok(written)
 }
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct LanguageIdList<'a>{bytes:&'a[u8]}
impl<'a> LanguageIdList<'a>{
 pub fn parse(bytes:&'a[u8])->Result<Self,UsbError>{
  if bytes.len()<4||bytes[1]!=STRING_DESCRIPTOR||bytes[0]<4||bytes[0]&1!=0{
   return Err(UsbError::MalformedDescriptor)
  }
  if bytes[0] as usize>bytes.len(){return Err(UsbError::BufferTooShort)}
  Ok(Self{bytes:&bytes[2..bytes[0] as usize]})
 }
 pub fn contains(self,language_id:u16)->bool{
  self.bytes.chunks_exact(2).any(|v|u16::from_le_bytes([v[0],v[1]])==language_id)
 }
 pub fn iter(self)->impl Iterator<Item=u16>+'_{
  self.bytes.chunks_exact(2).map(|v|u16::from_le_bytes([v[0],v[1]]))
 }
}

#[cfg(test)]
mod tests{
 use super::*;
 #[test]fn converts_ascii_and_multibyte_utf16(){
  // "A" followed by U+00E9 and U+1F600.
  let bytes=[10,3,0x41,0,0xe9,0,0x3d,0xd8,0x00,0xde];
  let d=StringDescriptor::parse(&bytes).unwrap();let mut out=[0;8];
  let n=d.write_utf8(&mut out).unwrap();
  assert_eq!(&out[..n],"Aé😀".as_bytes());
 }
 #[test]fn rejects_unpaired_surrogates(){
  let bytes=[4,3,0x00,0xd8];
  assert_eq!(StringDescriptor::parse(&bytes).unwrap().write_utf8(&mut[0;8]),Err(UsbError::MalformedDescriptor));
 }
 #[test]fn parses_supported_language_ids(){
  let ids=LanguageIdList::parse(&[6,3,0x09,0x04,0x11,0x04]).unwrap();
  assert!(ids.contains(0x0409));assert!(ids.contains(0x0411));assert!(!ids.contains(0x0407));
 }
 #[test]fn rejects_small_output_buffer(){
  let d=StringDescriptor::parse(&[4,3,0x41,0]).unwrap();
  assert_eq!(d.write_utf8(&mut[0;0]),Err(UsbError::NoResources));
 }
}
