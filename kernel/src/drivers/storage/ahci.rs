//! AHCI command/FIS/PRDT protocol primitives.
//!
//! PCI discovery, BAR mapping, DMA allocation, interrupt delivery and port
//! lifecycle are supplied by the corresponding kernel subsystems.
use super::block::{BlockError, BlockGeometry};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AhciError { InvalidCapabilities, InvalidPort, InvalidTransfer, AddressOverflow, InvalidAlignment, PrdtCapacity, UnsupportedAddressing }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AhciCapabilities {
    pub port_count:u8, pub command_slots:u8, pub supports_64bit_dma:bool,
    pub supports_ncq:bool, pub supports_staggered_spinup:bool,
    pub supports_aggressive_link_power:bool, pub supports_clo:bool,
}
impl AhciCapabilities {
    pub fn parse(cap:u32)->Result<Self,AhciError>{
        let ports=((cap&0x1F)+1) as u8;
        let slots=(((cap>>8)&0x1F)+1) as u8;
        if ports==0 || ports>32 || slots==0 || slots>32{return Err(AhciError::InvalidCapabilities);}
        Ok(Self{port_count:ports,command_slots:slots,supports_64bit_dma:cap&(1<<31)!=0,
            supports_ncq:cap&(1<<30)!=0,supports_staggered_spinup:cap&(1<<27)!=0,
            supports_aggressive_link_power:cap&(1<<26)!=0,supports_clo:cap&(1<<24)!=0})
    }
}

#[repr(C)]
#[derive(Clone,Copy,Debug,Default,Eq,PartialEq)]
pub struct AhciCommandHeader {
    pub flags:u16, pub prdt_length:u16, pub transferred_bytes:u32,
    pub command_table_base:u32, pub command_table_base_upper:u32,
    pub reserved:[u32;4],
}
const _:[();32]=[();core::mem::size_of::<AhciCommandHeader>()];

#[repr(C)]
#[derive(Clone,Copy,Debug,Default,Eq,PartialEq)]
pub struct AhciPrdtEntry {
    pub data_base:u32, pub data_base_upper:u32, pub reserved:u32, pub byte_count_and_interrupt:u32,
}
const _:[();16]=[();core::mem::size_of::<AhciPrdtEntry>()];
impl AhciPrdtEntry {
    pub const fn byte_count(&self)->u32{(self.byte_count_and_interrupt&0x003F_FFFF)+1}
    pub const fn interrupt_on_completion(&self)->bool{self.byte_count_and_interrupt&(1<<31)!=0}
}

#[repr(C)]
#[derive(Clone,Copy,Debug,Default,Eq,PartialEq)]
pub struct RegisterFisH2d {
    pub fis_type:u8, pub flags:u8, pub command:u8, pub feature_low:u8,
    pub lba0:u8, pub lba1:u8, pub lba2:u8, pub device:u8,
    pub lba3:u8, pub lba4:u8, pub lba5:u8, pub feature_high:u8,
    pub count_low:u8, pub count_high:u8, pub icc:u8, pub control:u8,
    pub reserved:[u8;4],
}
const _:[();20]=[();core::mem::size_of::<RegisterFisH2d>()];
impl RegisterFisH2d {
    pub fn read_dma_ext(lba:u64,sectors:u32)->Result<Self,AhciError>{Self::lba48(0x25,lba,sectors)}
    pub fn write_dma_ext(lba:u64,sectors:u32)->Result<Self,AhciError>{Self::lba48(0x35,lba,sectors)}
    fn lba48(command:u8,lba:u64,sectors:u32)->Result<Self,AhciError>{
        if sectors==0 || sectors>65_536 || lba >= (1u64<<48) || lba.checked_add(sectors as u64).filter(|end|*end <= (1u64<<48)).is_none(){
            return Err(AhciError::InvalidTransfer);
        }
        Ok(Self{fis_type:0x27,flags:0x80,command,feature_low:0,
            lba0:lba as u8,lba1:(lba>>8) as u8,lba2:(lba>>16) as u8,device:1<<6,
            lba3:(lba>>24) as u8,lba4:(lba>>32) as u8,lba5:(lba>>40) as u8,
            feature_high:0,count_low:sectors as u8,count_high:(sectors>>8) as u8,
            icc:0,control:0,reserved:[0;4]})
    }
    pub fn read_fpdma_queued(lba:u64,sectors:u32,tag:u8)->Result<Self,AhciError>{
        Self::ncq(0x60,lba,sectors,tag)
    }
    pub fn write_fpdma_queued(lba:u64,sectors:u32,tag:u8)->Result<Self,AhciError>{
        Self::ncq(0x61,lba,sectors,tag)
    }
    fn ncq(command:u8,lba:u64,sectors:u32,tag:u8)->Result<Self,AhciError>{
        if sectors==0 || sectors>65_536 || tag>31 || lba >= (1u64<<48)
            || lba.checked_add(sectors as u64).filter(|end|*end <= (1u64<<48)).is_none(){
            return Err(AhciError::InvalidTransfer);
        }
        Ok(Self{fis_type:0x27,flags:0x80,command,feature_low:sectors as u8,
            lba0:lba as u8,lba1:(lba>>8) as u8,lba2:(lba>>16) as u8,device:1<<6,
            lba3:(lba>>24) as u8,lba4:(lba>>32) as u8,lba5:(lba>>40) as u8,
            feature_high:(sectors>>8) as u8,count_low:tag<<3,count_high:0,
            icc:0,control:0,reserved:[0;4]})
    }
}

/// Build a PRDT for one physically contiguous DMA region. AHCI limits each
/// descriptor to 4 MiB; DBC stores byte_count - 1.
pub fn build_prdt(physical:u64,byte_len:usize,entries:&mut[AhciPrdtEntry],supports_64bit:bool)->Result<usize,AhciError>{
    if byte_len==0 || physical&1!=0{return Err(AhciError::InvalidTransfer);}
    let end=physical.checked_add(byte_len as u64).ok_or(AhciError::AddressOverflow)?;
    if !supports_64bit && end>u32::MAX as u64+1{return Err(AhciError::UnsupportedAddressing);}
    let count=byte_len.checked_add(4*1024*1024-1).ok_or(AhciError::AddressOverflow)?/(4*1024*1024);
    if count>entries.len() || count>u16::MAX as usize{return Err(AhciError::PrdtCapacity);}
    let mut address=physical; let mut remaining=byte_len; let mut index=0usize;
    while remaining!=0{
        let bytes=core::cmp::min(remaining,4*1024*1024);
        entries[index]=AhciPrdtEntry{data_base:address as u32,data_base_upper:(address>>32) as u32,
            reserved:0,byte_count_and_interrupt:(bytes as u32-1)|if index+1==count{1<<31}else{0}};
        address=address.checked_add(bytes as u64).ok_or(AhciError::AddressOverflow)?;
        remaining-=bytes; index+=1;
    }
    Ok(count)
}

pub fn command_header(table_phys:u64,prdt_count:u16,write:bool,command_fis_dwords:u8)->Result<AhciCommandHeader,AhciError>{
    if table_phys&0x7F!=0 || command_fis_dwords<5 || command_fis_dwords>16{return Err(AhciError::InvalidAlignment);}
    Ok(AhciCommandHeader{flags:command_fis_dwords as u16|if write{1<<6}else{0},
        prdt_length:prdt_count,transferred_bytes:0,command_table_base:table_phys as u32,
        command_table_base_upper:(table_phys>>32) as u32,reserved:[0;4]})
}

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct AhciIdentify {
    pub sector_count:u64,pub logical_sector_size:u32,pub supports_lba48:bool,pub supports_ncq:bool,
}
impl AhciIdentify {
    /// Parse the 512-byte ATA IDENTIFY DEVICE payload.
    pub fn parse(words:&[u16])->Result<Self,AhciError>{
        if words.len()<256{return Err(AhciError::InvalidCapabilities);}
        let word83_valid=words[83]&(1<<14)!=0 && words[83]&(1<<15)==0;
        let supports_lba48=word83_valid && words[83]&(1<<10)!=0;
        if !supports_lba48{return Err(AhciError::UnsupportedAddressing);}
        let sector_count=(words[100] as u64)|((words[101] as u64)<<16)|((words[102] as u64)<<32)|((words[103] as u64)<<48);
        if sector_count==0{return Err(AhciError::InvalidCapabilities);}
        let logical_sector_size=if words[106]&(1<<12)!=0 {
            if words[106]&(1<<14)==0 || words[106]&(1<<15)!=0 { return Err(AhciError::InvalidCapabilities); }
            let words_per_sector=(words[117] as u32)|((words[118] as u32)<<16);
            words_per_sector.checked_mul(2).ok_or(AhciError::InvalidCapabilities)?
        }else{512};
        if !(512..=65_536).contains(&logical_sector_size) || !logical_sector_size.is_power_of_two(){return Err(AhciError::InvalidCapabilities);}
        Ok(Self{sector_count,logical_sector_size,supports_lba48,supports_ncq:words[76]&(1<<8)!=0})
    }
    pub fn geometry(self)->Result<BlockGeometry,BlockError>{
        BlockGeometry{block_count:self.sector_count,block_size:self.logical_sector_size}.validate()
    }
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]fn capabilities_decode_ports_and_slots(){
        let cap=3|(7<<8)|(1<<31)|(1<<30);
        let c=AhciCapabilities::parse(cap).unwrap();
        assert_eq!(c.port_count,4);assert_eq!(c.command_slots,8);assert!(c.supports_64bit_dma);assert!(c.supports_ncq);
    }
    #[test]fn fis_encodes_48bit_lba_and_count(){
        let fis=RegisterFisH2d::read_dma_ext(0x1234_5678_9ABC,0x1234).unwrap();
        assert_eq!(fis.command,0x25);assert_eq!(fis.lba0,0xBC);assert_eq!(fis.lba5,0x12);
        assert_eq!(fis.count_low,0x34);assert_eq!(fis.count_high,0x12);
    }
    #[test]fn prdt_splits_at_four_megabytes(){
        let mut entries=[AhciPrdtEntry::default();2];
        let n=build_prdt(0x1000,4*1024*1024+512,&mut entries,true).unwrap();
        assert_eq!(n,2);assert_eq!(entries[0].byte_count(),4*1024*1024);
        assert_eq!(entries[1].byte_count(),512);assert!(entries[1].interrupt_on_completion());
    }
    #[test]fn ncq_fpdma_encodes_tag_and_sector_count(){
        let fis=RegisterFisH2d::read_fpdma_queued(0x1234,0x2345,7).unwrap();
        assert_eq!(fis.command,0x60);
        assert_eq!(fis.count_low,7<<3);
        assert_eq!(fis.feature_low,0x45);
        assert_eq!(fis.feature_high,0x23);
    }
    #[test]fn identify_decodes_lba48_and_large_logical_sector(){
        let mut words=[0u16;256];
        words[83]=(1<<14)|(1<<10); words[76]=1<<8;
        words[100]=0x5678; words[101]=0x1234; words[102]=1;
        words[106]=(1<<14)|(1<<12); words[117]=2048;
        let id=AhciIdentify::parse(&words).unwrap();
        assert_eq!(id.sector_count,0x1_1234_5678);
        assert_eq!(id.logical_sector_size,4096);
        assert!(id.supports_ncq);
    }
    #[test]fn rejects_dma_above_4g_without_support(){
        let mut entries=[AhciPrdtEntry::default();1];
        assert_eq!(build_prdt(0x1_0000_0000,512,&mut entries,false),Err(AhciError::UnsupportedAddressing));
    }
}
