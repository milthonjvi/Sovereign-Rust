//! AHCI command/FIS/PRDT protocol primitives.
//!
//! PCI discovery, BAR mapping, DMA allocation, interrupt delivery and port
//! lifecycle are supplied by the corresponding kernel subsystems.
use super::block::{BlockError, BlockGeometry};

/// PxIS bits that indicate a failed or unrecoverable command path.
pub const AHCI_PORT_ERROR_INTERRUPT_MASK: u32 = (1 << 30) | (1 << 29) | (1 << 28) | (1 << 27) | (1 << 26) | (1 << 24);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AhciError { InvalidCapabilities, InvalidPort, InvalidTransfer, AddressOverflow, InvalidAlignment, PrdtCapacity, UnsupportedAddressing, QueueFull, Timeout, PortNotRunning, SlotBusy, DeviceFault, InvalidPortMemory, RegisterIo }

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
    if byte_len==0 || physical&0x3!=0{return Err(AhciError::InvalidTransfer);}
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

pub fn command_header(table_phys:u64,prdt_count:u16,write:bool,command_fis_dwords:u8,supports_64bit_dma:bool)->Result<AhciCommandHeader,AhciError>{
    if table_phys&0x7F!=0 || command_fis_dwords<5 || command_fis_dwords>16{return Err(AhciError::InvalidAlignment);}
    if !supports_64bit_dma && table_phys>u32::MAX as u64{return Err(AhciError::UnsupportedAddressing);}
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
    #[test]fn rejects_non_dword_aligned_prdt_buffer(){
        let mut entries=[AhciPrdtEntry::default();1];
        assert_eq!(build_prdt(0x1002,512,&mut entries,true),Err(AhciError::InvalidTransfer));
        assert_eq!(build_prdt(0x1001,512,&mut entries,true),Err(AhciError::InvalidTransfer));
    }
    #[test]fn rejects_prdt_address_range_overflow(){
        let mut entries=[AhciPrdtEntry::default();1];
        assert_eq!(build_prdt(u64::MAX-3,8,&mut entries,true),Err(AhciError::AddressOverflow));
    }
    #[test]fn rejects_dma_above_4g_without_support(){
        let mut entries=[AhciPrdtEntry::default();1];
        assert_eq!(build_prdt(0x1_0000_0000,512,&mut entries,false),Err(AhciError::UnsupportedAddressing));
    }
}


/// Fixed-capacity AHCI command-slot allocator. N must not exceed the
/// controller's 32 command slots; slot indices can also be used as NCQ tags.
pub struct AhciCommandSlots<const N:usize>{used:[bool;N],in_flight:usize}
impl<const N:usize> AhciCommandSlots<N>{
    pub const fn new()->Self{Self{used:[false;N],in_flight:0}}
    pub const fn in_flight(&self)->usize{self.in_flight}
    pub fn allocate(&mut self)->Result<u8,AhciError>{
        if N==0 || N>32{return Err(AhciError::InvalidCapabilities);}
        if let Some(index)=self.used.iter().position(|used|!*used){
            self.used[index]=true;self.in_flight+=1;Ok(index as u8)
        }else{Err(AhciError::QueueFull)}
    }
    pub fn release(&mut self,slot:u8)->bool{
        let index=slot as usize;
        if index<N && self.used[index]{self.used[index]=false;self.in_flight-=1;true}else{false}
    }
    pub fn is_used(&self,slot:u8)->bool{self.used.get(slot as usize).copied().unwrap_or(false)}
}
impl<const N:usize> Default for AhciCommandSlots<N>{fn default()->Self{Self::new()}}

#[cfg(test)]
mod slot_tests{
    use super::*;
    #[test]fn allocates_and_releases_slots_without_duplicates(){
        let mut slots=AhciCommandSlots::<2>::new();
        let a=slots.allocate().unwrap();let b=slots.allocate().unwrap();
        assert_ne!(a,b);assert_eq!(slots.allocate(),Err(AhciError::QueueFull));
        assert!(slots.release(a));assert!(!slots.release(a));
        assert_eq!(slots.in_flight(),1);assert!(slots.is_used(b));
    }
}


pub trait AhciRegisterIo {
    fn read32(&mut self, offset:u32)->Result<u32,AhciError>;
    fn write32(&mut self, offset:u32, value:u32)->Result<(),AhciError>;
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct AhciPortMemory { pub command_list_phys:u64, pub received_fis_phys:u64 }
fn port_register(base:u32,register:u32)->Result<u32,AhciError>{
    base.checked_add(register).ok_or(AhciError::AddressOverflow)
}
fn wait_register<R:AhciRegisterIo>(io:&mut R,offset:u32,mask:u32,clear:bool,polls:u32)->Result<(),AhciError>{
    for _ in 0..polls{
        let value=io.read32(offset)?;
        if (value&mask==0)==clear{return Ok(());}
        core::hint::spin_loop();
    }
    Err(AhciError::Timeout)
}

/// Stop command processing and FIS reception in the AHCI-defined order.
pub fn stop_port<R:AhciRegisterIo>(io:&mut R,base:u32,polls:u32)->Result<(),AhciError>{
    const CMD:u32=0x18; const ST:u32=1; const FRE:u32=1<<4; const FR:u32=1<<14; const CR:u32=1<<15;
    let reg=port_register(base,CMD)?;
    let command=io.read32(reg)?;
    io.write32(reg,command&!ST)?;
    wait_register(io,reg,CR,true,polls)?;
    let command=io.read32(reg)?;
    io.write32(reg,command&!FRE)?;
    wait_register(io,reg,FR,true,polls)
}

/// Program DMA bases while the port is stopped. The supplied regions must be
/// DMA-mapped, physically contiguous, command-list aligned to 1 KiB and FIS
/// receive memory aligned to 256 bytes.
pub fn configure_port<R:AhciRegisterIo>(
    io:&mut R,base:u32,memory:AhciPortMemory,interrupt_enable:u32,supports_64bit_dma:bool,polls:u32,
)->Result<(),AhciError>{
    if memory.command_list_phys&0x3FF!=0 || memory.received_fis_phys&0xFF!=0
        || memory.command_list_phys.checked_add(1024).is_none()
        || memory.received_fis_phys.checked_add(256).is_none()
        || (!supports_64bit_dma && (memory.command_list_phys.checked_add(1024).map_or(true,|v|v>u32::MAX as u64+1)
            || memory.received_fis_phys.checked_add(256).map_or(true,|v|v>u32::MAX as u64+1))){
        return Err(AhciError::InvalidPortMemory);
    }
    stop_port(io,base,polls)?;
    io.write32(port_register(base,0x00)?,memory.command_list_phys as u32)?;
    io.write32(port_register(base,0x04)?,(memory.command_list_phys>>32) as u32)?;
    io.write32(port_register(base,0x08)?,memory.received_fis_phys as u32)?;
    io.write32(port_register(base,0x0C)?,(memory.received_fis_phys>>32) as u32)?;
    io.write32(port_register(base,0x30)?,u32::MAX)?;
    io.write32(port_register(base,0x10)?,u32::MAX)?;
    io.write32(port_register(base,0x14)?,interrupt_enable)?;
    Ok(())
}

pub fn start_port<R:AhciRegisterIo>(io:&mut R,base:u32,polls:u32)->Result<(),AhciError>{
    start_port_with_spinup(io,base,false,polls)
}
pub fn start_port_with_spinup<R:AhciRegisterIo>(io:&mut R,base:u32,staggered_spinup:bool,polls:u32)->Result<(),AhciError>{
    const CMD:u32=0x18; const ST:u32=1; const SUD:u32=1<<1; const FRE:u32=1<<4; const FR:u32=1<<14; const CR:u32=1<<15;
    let reg=port_register(base,CMD)?;
    wait_register(io,reg,CR|FR,true,polls)?;
    let command=io.read32(reg)?;
    let command=(command|FRE)&!SUD;
    io.write32(reg,command|if staggered_spinup{SUD}else{0})?;
    wait_register(io,reg,FR,false,polls)?;
    let command=io.read32(reg)?;
    io.write32(reg,command|ST)?;
    wait_register(io,reg,CR,false,polls)
}

/// Submit a prepared command-table slot. For NCQ, SACT is set before CI.
///
/// The caller must serialize concurrent submissions to the same port. The
/// read-modify-write sequences for PxSACT and PxCI are not atomic across CPUs.
pub fn issue_command<R:AhciRegisterIo>(
    io:&mut R,base:u32,slot:u8,ncq:bool,polls:u32,
)->Result<(),AhciError>{
    if slot>=32{return Err(AhciError::InvalidPort);}
    let cmd=port_register(base,0x18)?;
    const ST:u32=1;
    const CR:u32=1<<15;
    if io.read32(cmd)?&(ST|CR)!=(ST|CR){return Err(AhciError::PortNotRunning);}
    // Do not submit new work while an earlier fatal port condition remains latched.
    let interrupt_status=io.read32(port_register(base,0x10)?)?;
    if interrupt_status&AHCI_PORT_ERROR_INTERRUPT_MASK!=0{return Err(AhciError::DeviceFault);}
    let tfd=port_register(base,0x20)?;
    let mut ready=false;
    for _ in 0..polls{
        let status=io.read32(tfd)?;
        if status&(1|(1<<5))!=0{return Err(AhciError::DeviceFault);}
        if status&(0x80|0x08)==0{ready=true;break;}
        core::hint::spin_loop();
    }
    if !ready{return Err(AhciError::Timeout);}
    let bit=1u32<<slot;
    let ci=port_register(base,0x38)?;
    let issued=io.read32(ci)?;
    if issued&bit!=0{return Err(AhciError::SlotBusy);}
    let sact=port_register(base,0x34)?;
    let active=io.read32(sact)?;
    if ncq {
        if active&bit!=0{return Err(AhciError::SlotBusy);}
        io.write32(sact,active|bit)?;
    } else if active!=0 {
        // A non-NCQ command must not overlap any outstanding NCQ command.
        return Err(AhciError::SlotBusy);
    }
    io.write32(ci,issued|bit)
}

/// Return true only after hardware has cleared CI (and SACT for NCQ).
pub fn command_complete<R:AhciRegisterIo>(
    io:&mut R,base:u32,slot:u8,ncq:bool,
)->Result<bool,AhciError>{
    if slot>=32{return Err(AhciError::InvalidPort);}
    let status=io.read32(port_register(base,0x10)?)?;
    let task_file=io.read32(port_register(base,0x20)?)?;
    if status&AHCI_PORT_ERROR_INTERRUPT_MASK!=0 || task_file&(1|(1<<5))!=0{return Err(AhciError::DeviceFault);}
    let bit=1u32<<slot;
    if io.read32(port_register(base,0x38)?)?&bit!=0{return Ok(false);}
    if ncq && io.read32(port_register(base,0x34)?)?&bit!=0{return Ok(false);}
    Ok(true)
}


#[cfg(test)]
mod port_tests {
    use super::*;
    struct Registers([u32;64]);
    impl AhciRegisterIo for Registers {
        fn read32(&mut self, offset:u32)->Result<u32,AhciError>{Ok(self.0[(offset/4) as usize])}
        fn write32(&mut self, offset:u32,value:u32)->Result<(),AhciError>{
            let slot=&mut self.0[(offset/4) as usize];
            if offset==0x10 || offset==0x30 { *slot &= !value; }
            else if offset==0x18 {
                let mut next=value;
                if value&(1<<4)!=0 { next|=1<<14; } else { next&=!(1<<14); }
                if value&1!=0 { next|=1<<15; } else { next&=!(1<<15); }
                *slot=next;
            } else { *slot=value; }
            Ok(())
        }
    }
    #[test]
    fn configures_starts_and_issues_port_command() {
        let mut regs=Registers([0;64]);
        regs.0[0x18/4]=(1<<0)|(1<<4);
        configure_port(&mut regs,0,AhciPortMemory{command_list_phys:0x4000,received_fis_phys:0x5000},0xFFFF,true,8).unwrap();
        assert_eq!(regs.0[0],0x4000);
        assert_eq!(regs.0[2],0x5000);
        start_port(&mut regs,0,8).unwrap();
        assert_eq!(regs.0[0x18/4]&0x11,0x11);
        issue_command(&mut regs,0,3,true,8).unwrap();
        assert_eq!(regs.0[0x34/4],1<<3);
        assert_eq!(regs.0[0x38/4],1<<3);
        assert_eq!(command_complete(&mut regs,0,3,true),Ok(false));
        regs.0[0x34/4]=0;regs.0[0x38/4]=0;
        assert_eq!(command_complete(&mut regs,0,3,true),Ok(true));
        regs.0[0x20/4]=1<<5;
        assert_eq!(command_complete(&mut regs,0,3,true),Err(AhciError::DeviceFault));
        regs.0[0x20/4]=0;
        regs.0[0x10/4]=1<<27;
        assert_eq!(command_complete(&mut regs,0,3,true),Err(AhciError::DeviceFault));
    }
    #[test]
    fn refuses_submission_while_fatal_port_status_is_latched() {
        let mut regs=Registers([0;64]);
        regs.0[0x18/4]=(1<<0)|(1<<15);
        regs.0[0x10/4]=1<<27; // Interface fatal error.
        assert_eq!(issue_command(&mut regs,0,0,false,1),Err(AhciError::DeviceFault));
        assert_eq!(regs.0[0x38/4],0);
        assert_eq!(regs.0[0x10/4],1<<27); // Status remains available for recovery.
    }

    #[test]
    fn non_ncq_submission_waits_for_all_ncq_commands_to_finish() {
        let mut regs=Registers([0;64]);
        regs.0[0x18/4]=(1<<0)|(1<<15);
        regs.0[0x34/4]=1<<2;
        assert_eq!(issue_command(&mut regs,0,1,false,1),Err(AhciError::SlotBusy));
        assert_eq!(regs.0[0x38/4],0);
    }

    #[test]
    fn non_staggered_start_clears_stale_spinup_request() {
        let mut regs=Registers([0;64]);
        regs.0[0x18/4]=1<<1; // A stale SUD bit must not survive explicit non-staggered start.
        start_port_with_spinup(&mut regs,0,false,4).unwrap();
        assert_eq!(regs.0[0x18/4]&(1<<1),0);
        assert_ne!(regs.0[0x18/4]&1,0);
    }

    #[test]
    fn refuses_submission_until_command_engine_is_running() {
        let mut regs=Registers([0;64]);
        regs.0[0x18/4]=1; // ST set, CR not yet asserted by hardware.
        assert_eq!(issue_command(&mut regs,0,0,false,1),Err(AhciError::PortNotRunning));
        assert_eq!(regs.0[0x38/4],0);
    }

    #[test]
    fn rejects_unaligned_port_dma_memory() {
        let mut regs=Registers([0;64]);
        assert_eq!(configure_port(&mut regs,0,AhciPortMemory{command_list_phys:0x4100,received_fis_phys:0x5000},0,true,1),Err(AhciError::InvalidPortMemory));
    }
}


/// DMA command-table layout. The first 128 bytes contain CFIS, ATAPI command
/// and reserved fields; PRDT entries follow at offset 128.
#[repr(C,align(128))]
pub struct AhciCommandTable<const N:usize>{
    pub command_fis:[u8;64],
    pub atapi_command:[u8;16],
    pub reserved:[u8;48],
    pub prdt:[AhciPrdtEntry;N],
}
const _: [();128]=[();core::mem::size_of::<AhciCommandTable<0>>()];
impl<const N:usize> AhciCommandTable<N>{
    pub const fn new()->Self{
        Self{command_fis:[0;64],atapi_command:[0;16],reserved:[0;48],
            prdt:[AhciPrdtEntry{data_base:0,data_base_upper:0,reserved:0,byte_count_and_interrupt:0};N]}
    }
    pub fn set_register_fis(&mut self,fis:RegisterFisH2d){
        let bytes=[fis.fis_type,fis.flags,fis.command,fis.feature_low,
            fis.lba0,fis.lba1,fis.lba2,fis.device,fis.lba3,fis.lba4,fis.lba5,
            fis.feature_high,fis.count_low,fis.count_high,fis.icc,fis.control,
            fis.reserved[0],fis.reserved[1],fis.reserved[2],fis.reserved[3]];
        self.command_fis[..20].copy_from_slice(&bytes);
        self.command_fis[20..].fill(0);
    }
    pub fn set_prdt(&mut self,entries:&[AhciPrdtEntry])->Result<u16,AhciError>{
        if entries.len()>N || entries.len()>u16::MAX as usize{return Err(AhciError::PrdtCapacity);}
        self.prdt.fill(AhciPrdtEntry::default());
        self.prdt[..entries.len()].copy_from_slice(entries);
        Ok(entries.len() as u16)
    }
}
impl<const N:usize> Default for AhciCommandTable<N>{fn default()->Self{Self::new()}}

#[cfg(test)]
mod table_tests{
    use super::*;
    #[test]
    fn command_table_has_protocol_offset_and_copies_fis(){
        let mut table=AhciCommandTable::<4>::new();
        let fis=RegisterFisH2d::read_dma_ext(0x1234,8).unwrap();
        table.set_register_fis(fis);
        assert_eq!(table.command_fis[0],0x27);
        assert_eq!(table.command_fis[2],0x25);
        assert_eq!(table.command_fis[19],0);
        assert_eq!(core::mem::size_of_val(&table),256);
    }
}


#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub enum AhciDeviceType{Ata,Atapi,PortMultiplier,EnclosureManagement,Unknown(u32)}
impl AhciDeviceType{
    pub const fn from_signature(signature:u32)->Self{
        match signature{
            0x0000_0101=>Self::Ata,
            0xEB14_0101=>Self::Atapi,
            0x9669_0101=>Self::PortMultiplier,
            0xC33C_0101=>Self::EnclosureManagement,
            other=>Self::Unknown(other),
        }
    }
}
/// Return None unless PxSSTS.DET reports an established device link.
pub fn detect_port_device<R:AhciRegisterIo>(io:&mut R,base:u32)->Result<Option<AhciDeviceType>,AhciError>{
    let status=io.read32(port_register(base,0x28)?)?;
    if status&0x0F!=3{return Ok(None);}
    let signature=io.read32(port_register(base,0x24)?)?;
    Ok(Some(AhciDeviceType::from_signature(signature)))
}

#[cfg(test)]
mod detection_tests{
    use super::*;
    #[test]fn classifies_ata_and_atapi_signatures(){
        assert_eq!(AhciDeviceType::from_signature(0x0000_0101),AhciDeviceType::Ata);
        assert_eq!(AhciDeviceType::from_signature(0xEB14_0101),AhciDeviceType::Atapi);
        assert_eq!(AhciDeviceType::from_signature(0x1234_5678),AhciDeviceType::Unknown(0x1234_5678));
    }
}


/// Snapshot of the AHCI port registers relevant to command completion and
/// recovery. Values are captured in one ordered software pass; hardware may
/// still change them between individual MMIO reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AhciPortSnapshot {
    pub interrupt_status: u32,
    pub task_file_data: u32,
    pub sata_error: u32,
    pub command_issue: u32,
    pub sata_active: u32,
}
impl AhciPortSnapshot {
    pub const fn task_file_error(self) -> bool { self.task_file_data & (1 << 0) != 0 }
    pub const fn task_file_busy(self) -> bool { self.task_file_data & (1 << 7) != 0 }
    pub const fn task_file_data_request(self) -> bool { self.task_file_data & (1 << 3) != 0 }
    pub const fn task_file_device_fault(self) -> bool { self.task_file_data & (1 << 5) != 0 }
    /// True only when ATA task-file status permits a new command.
    /// Device Fault (DF) is a fault state, not an idle/ready state.
    pub const fn task_file_ready(self) -> bool {
        !self.task_file_error() && !self.task_file_device_fault()
            && !self.task_file_busy() && !self.task_file_data_request()
    }
    pub const fn interface_fatal(self) -> bool { self.interrupt_status & (1 << 27) != 0 }
    pub const fn has_command_error(self) -> bool {
        self.interrupt_status & AHCI_PORT_ERROR_INTERRUPT_MASK != 0
            || self.task_file_error() || self.task_file_device_fault()
    }
}

/// Read a diagnostic snapshot without acknowledging write-one-to-clear
/// registers. Callers can retain the snapshot before performing recovery.
pub fn snapshot_port<R: AhciRegisterIo>(
    io: &mut R,
    base: u32,
) -> Result<AhciPortSnapshot, AhciError> {
    Ok(AhciPortSnapshot {
        interrupt_status: io.read32(port_register(base, 0x10)?)?,
        task_file_data: io.read32(port_register(base, 0x20)?)?,
        sata_error: io.read32(port_register(base, 0x30)?)?,
        command_issue: io.read32(port_register(base, 0x38)?)?,
        sata_active: io.read32(port_register(base, 0x34)?)?,
    })
}

/// Acknowledge only the supplied observed W1C bits. This avoids clearing
/// unrelated status bits that may have arrived after a snapshot was taken.
pub fn acknowledge_port_status<R: AhciRegisterIo>(
    io: &mut R,
    base: u32,
    interrupt_bits: u32,
    sata_error_bits: u32,
) -> Result<(), AhciError> {
    if interrupt_bits != 0 {
        io.write32(port_register(base, 0x10)?, interrupt_bits)?;
    }
    if sata_error_bits != 0 {
        io.write32(port_register(base, 0x30)?, sata_error_bits)?;
    }
    Ok(())
}

#[cfg(test)]
mod recovery_status_tests {
    use super::*;

    struct StatusRegisters([u32; 64]);
    impl AhciRegisterIo for StatusRegisters {
        fn read32(&mut self, offset: u32) -> Result<u32, AhciError> {
            Ok(self.0[(offset / 4) as usize])
        }
        fn write32(&mut self, offset: u32, value: u32) -> Result<(), AhciError> {
            let register = &mut self.0[(offset / 4) as usize];
            if offset == 0x10 || offset == 0x30 { *register &= !value; }
            else { *register = value; }
            Ok(())
        }
    }

    #[test]
    fn snapshot_reports_command_and_link_faults_without_acknowledging() {
        let mut regs = StatusRegisters([0; 64]);
        regs.0[0x10 / 4] = (1 << 30) | (1 << 27);
        regs.0[0x20 / 4] = (1 << 0) | (1 << 5);
        regs.0[0x30 / 4] = 0xA5;
        regs.0[0x38 / 4] = 1 << 3;
        let status = snapshot_port(&mut regs, 0).unwrap();
        assert!(status.has_command_error());
        assert!(status.interface_fatal());
        assert!(status.task_file_error());
        assert!(status.task_file_device_fault());
        assert_eq!(status.sata_error, 0xA5);
        assert_eq!(regs.0[0x10 / 4], (1 << 30) | (1 << 29));
    }

    #[test]
    fn acknowledges_only_requested_w1c_bits() {
        let mut regs = StatusRegisters([0; 64]);
        regs.0[0x10 / 4] = 0b1111;
        regs.0[0x30 / 4] = 0b1010;
        acknowledge_port_status(&mut regs, 0, 0b0101, 0b0010).unwrap();
        assert_eq!(regs.0[0x10 / 4], 0b1010);
        assert_eq!(regs.0[0x30 / 4], 0b1000);
    }

    #[test]
    fn task_file_ready_rejects_busy_and_data_request() {
        let ready = AhciPortSnapshot { interrupt_status: 0, task_file_data: 0, sata_error: 0, command_issue: 0, sata_active: 0 };
        assert!(ready.task_file_ready());
        let busy = AhciPortSnapshot { task_file_data: 1 << 7, ..ready };
        let data_request = AhciPortSnapshot { task_file_data: 1 << 3, ..ready };
        let error = AhciPortSnapshot { task_file_data: 1, ..ready };
        let device_fault = AhciPortSnapshot { task_file_data: 1 << 5, ..ready };
        assert!(!busy.task_file_ready());
        assert!(!data_request.task_file_ready());
        assert!(!error.task_file_ready());
        assert!(!device_fault.task_file_ready());
    }
}
