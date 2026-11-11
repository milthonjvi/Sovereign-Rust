//! Standard and PCI Express extended capability list traversal.
use super::config::{ConfigAccess,ConfigError,PciAddress};

#[derive(Clone,Copy,Debug,Eq,PartialEq)] pub struct Capability{pub id:u8,pub offset:u8}
/// Iterator over the conventional 0x40..=0xfc capability list.
pub struct StandardCapabilities<'a,A:ConfigAccess>{io:&'a A,address:PciAddress,next:u8,visited:[u64;4],remaining:u8}
impl<'a,A:ConfigAccess> StandardCapabilities<'a,A>{
 pub fn new(io:&'a A,address:PciAddress,status:u16)->Result<Self,ConfigError>{let pointer=if status&(1<<4)==0{0}else{(io.read_u32(address,0x34)?&0xff)as u8};if pointer!=0&&(pointer<0x40||pointer>0xfc||pointer&3!=0){return Err(ConfigError::OutOfRange)}Ok(Self{io,address,next:pointer,visited:[0;4],remaining:48})}
}
impl<A:ConfigAccess> Iterator for StandardCapabilities<'_,A>{type Item=Result<Capability,ConfigError>;fn next(&mut self)->Option<Self::Item>{let offset=self.next;if offset==0||self.remaining==0{return None}if offset<0x40||offset>0xfc||offset&3!=0{self.next=0;return Some(Err(ConfigError::OutOfRange))}let slot=(offset/4)as usize;let bit=1u64<<(slot%64);if self.visited[slot/64]&bit!=0{self.next=0;return Some(Err(ConfigError::OutOfRange))}self.visited[slot/64]|=bit;self.remaining-=1;match self.io.read_u32(self.address,offset as u16){Ok(word)=>{let id=word as u8;let next=(word>>8)as u8;self.next=next;Some(Ok(Capability{id,offset}))},Err(e)=>{self.next=0;Some(Err(e))}}}}

#[derive(Clone,Copy,Debug,Eq,PartialEq)]pub struct ExtendedCapability{pub id:u16,pub version:u8,pub offset:u16,pub next:u16}
/// Iterator over PCIe extended capabilities (0x100..=0xffc).
pub struct ExtendedCapabilities<'a,A:ConfigAccess>{io:&'a A,address:PciAddress,next:u16,visited:[u64;16],remaining:u16}
impl<'a,A:ConfigAccess> ExtendedCapabilities<'a,A>{pub const fn new(io:&'a A,address:PciAddress)->Self{Self{io,address,next:0x100,visited:[0;16],remaining:960}}}
impl<A:ConfigAccess> Iterator for ExtendedCapabilities<'_,A>{type Item=Result<ExtendedCapability,ConfigError>;fn next(&mut self)->Option<Self::Item>{let offset=self.next;if offset==0||self.remaining==0{return None}if offset<0x100||offset>0xffc||offset&3!=0{self.next=0;return Some(Err(ConfigError::OutOfRange))}let slot=(offset/4)as usize;let bit=1u64<<(slot%64);if self.visited[slot/64]&bit!=0{self.next=0;return Some(Err(ConfigError::OutOfRange))}self.visited[slot/64]|=bit;self.remaining-=1;match self.io.read_u32(self.address,offset){Ok(0)=>{self.next=0;None},Ok(header)=>{let id=header as u16;if id==0xffff{self.next=0;return None}let version=((header>>16)&0xf)as u8;let next=((header>>20)&0xfff)as u16;if next!=0&&(next<0x100||next>0xffc||next&3!=0){self.next=next;}else{self.next=next;}Some(Ok(ExtendedCapability{id,version,offset,next}))},Err(e)=>{self.next=0;Some(Err(e))}}}}


#[cfg(test)]
mod tests {
    use super::*;

    struct MockConfig([u32; 1024]);
    impl ConfigAccess for MockConfig {
        fn read_u32(&self, _address: PciAddress, register: u16) -> Result<u32, ConfigError> {
            self.0.get((register / 4) as usize).copied().ok_or(ConfigError::OutOfRange)
        }
        fn write_u32(&self, _address: PciAddress, _register: u16, _value: u32) -> Result<(), ConfigError> {
            Err(ConfigError::OutOfRange)
        }
    }

    fn address() -> PciAddress {
        PciAddress { segment: 0, bus: 0, device: 0, function: 0 }
    }

    #[test]
    fn rejects_malformed_standard_capability_pointer() {
        let mut registers = [0u32; 1024];
        registers[0x34 / 4] = 0x20;
        let io = MockConfig(registers);
        assert!(matches!(StandardCapabilities::new(&io, address(), 1 << 4), Err(ConfigError::OutOfRange)));
    }

    #[test]
    fn reports_invalid_standard_next_pointer() {
        let mut registers = [0u32; 1024];
        registers[0x40 / 4] = 0x0000_2001;
        let io = MockConfig(registers);
        let mut caps = StandardCapabilities::new(&io, address(), 1 << 4).unwrap();
        assert_eq!(caps.next().unwrap().unwrap().id, 1);
        assert_eq!(caps.next(), Some(Err(ConfigError::OutOfRange)));
        assert!(caps.next().is_none());
    }

    #[test]
    fn reports_invalid_extended_next_pointer() {
        let mut registers = [0u32; 1024];
        registers[0x100 / 4] = 1 | (1 << 16) | (0x80 << 20);
        let io = MockConfig(registers);
        let mut caps = ExtendedCapabilities::new(&io, address());
        assert_eq!(caps.next().unwrap().unwrap().id, 1);
        assert_eq!(caps.next(), Some(Err(ConfigError::OutOfRange)));
        assert!(caps.next().is_none());
    }
}
