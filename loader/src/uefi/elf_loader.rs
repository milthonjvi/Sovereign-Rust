//! ELF64 PT_LOAD image allocation and population for Sovereign Boot.
//!
//! The loader reserves one contiguous physical span for the kernel's virtual
//! image, zeroes it, and copies each validated PT_LOAD payload. The caller
//! constructs permission-correct page mappings before transferring control.

use core::{ffi::c_void, ptr};

use super::{
    boot_services::{allocate_loader_pages, EfiBootServices},
    protocols::{
        EfiFileProtocol, EfiLoadedImageProtocol, EfiSimpleFileSystemProtocol,
        EFI_FILE_INFO_GUID, EFI_FILE_MODE_READ, EFI_LOADED_IMAGE_PROTOCOL_GUID,
        EFI_SIMPLE_FILE_SYSTEM_PROTOCOL_GUID,
    },
    EfiHandle, EfiStatus, EfiSystemTable, EFI_COMPROMISED_DATA,
    EFI_INVALID_PARAMETER, EFI_SUCCESS, EFI_UNSUPPORTED,
};

const ELF_HEADER_SIZE: usize = 64;
const ELF_PROGRAM_HEADER_SIZE: usize = 56;
const MAX_PROGRAM_HEADERS: usize = 128;
const FILE_INFO_BUFFER_SIZE: usize = 512;
const PAGE_SIZE: u64 = 4096;
const MAX_KERNEL_IMAGE_SIZE: u64 = 256 * 1024 * 1024;
const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const ET_EXEC: u16 = 2;
const EM_X86_64: u16 = 62;
const PT_LOAD: u32 = 1;
const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;
const KERNEL_PATH: [u16; 31] = [
    92,69,70,73,92,66,79,79,84,92,115,111,118,101,114,101,105,103,110,45,
    107,101,114,110,101,108,46,101,108,102,0,
];

#[derive(Clone, Copy, Debug)]
pub struct LoadedSegment {
    pub virtual_address: u64,
    pub file_size: u64,
    pub memory_size: u64,
    pub flags: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct LoadedKernel {
    pub physical_base: u64,
    pub virtual_base: u64,
    pub image_size: u64,
    pub entry: u64,
    pub pages: usize,
    pub loader_image_base: u64,
    pub loader_image_size: u64,
    pub segments: [LoadedSegment; MAX_PROGRAM_HEADERS],
    pub segment_count: usize,
}

#[derive(Clone, Copy)]
struct Segment {
    offset: u64,
    virtual_address: u64,
    file_size: u64,
    memory_size: u64,
    flags: u32,
}
impl Segment {
    const EMPTY: Self = Self { offset: 0, virtual_address: 0, file_size: 0, memory_size: 0, flags: 0 };
}

pub fn load_kernel_file(
    image_handle: EfiHandle,
    system_table: &mut EfiSystemTable,
) -> Result<LoadedKernel, EfiStatus> {
    if image_handle.is_null() || system_table.boot_services.is_null() {
        return Err(EFI_INVALID_PARAMETER);
    }
    // SAFETY: The system table contains the live firmware Boot Services pointer.
    let boot = unsafe { &mut *(system_table.boot_services as *mut EfiBootServices) };
    let handle_protocol = boot.handle_protocol.ok_or(EFI_UNSUPPORTED)?;

    let mut loaded_ptr: *mut c_void = ptr::null_mut();
    // SAFETY: UEFI protocol lookup receives valid GUID and output storage.
    let status = unsafe { handle_protocol(image_handle, &EFI_LOADED_IMAGE_PROTOCOL_GUID, &mut loaded_ptr) };
    if status != EFI_SUCCESS { return Err(status); }
    if loaded_ptr.is_null() { return Err(EFI_COMPROMISED_DATA); }
    // SAFETY: HandleProtocol returned the loaded-image interface.
    let loaded = unsafe { &*(loaded_ptr as *const EfiLoadedImageProtocol) };
    if loaded.device_handle.is_null() || loaded.image_base.is_null() || loaded.image_size == 0 { return Err(EFI_COMPROMISED_DATA); }
    let loader_image_base = loaded.image_base as usize as u64;
    let loader_image_size = u64::try_from(loaded.image_size).map_err(|_| EFI_COMPROMISED_DATA)?;

    let mut fs_ptr: *mut c_void = ptr::null_mut();
    // SAFETY: Device handle is supplied by the loaded-image protocol.
    let status = unsafe { handle_protocol(loaded.device_handle, &EFI_SIMPLE_FILE_SYSTEM_PROTOCOL_GUID, &mut fs_ptr) };
    if status != EFI_SUCCESS { return Err(status); }
    if fs_ptr.is_null() { return Err(EFI_COMPROMISED_DATA); }
    // SAFETY: HandleProtocol returned the simple-filesystem interface.
    let fs = unsafe { &mut *(fs_ptr as *mut EfiSimpleFileSystemProtocol) };
    let open_volume = fs.open_volume.ok_or(EFI_UNSUPPORTED)?;
    let mut root: *mut EfiFileProtocol = ptr::null_mut();
    // SAFETY: Root output pointer is writable and fs is a valid protocol.
    let status = unsafe { open_volume(fs, &mut root) };
    if status != EFI_SUCCESS { return Err(status); }
    if root.is_null() { return Err(EFI_COMPROMISED_DATA); }

    let result = open_and_load(root, boot, loader_image_base, loader_image_size);
    close_file(root);
    result
}

fn open_and_load(
    root: *mut EfiFileProtocol,
    boot: &mut EfiBootServices,
    loader_image_base: u64,
    loader_image_size: u64,
) -> Result<LoadedKernel, EfiStatus> {
    // SAFETY: Root is a valid protocol returned by OpenVolume.
    let open = unsafe { root.as_ref() }.and_then(|f| f.open).ok_or(EFI_UNSUPPORTED)?;
    let mut file: *mut EfiFileProtocol = ptr::null_mut();
    // SAFETY: Path is NUL-terminated UTF-16 and output pointer is valid.
    let status = unsafe { open(root, &mut file, KERNEL_PATH.as_ptr(), EFI_FILE_MODE_READ, 0) };
    if status != EFI_SUCCESS { return Err(status); }
    if file.is_null() { return Err(EFI_COMPROMISED_DATA); }
    let result = load_open_file(file, boot, loader_image_base, loader_image_size);
    close_file(file);
    result
}

fn load_open_file(
    file: *mut EfiFileProtocol,
    boot: &mut EfiBootServices,
    loader_image_base: u64,
    loader_image_size: u64,
) -> Result<LoadedKernel, EfiStatus> {
    // SAFETY: File is a live EFI_FILE_PROTOCOL handle.
    let protocol = unsafe { file.as_ref() }.ok_or(EFI_INVALID_PARAMETER)?;
    let read = protocol.read.ok_or(EFI_UNSUPPORTED)?;
    let set_position = protocol.set_position.ok_or(EFI_UNSUPPORTED)?;
    let get_info = protocol.get_info.ok_or(EFI_UNSUPPORTED)?;

    let mut info = [0u8; FILE_INFO_BUFFER_SIZE];
    let mut info_size = info.len();
    // SAFETY: Info buffer is writable and size is initialized to capacity.
    let status = unsafe { get_info(file, &EFI_FILE_INFO_GUID, &mut info_size, info.as_mut_ptr().cast()) };
    if status != EFI_SUCCESS { return Err(status); }
    if info_size < 16 || info_size > info.len() { return Err(EFI_COMPROMISED_DATA); }
    let file_size = read_u64(&info, 8);

    let mut header = [0u8; ELF_HEADER_SIZE];
    let mut header_size = header.len();
    // SAFETY: Header buffer is writable for its full capacity.
    let status = unsafe { read(file, &mut header_size, header.as_mut_ptr().cast()) };
    if status != EFI_SUCCESS { return Err(status); }
    if header_size != ELF_HEADER_SIZE || file_size < ELF_HEADER_SIZE as u64 { return Err(EFI_COMPROMISED_DATA); }
    if header[0..4] != ELF_MAGIC || header[4] != 2 || header[5] != 1 || header[6] != 1
        || read_u16(&header, 16) != ET_EXEC || read_u16(&header, 18) != EM_X86_64
        || read_u32(&header, 20) != 1 || read_u16(&header, 52) as usize != ELF_HEADER_SIZE {
        return Err(EFI_COMPROMISED_DATA);
    }

    let entry = read_u64(&header, 24);
    let phoff = read_u64(&header, 32);
    let phentsize = read_u16(&header, 54);
    let phnum = read_u16(&header, 56) as usize;
    if phoff < ELF_HEADER_SIZE as u64 || phentsize as usize != ELF_PROGRAM_HEADER_SIZE
        || phnum == 0 || phnum > MAX_PROGRAM_HEADERS {
        return Err(EFI_COMPROMISED_DATA);
    }
    let table_end = phoff.checked_add((phentsize as u64).checked_mul(phnum as u64).ok_or(EFI_COMPROMISED_DATA)?)
        .ok_or(EFI_COMPROMISED_DATA)?;
    if table_end > file_size { return Err(EFI_COMPROMISED_DATA); }

    let mut segments = [Segment::EMPTY; MAX_PROGRAM_HEADERS];
    let mut count = 0usize;
    let mut virtual_base = u64::MAX;
    let mut virtual_end = 0u64;
    let mut entry_executable = false;
    let mut header_buf = [0u8; ELF_PROGRAM_HEADER_SIZE];

    for i in 0..phnum {
        let offset = phoff.checked_add((i as u64).checked_mul(ELF_PROGRAM_HEADER_SIZE as u64).ok_or(EFI_COMPROMISED_DATA)?)
            .ok_or(EFI_COMPROMISED_DATA)?;
        // SAFETY: File position is a validated offset within file_size.
        let status = unsafe { set_position(file, offset) };
        if status != EFI_SUCCESS { return Err(status); }
        let mut bytes = header_buf.len();
        // SAFETY: Fixed program-header buffer is writable.
        let status = unsafe { read(file, &mut bytes, header_buf.as_mut_ptr().cast()) };
        if status != EFI_SUCCESS { return Err(status); }
        if bytes != ELF_PROGRAM_HEADER_SIZE { return Err(EFI_COMPROMISED_DATA); }
        if read_u32(&header_buf, 0) != PT_LOAD { continue; }

        let flags = read_u32(&header_buf, 4);
        let segment = Segment {
            offset: read_u64(&header_buf, 8),
            virtual_address: read_u64(&header_buf, 16),
            file_size: read_u64(&header_buf, 32),
            memory_size: read_u64(&header_buf, 40),
            flags,
        };
        if segment.file_size > segment.memory_size
            || flags & !0x7 != 0
            || flags & PF_R == 0
            || flags & (PF_W | PF_X) == (PF_W | PF_X)
        {
            return Err(EFI_COMPROMISED_DATA);
        }
        let file_end = segment.offset.checked_add(segment.file_size).ok_or(EFI_COMPROMISED_DATA)?;
        if file_end > file_size { return Err(EFI_COMPROMISED_DATA); }
        let alignment = read_u64(&header_buf, 48);
        if alignment > 1 && (!alignment.is_power_of_two()
            || segment.offset % alignment != segment.virtual_address % alignment) {
            return Err(EFI_COMPROMISED_DATA);
        }
        if segment.memory_size == 0 { continue; }
        let end = segment.virtual_address.checked_add(segment.memory_size).ok_or(EFI_COMPROMISED_DATA)?;
        if !canonical(segment.virtual_address) || !canonical(end - 1)
            || (segment.virtual_address >> 47) != ((end - 1) >> 47) {
            return Err(EFI_COMPROMISED_DATA);
        }
        let page_start = align_down(segment.virtual_address);
        let page_end = align_up(end).ok_or(EFI_COMPROMISED_DATA)?;
        // Reject page overlap: a shared page with distinct segment permissions
        // would make W^X enforcement ambiguous at page granularity.
        for previous in segments.iter().take(count) {
            let prev_start = align_down(previous.virtual_address);
            let prev_end = align_up(previous.virtual_address.checked_add(previous.memory_size).ok_or(EFI_COMPROMISED_DATA)?)
                .ok_or(EFI_COMPROMISED_DATA)?;
            if page_start < prev_end && prev_start < page_end { return Err(EFI_COMPROMISED_DATA); }
        }
        if flags & PF_X != 0 && entry >= segment.virtual_address && entry < end {
            entry_executable = true;
        }
        virtual_base = virtual_base.min(page_start);
        virtual_end = virtual_end.max(page_end);
        segments[count] = segment;
        count += 1;
    }

    if count == 0 || !canonical(entry) || !entry_executable || virtual_end <= virtual_base {
        return Err(EFI_COMPROMISED_DATA);
    }
    let image_size = virtual_end - virtual_base;
    if image_size > MAX_KERNEL_IMAGE_SIZE { return Err(EFI_COMPROMISED_DATA); }
    let pages_u64 = image_size / PAGE_SIZE;
    let pages = usize::try_from(pages_u64).map_err(|_| EFI_COMPROMISED_DATA)?;
    let allocate = boot.allocate_pages.ok_or(EFI_UNSUPPORTED)?;
    let mut physical_base = 0u64;
    // SAFETY: Boot Services is live and the allocation output is writable.
    let status = unsafe { allocate(0, 2, pages, &mut physical_base) };
    if status != EFI_SUCCESS { return Err(status); }
    if physical_base == 0 || physical_base & (PAGE_SIZE - 1) != 0
        || physical_base.checked_add(image_size).is_none() {
        return Err(EFI_COMPROMISED_DATA);
    }

    // UEFI x86_64 loader execution currently relies on the firmware identity
    // mapping for AllocatePages memory. This assumption is explicit and will
    // be removed when the final loader address-space transition is installed.
    let image_ptr = physical_base as usize as *mut u8;
    // SAFETY: AllocatePages returned a writable contiguous span; zeroing it
    // initializes both inter-segment gaps and every segment's BSS.
    unsafe { ptr::write_bytes(image_ptr, 0, image_size as usize); }

    for segment in segments.iter().take(count) {
        if segment.file_size == 0 { continue; }
        let destination = physical_base.checked_add(segment.virtual_address - virtual_base)
            .ok_or(EFI_COMPROMISED_DATA)? as usize as *mut u8;
        // SAFETY: Segment destination lies inside the reserved image span.
        // Source reads are bounded by the validated file offset and file size.
        // SAFETY: SetPosition and Read operate on the live file protocol.
        let status = unsafe { set_position(file, segment.offset) };
        if status != EFI_SUCCESS { return Err(status); }
        let mut remaining = usize::try_from(segment.file_size).map_err(|_| EFI_COMPROMISED_DATA)?;
        let mut cursor = destination;
        while remaining != 0 {
            let mut chunk = remaining;
            // SAFETY: cursor points into the allocated image span.
            let status = unsafe { read(file, &mut chunk, cursor.cast::<c_void>()) };
            if status != EFI_SUCCESS { return Err(status); }
            if chunk == 0 || chunk > remaining { return Err(EFI_COMPROMISED_DATA); }
            // SAFETY: The firmware reports the exact number of bytes written.
            cursor = unsafe { cursor.add(chunk) };
            remaining -= chunk;
        }
    }

    let mut loaded_segments = [LoadedSegment { virtual_address: 0, file_size: 0, memory_size: 0, flags: 0 }; MAX_PROGRAM_HEADERS];
    for (index, segment) in segments.iter().take(count).enumerate() {
        loaded_segments[index] = LoadedSegment {
            virtual_address: segment.virtual_address,
            file_size: segment.file_size,
            memory_size: segment.memory_size,
            flags: segment.flags,
        };
    }
    Ok(LoadedKernel {
        physical_base, virtual_base, image_size, entry, pages,
        loader_image_base, loader_image_size,
        segments: loaded_segments, segment_count: count,
    })
}

fn align_down(value: u64) -> u64 { value & !(PAGE_SIZE - 1) }
fn align_up(value: u64) -> Option<u64> { value.checked_add(PAGE_SIZE - 1).map(align_down) }
fn canonical(value: u64) -> bool {
    let upper = value >> 48;
    let sign = (value >> 47) & 1;
    (sign == 0 && upper == 0) || (sign == 1 && upper == 0xffff)
}
fn read_u16(b: &[u8], o: usize) -> u16 { u16::from_le_bytes([b[o], b[o + 1]]) }
fn read_u32(b: &[u8], o: usize) -> u32 { u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) }
fn read_u64(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes([b[o],b[o+1],b[o+2],b[o+3],b[o+4],b[o+5],b[o+6],b[o+7]])
}
fn close_file(file: *mut EfiFileProtocol) {
    // SAFETY: The caller passes a protocol handle returned by Open/OpenVolume.
    if let Some(close) = unsafe { file.as_ref() }.and_then(|f| f.close) {
        // SAFETY: The handle remains open and owned by this loader.
        let _ = unsafe { close(file) };
    }
}
