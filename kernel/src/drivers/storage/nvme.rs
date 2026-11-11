//! NVMe protocol primitives; PCI, MMIO, DMA allocation and IRQ routing are external.
use super::block::{BlockError, BlockGeometry};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NvmeError { InvalidCapabilities, InvalidQueueDepth, QueueFull, InvalidNamespace, UnsupportedLbaFormat, InvalidTransfer, AddressOverflow, InvalidAlignment, PrpListTooSmall }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NvmeCapabilities {
    pub max_queue_entries: u32,
    pub contiguous_queues_required: bool,
    pub timeout_units_500ms: u8,
    pub doorbell_stride_bytes: u32,
    pub min_page_size: u32,
    pub max_page_size: u32,
}
impl NvmeCapabilities {
    pub fn parse(cap: u64) -> Result<Self, NvmeError> {
        let min_shift = 12 + ((cap >> 48) & 0xF) as u32;
        let max_shift = 12 + ((cap >> 52) & 0xF) as u32;
        if min_shift > max_shift { return Err(NvmeError::InvalidCapabilities); }
        Ok(Self {
            max_queue_entries: (cap & 0xFFFF) as u32 + 1,
            contiguous_queues_required: cap & (1 << 16) != 0,
            timeout_units_500ms: (cap >> 24) as u8,
            doorbell_stride_bytes: 4u32.checked_shl(((cap >> 32) & 0xF) as u32).ok_or(NvmeError::InvalidCapabilities)?,
            min_page_size: 1u32.checked_shl(min_shift).ok_or(NvmeError::InvalidCapabilities)?,
            max_page_size: 1u32.checked_shl(max_shift).ok_or(NvmeError::InvalidCapabilities)?,
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NvmeCommand {
    pub opcode:u8, pub flags:u8, pub command_id:u16, pub namespace_id:u32,
    pub cdw2:u32, pub cdw3:u32, pub metadata_pointer:u64, pub prp1:u64, pub prp2:u64,
    pub cdw10:u32, pub cdw11:u32, pub cdw12:u32, pub cdw13:u32, pub cdw14:u32, pub cdw15:u32,
}
const _: [();64] = [(); core::mem::size_of::<NvmeCommand>()];

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NvmeCompletion {
    pub result:u32, pub reserved:u32, pub submission_queue_head:u16,
    pub submission_queue_id:u16, pub command_id:u16, pub status:u16,
}
const _: [();16] = [(); core::mem::size_of::<NvmeCompletion>()];
impl NvmeCompletion {
    pub const fn phase(self)->bool { self.status & 1 != 0 }
    pub const fn status_code(self)->u8 { ((self.status >> 1) & 0xFF) as u8 }
    pub const fn status_code_type(self)->u8 { ((self.status >> 9) & 7) as u8 }
    pub const fn do_not_retry(self)->bool { self.status & 0x8000 != 0 }
    pub const fn successful(self)->bool { self.status_code()==0 && self.status_code_type()==0 }
}

pub struct NvmeSubmissionQueue<'a> { entries:&'a mut [NvmeCommand], depth:u32, head:u32, tail:u32 }
impl<'a> NvmeSubmissionQueue<'a> {
    pub fn new(entries:&'a mut [NvmeCommand], depth:u32)->Result<Self,NvmeError>{
        if depth<2 || depth as usize>entries.len(){return Err(NvmeError::InvalidQueueDepth);}
        entries[..depth as usize].fill(NvmeCommand::default());
        Ok(Self{entries,depth,head:0,tail:0})
    }
    pub const fn tail(&self)->u32{self.tail}
    pub fn update_head(&mut self,head:u32)->Result<(),NvmeError>{
        if head>=self.depth{return Err(NvmeError::InvalidQueueDepth);} self.head=head; Ok(())
    }
    /// Returns the new tail for the submission doorbell. Caller must issue the
    /// platform DMA write barrier before ringing the MMIO doorbell.
    pub fn submit(&mut self,cmd:NvmeCommand)->Result<u32,NvmeError>{
        let next=(self.tail+1)%self.depth;
        if next==self.head{return Err(NvmeError::QueueFull);}
        // SAFETY: tail is bounded by depth, and the caller owns the DMA queue mapping.
        unsafe { core::ptr::write_volatile(self.entries.as_mut_ptr().add(self.tail as usize), cmd); }
        self.tail=next; Ok(next)
    }
}

pub struct NvmeCompletionQueue<'a>{entries:&'a [NvmeCompletion],depth:u32,head:u32,phase:bool}
impl<'a> NvmeCompletionQueue<'a>{
    pub fn new(entries:&'a [NvmeCompletion],depth:u32)->Result<Self,NvmeError>{
        if depth<2 || depth as usize>entries.len(){return Err(NvmeError::InvalidQueueDepth);}
        Ok(Self{entries,depth,head:0,phase:true})
    }
    pub const fn head(&self)->u32{self.head}
    /// Caller must issue the platform DMA read barrier before polling.
    pub fn poll(&mut self)->Option<NvmeCompletion>{
        // SAFETY: head is bounded by depth; caller must perform its DMA read barrier before polling.
        let c=unsafe { core::ptr::read_volatile(self.entries.as_ptr().add(self.head as usize)) };
        if c.phase()!=self.phase{return None;}
        self.head+=1; if self.head==self.depth{self.head=0;self.phase=!self.phase;} Some(c)
    }
}

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct NvmePrpMapping{pub prp1:u64,pub prp2:u64,pub list_entries:usize}
/// Map one physically contiguous DMA buffer. Supports PRP1, direct PRP2, or
/// one PRP-list page; larger lists are rejected rather than silently truncated.
pub fn build_prps(buffer_phys:u64,byte_len:usize,page_size:u32,list_phys:u64,list:&mut[u64])->Result<NvmePrpMapping,NvmeError>{
    if byte_len==0 || !page_size.is_power_of_two() || page_size<4096{return Err(NvmeError::InvalidTransfer);}
    let page=page_size as u64;
    let total=byte_len as u64;
    buffer_phys.checked_add(total).ok_or(NvmeError::AddressOverflow)?;
    let first=core::cmp::min(total,page-(buffer_phys&(page-1)));
    let remaining=byte_len as u64-first;
    let pages=if remaining==0{0}else{remaining.checked_add(page-1).ok_or(NvmeError::AddressOverflow)?/page};
    if pages==0{return Ok(NvmePrpMapping{prp1:buffer_phys,prp2:0,list_entries:0});}
    let second=buffer_phys.checked_add(first).ok_or(NvmeError::AddressOverflow)?;
    if pages==1{return Ok(NvmePrpMapping{prp1:buffer_phys,prp2:second,list_entries:0});}
    if list_phys&(page-1)!=0{return Err(NvmeError::InvalidAlignment);}
    list_phys.checked_add(page).ok_or(NvmeError::AddressOverflow)?;
    if pages as usize>list.len() || pages*8>page{return Err(NvmeError::PrpListTooSmall);}
    for (i,slot) in list[..pages as usize].iter_mut().enumerate(){
        *slot=second.checked_add((i as u64).checked_mul(page).ok_or(NvmeError::AddressOverflow)?).ok_or(NvmeError::AddressOverflow)?;
    }
    Ok(NvmePrpMapping{prp1:buffer_phys,prp2:list_phys,list_entries:pages as usize})
}

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub struct NvmeNamespace{pub block_count:u64,pub block_size:u32,pub active_lba_format:u8,pub metadata_size:u16}
impl NvmeNamespace{
    pub fn parse_identify(data:&[u8])->Result<Self,NvmeError>{
        if data.len()<132{return Err(NvmeError::InvalidNamespace);}
        let nsze=u64::from_le_bytes(data[0..8].try_into().map_err(|_|NvmeError::InvalidNamespace)?);
        let ncap=u64::from_le_bytes(data[8..16].try_into().map_err(|_|NvmeError::InvalidNamespace)?);
        let nlbaf=data[25]; let flbas=data[26]&0x0F;
        if nsze==0 || ncap>nsze || flbas>nlbaf{return Err(NvmeError::InvalidNamespace);}
        let off=128+flbas as usize*4;
        let metadata=u16::from_le_bytes([data[off],data[off+1]]);
        let shift=data[off+2];
        if metadata!=0 || !(9..=16).contains(&shift){return Err(NvmeError::UnsupportedLbaFormat);}
        Ok(Self{block_count:nsze,block_size:1u32.checked_shl(shift as u32).ok_or(NvmeError::UnsupportedLbaFormat)?,active_lba_format:flbas,metadata_size:metadata})
    }
    pub fn geometry(self)->Result<BlockGeometry,BlockError>{
        BlockGeometry{block_count:self.block_count,block_size:self.block_size}.validate()
    }
}

pub fn identify_command(cid:u16,nsid:u32,cns:u8,prp1:u64,prp2:u64)->NvmeCommand{
    NvmeCommand{opcode:0x06,command_id:cid,namespace_id:nsid,prp1,prp2,cdw10:cns as u32,..NvmeCommand::default()}
}
pub fn read_command(cid:u16,nsid:u32,slba:u64,nlb:u32,prp1:u64,prp2:u64)->Result<NvmeCommand,NvmeError>{
    io_command(0x02,cid,nsid,slba,nlb,prp1,prp2)
}
pub fn write_command(cid:u16,nsid:u32,slba:u64,nlb:u32,prp1:u64,prp2:u64)->Result<NvmeCommand,NvmeError>{
    io_command(0x01,cid,nsid,slba,nlb,prp1,prp2)
}
fn io_command(op:u8,cid:u16,nsid:u32,slba:u64,nlb:u32,prp1:u64,prp2:u64)->Result<NvmeCommand,NvmeError>{
    if nsid==0 || nlb==0 || nlb>65_536 || slba.checked_add(nlb as u64).is_none(){return Err(NvmeError::InvalidTransfer);}
    Ok(NvmeCommand{opcode:op,command_id:cid,namespace_id:nsid,prp1,prp2,cdw10:slba as u32,cdw11:(slba>>32) as u32,cdw12:nlb-1,..NvmeCommand::default()})
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]fn command_is_64_bytes(){assert_eq!(core::mem::size_of::<NvmeCommand>(),64);}
    #[test]fn prps_cover_one_two_and_many_pages(){
        let one=build_prps(0x1003,100,4096,0x8000,&mut[0;8]).unwrap(); assert_eq!(one.prp2,0);
        let two=build_prps(0x1000,8192,4096,0x8000,&mut[0;8]).unwrap(); assert_eq!(two.prp2,0x2000);
        let mut list=[0u64;8]; let many=build_prps(0x1000,12288,4096,0x8000,&mut list).unwrap();
        assert_eq!(many.list_entries,2); assert_eq!(&list[..2],&[0x2000,0x3000]);
    }
    #[test]fn submission_queue_reserves_one_slot(){
        let mut entries=[NvmeCommand::default();3]; let mut q=NvmeSubmissionQueue::new(&mut entries,3).unwrap();
        assert_eq!(q.submit(NvmeCommand::default()),Ok(1)); assert_eq!(q.submit(NvmeCommand::default()),Ok(2));
        assert_eq!(q.submit(NvmeCommand::default()),Err(NvmeError::QueueFull)); q.update_head(1).unwrap();
        assert_eq!(q.submit(NvmeCommand::default()),Ok(0));
    }
    #[test]fn namespace_reads_selected_lba_format(){
        let mut data=[0u8;136]; data[..8].copy_from_slice(&1000u64.to_le_bytes()); data[8..16].copy_from_slice(&900u64.to_le_bytes());
        data[25]=1;data[26]=1;data[134]=9;
        let ns=NvmeNamespace::parse_identify(&data).unwrap();assert_eq!(ns.block_size,512);assert_eq!(ns.block_count,1000);
    }
}

#[cfg(test)]
mod capability_tests {
    use super::*;
    #[test]
    fn capabilities_decode_queue_and_page_limits() {
        let cap = 63u64 | (1u64 << 16) | (10u64 << 24) | (1u64 << 32) | (1u64 << 48) | (2u64 << 52);
        let decoded = NvmeCapabilities::parse(cap).unwrap();
        assert_eq!(decoded.max_queue_entries, 64);
        assert!(decoded.contiguous_queues_required);
        assert_eq!(decoded.doorbell_stride_bytes, 8);
        assert_eq!(decoded.min_page_size, 8192);
        assert_eq!(decoded.max_page_size, 32768);
    }
    #[test]
    fn completion_queue_tracks_phase_and_wrap() {
        let mut entries = [NvmeCompletion::default(); 2];
        entries[0].status = 1;
        entries[1].status = 1;
        let mut queue = NvmeCompletionQueue::new(&entries, 2).unwrap();
        assert!(queue.poll().is_some());
        assert!(queue.poll().is_some());
        assert_eq!(queue.head(), 0);
        assert!(queue.poll().is_none());
    }
}
