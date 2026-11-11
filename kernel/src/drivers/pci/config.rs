//! PCI/PCIe configuration primitives, identification and bounded enumeration.
use crate::drivers::api::{BusAddress, DeviceClass, DeviceInfo};
pub const PCI_CONFIG_SIZE:u16=256; pub const PCIE_CONFIG_SIZE:u16=4096; pub const MAX_DEVICES:u8=32; pub const MAX_FUNCTIONS:u8=8; pub const INVALID_VENDOR_ID:u16=0xffff;
#[derive(Clone,Copy,Debug,Eq,PartialEq)] pub struct PciAddress{pub segment:u16,pub bus:u8,pub device:u8,pub function:u8}
impl PciAddress{pub const fn new(segment:u16,bus:u8,device:u8,function:u8)->Option<Self>{if device<32&&function<8{Some(Self{segment,bus,device,function})}else{None}} pub const fn bus_address(self)->BusAddress{BusAddress::Pci{segment:self.segment,bus:self.bus,device:self.device,function:self.function}}}
#[derive(Clone,Copy,Debug,Eq,PartialEq)] pub enum ConfigError{InvalidAddress,UnalignedRegister,OutOfRange,UnsupportedSegment,MappingUnavailable,BufferTooSmall,InvalidBar}
/// Implementations validate alignment, address bounds and mapped hardware windows.
pub trait ConfigAccess{fn read_u32(&self,address:PciAddress,register:u16)->Result<u32,ConfigError>;fn write_u32(&self,address:PciAddress,register:u16,value:u32)->Result<(),ConfigError>;}
#[derive(Clone,Copy,Debug,Eq,PartialEq)] pub struct PciIdentity{pub vendor_id:u16,pub device_id:u16,pub command:u16,pub status:u16,pub revision:u8,pub programming_interface:u8,pub subclass:u8,pub class_code:u8,pub header_type:u8}
impl PciIdentity{pub fn read<A:ConfigAccess>(io:&A,a:PciAddress)->Result<Option<Self>,ConfigError>{let id=io.read_u32(a,0)?;let vendor=id as u16;if vendor==0||vendor==INVALID_VENDOR_ID{return Ok(None)}let cs=io.read_u32(a,4)?;let c=io.read_u32(a,8)?;let h=io.read_u32(a,12)?;Ok(Some(Self{vendor_id:vendor,device_id:(id>>16)as u16,command:cs as u16,status:(cs>>16)as u16,revision:c as u8,programming_interface:(c>>8)as u8,subclass:(c>>16)as u8,class_code:(c>>24)as u8,header_type:(h>>16)as u8}))} pub const fn is_multifunction(self)->bool{self.header_type&0x80!=0} pub const fn bar_register_count(self)->u8{match self.header_type&0x7f{0=>6,1=>2,2=>1,_=>0}} pub const fn device_class(self)->DeviceClass{match(self.class_code,self.subclass){(1,_)=>DeviceClass::Storage,(2,_)=>DeviceClass::Network,(3,_)=>DeviceClass::Display,(12,3)=>DeviceClass::Usb,(7,_)=>DeviceClass::Serial,_=>DeviceClass::Other}} pub const fn device_info(self,a:PciAddress)->DeviceInfo{DeviceInfo{class:self.device_class(),vendor_id:Some(self.vendor_id),device_id:Some(self.device_id),address:a.bus_address()}}}
#[derive(Clone,Copy,Debug,Eq,PartialEq)] pub struct PciFunction{pub address:PciAddress,pub identity:PciIdentity}
/// Summary of a single-bus scan, including devices omitted by a small buffer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BusScanReport {
    pub written: usize,
    pub devices_found: u32,
    pub truncated: bool,
}

/// Scan one bus and report buffer truncation explicitly; no heap allocation is performed.
pub fn scan_bus_report<A: ConfigAccess>(
    io: &A,
    segment: u16,
    bus: u8,
    out: &mut [PciFunction],
) -> Result<BusScanReport, ConfigError> {
    let mut report = BusScanReport::default();
    for device in 0..MAX_DEVICES {
        let first_address = PciAddress { segment, bus, device, function: 0 };
        let Some(first_identity) = PciIdentity::read(io, first_address)? else {
            continue;
        };
        record_bus_function(
            PciFunction { address: first_address, identity: first_identity },
            &mut report,
            out,
        );
        let function_limit = if first_identity.is_multifunction() { MAX_FUNCTIONS } else { 1 };
        for function in 1..function_limit {
            let address = PciAddress { function, ..first_address };
            if let Some(identity) = PciIdentity::read(io, address)? {
                record_bus_function(PciFunction { address, identity }, &mut report, out);
            }
        }
    }
    Ok(report)
}

fn record_bus_function(function: PciFunction, report: &mut BusScanReport, out: &mut [PciFunction]) {
    report.devices_found = report.devices_found.saturating_add(1);
    if report.written < out.len() {
        out[report.written] = function;
        report.written += 1;
    } else {
        report.truncated = true;
    }
}

/// Compatibility scan returning the number stored. Use scan_bus_report when
/// callers must distinguish a complete scan from output-buffer truncation.
pub fn scan_bus<A: ConfigAccess>(
    io: &A,
    segment: u16,
    bus: u8,
    out: &mut [PciFunction],
) -> Result<usize, ConfigError> {
    Ok(scan_bus_report(io, segment, bus, out)?.written)
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)] pub enum BarKind{Io,MemoryBelow1M,Memory32,Memory64}
#[derive(Clone,Copy,Debug,Eq,PartialEq)] pub struct Bar{pub index:u8,pub kind:BarKind,pub prefetchable:bool,pub base:u64,pub raw_low:u32,pub raw_high:Option<u32>}
impl Bar {
    /// Decode a BAR value. Reserved encodings and reserved address bits are rejected.
    pub const fn decode(index: u8, low: u32, high: Option<u32>) -> Option<Self> {
        if low & 1 != 0 {
            // Bit 1 of an I/O BAR is reserved and must read as zero.
            if low & 0x2 != 0 {
                return None;
            }
            return Some(Self {
                index,
                kind: BarKind::Io,
                prefetchable: false,
                base: (low & !3) as u64,
                raw_low: low,
                raw_high: None,
            });
        }

        let memory_type = (low >> 1) & 3;
        match memory_type {
            0 => Some(Self {
                index,
                kind: BarKind::Memory32,
                prefetchable: low & 8 != 0,
                base: (low & !15) as u64,
                raw_low: low,
                raw_high: None,
            }),
            1 => {
                // A below-1-MiB BAR has only address bits 19:4.
                if low & 0xfff0_0000 != 0 {
                    return None;
                }
                Some(Self {
                    index,
                    kind: BarKind::MemoryBelow1M,
                    prefetchable: low & 8 != 0,
                    base: (low & 0x000f_fff0) as u64,
                    raw_low: low,
                    raw_high: None,
                })
            }
            2 => match high {
                Some(upper) => Some(Self {
                    index,
                    kind: BarKind::Memory64,
                    prefetchable: low & 8 != 0,
                    base: ((upper as u64) << 32) | (low as u64 & !15),
                    raw_low: low,
                    raw_high: Some(upper),
                }),
                None => None,
            },
            _ => None,
        }
    }
}
/// PCIe ECAM window descriptor, normally sourced from ACPI MCFG.
#[derive(Clone,Copy,Debug,Eq,PartialEq)] pub struct EcamWindow{pub segment:u16,pub start_bus:u8,pub end_bus:u8,pub physical_base:u64}
impl EcamWindow{pub const fn contains(self,a:PciAddress)->bool{a.segment==self.segment&&a.bus>=self.start_bus&&a.bus<=self.end_bus}pub const fn offset(self,a:PciAddress,register:u16)->Option<u64>{if !self.contains(a)||a.device>=MAX_DEVICES||a.function>=MAX_FUNCTIONS||register>=PCIE_CONFIG_SIZE||register&3!=0{return None}Some((((a.bus-self.start_bus)as u64)<<20)|((a.device as u64)<<15)|((a.function as u64)<<12)|register as u64)}}

/// Read the currently programmed BAR values without probing or modifying them.
/// The output buffer must hold every decoded BAR; a 64-bit BAR consumes two
/// configuration registers but produces one Bar entry.
pub fn read_bars<A: ConfigAccess>(io: &A, function: PciFunction, out: &mut [Bar]) -> Result<usize, ConfigError> {
    let count = function.identity.bar_register_count();
    let mut index = 0u8;
    let mut written = 0usize;
    while index < count {
        let register = 0x10u16 + (index as u16) * 4;
        let low = io.read_u32(function.address, register)?;
        let is_64_bit_memory = low & 1 == 0 && ((low >> 1) & 3) == 2;
        let high = if is_64_bit_memory {
            if index + 1 >= count { return Err(ConfigError::InvalidBar); }
            Some(io.read_u32(function.address, register + 4)?)
        } else { None };
        let bar = Bar::decode(index, low, high).ok_or(ConfigError::InvalidBar)?;
        if written >= out.len() { return Err(ConfigError::BufferTooSmall); }
        out[written] = bar;
        written += 1;
        index += if is_64_bit_memory { 2 } else { 1 };
    }
    Ok(written)
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn bdf_bounds(){assert!(PciAddress::new(0,0,31,7).is_some());assert!(PciAddress::new(0,0,32,0).is_none());}
    #[test] fn bar_decode(){assert_eq!(Bar::decode(0,0x1001,None).unwrap().base,0x1000);assert!(Bar::decode(0,0x1004,None).is_none());assert_eq!(Bar::decode(0,0x100c,Some(1)).unwrap().base,0x1_0000_1000);}
    #[test] fn decodes_below_one_megabyte_bar(){let bar=Bar::decode(0,0x0008_1002,None).unwrap();assert_eq!(bar.kind,BarKind::MemoryBelow1M);assert_eq!(bar.base,0x81000);}
    #[test] fn ecam_offset(){let w=EcamWindow{segment:0,start_bus:32,end_bus:63,physical_base:0xe0000000};let a=PciAddress{segment:0,bus:33,device:2,function:3};assert_eq!(w.offset(a,0x100),Some(0x10_0000+0x1_0000+0x1800+0x100));}
}


#[cfg(test)]
mod scan_report_tests {
    use super::*;

    struct MockConfig {
        functions: [(PciAddress, [u32; 4]); 2],
    }

    impl ConfigAccess for MockConfig {
        fn read_u32(&self, address: PciAddress, register: u16) -> Result<u32, ConfigError> {
            let index = (register / 4) as usize;
            for (function_address, registers) in self.functions {
                if function_address == address {
                    return registers.get(index).copied().ok_or(ConfigError::OutOfRange);
                }
            }
            Ok(if register == 0 { u32::MAX } else { 0 })
        }

        fn write_u32(&self, _address: PciAddress, _register: u16, _value: u32) -> Result<(), ConfigError> {
            Err(ConfigError::OutOfRange)
        }
    }

    fn empty_function() -> PciFunction {
        PciFunction {
            address: PciAddress { segment: 0, bus: 0, device: 0, function: 0 },
            identity: PciIdentity {
                vendor_id: 0, device_id: 0, command: 0, status: 0, revision: 0,
                programming_interface: 0, subclass: 0, class_code: 0, header_type: 0,
            },
        }
    }

    #[test]
    fn reports_devices_omitted_by_small_output_buffer() {
        let first = PciAddress { segment: 0, bus: 0, device: 0, function: 0 };
        let second = PciAddress { segment: 0, bus: 0, device: 1, function: 0 };
        let io = MockConfig {
            functions: [
                (first, [0x5678_1234, 0, 0, 0]),
                (second, [0x9abc_4321, 0, 0, 0]),
            ],
        };
        let mut output = [empty_function()];
        let report = scan_bus_report(&io, 0, 0, &mut output).unwrap();
        assert_eq!(report.written, 1);
        assert_eq!(report.devices_found, 2);
        assert!(report.truncated);
        assert_eq!(output[0].address, first);
    }

    #[test]
    fn reports_complete_scan_when_buffer_fits() {
        let first = PciAddress { segment: 0, bus: 0, device: 0, function: 0 };
        let second = PciAddress { segment: 0, bus: 0, device: 1, function: 0 };
        let io = MockConfig {
            functions: [
                (first, [0x5678_1234, 0, 0, 0]),
                (second, [0x9abc_4321, 0, 0, 0]),
            ],
        };
        let mut output = [empty_function(); 2];
        let report = scan_bus_report(&io, 0, 0, &mut output).unwrap();
        assert_eq!(report.written, 2);
        assert_eq!(report.devices_found, 2);
        assert!(!report.truncated);
    }

    #[test]
    fn rejects_reserved_bar_encodings_and_address_bits() {
        assert!(Bar::decode(0, 0x1003, None).is_none());
        assert!(Bar::decode(0, 0x0010_1002, None).is_none());
        assert!(Bar::decode(0, 0x0008_1002, None).is_some());
    }

}
