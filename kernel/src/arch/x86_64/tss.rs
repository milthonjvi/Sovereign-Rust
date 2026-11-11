//! x86_64 Global Descriptor Table and Task State Segment.
//!
//! Installs a private kernel GDT and a TSS with dedicated IST stacks for
//! double fault, NMI, and machine-check delivery. No allocator is required.

use core::mem::size_of;
use core::sync::atomic::{AtomicBool, Ordering};

const KERNEL_CODE_SELECTOR: u16 = 0x08;
const KERNEL_DATA_SELECTOR: u16 = 0x10;
const TSS_SELECTOR: u16 = 0x18;
const STACK_SIZE: usize = 16 * 1024;

#[repr(C, packed)]
struct TaskStateSegment {
    reserved0: u32,
    rsp: [u64; 3],
    reserved1: u64,
    ist: [u64; 7],
    reserved2: u64,
    reserved3: u16,
    io_map_base: u16,
}

impl TaskStateSegment {
    const fn new() -> Self {
        Self {
            reserved0: 0,
            rsp: [0; 3],
            reserved1: 0,
            ist: [0; 7],
            reserved2: 0,
            reserved3: 0,
            io_map_base: size_of::<Self>() as u16,
        }
    }
}

#[repr(C, align(16))]
struct Stack([u8; STACK_SIZE]);

static mut TSS: TaskStateSegment = TaskStateSegment::new();
static mut GDT: [u64; 5] = [0; 5];
static mut DOUBLE_FAULT_STACK: Stack = Stack([0; STACK_SIZE]);
static mut NMI_STACK: Stack = Stack([0; STACK_SIZE]);
static mut MACHINE_CHECK_STACK: Stack = Stack([0; STACK_SIZE]);
static INSTALLED: AtomicBool = AtomicBool::new(false);

#[repr(C, packed)]
struct DescriptorTablePointer {
    limit: u16,
    base: u64,
}

core::arch::global_asm!(
    ".intel_syntax noprefix",
    ".global sovereign_load_gdt_and_tss",
    ".type sovereign_load_gdt_and_tss, @function",
    "sovereign_load_gdt_and_tss:",
    "lgdt [rdi]",
    "push 0x08",
    "lea rax, [rip + 1f]",
    "push rax",
    "retfq",
    "1:",
    "mov ax, 0x10",
    "mov ds, ax",
    "mov es, ax",
    "mov ss, ax",
    "xor eax, eax",
    "mov fs, ax",
    "mov gs, ax",
    "mov ax, 0x18",
    "ltr ax",
    "ret",
);

unsafe extern "C" {
    fn sovereign_load_gdt_and_tss(pointer: *const DescriptorTablePointer);
}

/// TSS/GDT initialization failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TssError {
    AlreadyInstalled,
}

/// Installs the kernel GDT and loads the TSS on the current CPU.
///
/// The function is intended for BSP initialization while interrupts are
/// disabled. Each AP must install its own TSS and per-CPU GDT before startup.
///
/// # Safety
/// Must execute at CPL 0 with interrupts disabled, before loading IDT gates
/// that use IST. The static kernel stacks and descriptor tables must remain
/// mapped and writable for the lifetime of this CPU.
pub unsafe fn install() -> Result<(), TssError> {
    if INSTALLED.load(Ordering::Acquire) {
        return Err(TssError::AlreadyInstalled);
    }

    // SAFETY: The caller serializes early BSP initialization with IF clear.
    unsafe {
        let tss = core::ptr::addr_of_mut!(TSS);
        let df_top = core::ptr::addr_of_mut!(DOUBLE_FAULT_STACK.0) as *mut u8 as u64
            + STACK_SIZE as u64;
        let nmi_top = core::ptr::addr_of_mut!(NMI_STACK.0) as *mut u8 as u64
            + STACK_SIZE as u64;
        let mc_top = core::ptr::addr_of_mut!(MACHINE_CHECK_STACK.0) as *mut u8 as u64
            + STACK_SIZE as u64;

        core::ptr::addr_of_mut!((*tss).ist[0]).write_unaligned(df_top);
        core::ptr::addr_of_mut!((*tss).ist[1]).write_unaligned(nmi_top);
        core::ptr::addr_of_mut!((*tss).ist[2]).write_unaligned(mc_top);

        let base = tss as u64;
        let limit = (size_of::<TaskStateSegment>() - 1) as u64;
        let low = (limit & 0xFFFF)
            | ((base & 0x00FF_FFFF) << 16)
            | (0x89 << 40)
            | ((limit & 0xF0000) << 32)
            | ((base & 0xFF00_0000) << 32);
        let high = base >> 32;

        let gdt = core::ptr::addr_of_mut!(GDT);
        (*gdt)[0] = 0;
        (*gdt)[1] = 0x00AF_9A00_0000_FFFF;
        (*gdt)[2] = 0x00CF_9200_0000_FFFF;
        (*gdt)[3] = low;
        (*gdt)[4] = high;

        let pointer = DescriptorTablePointer {
            limit: (size_of::<[u64; 5]>() - 1) as u16,
            base: gdt as u64,
        };
        sovereign_load_gdt_and_tss(&pointer);
    }

    INSTALLED.store(true, Ordering::Release);
    Ok(())
}

/// Updates the ring-0 stack used when entering from a less-privileged ring.
///
/// # Safety
/// The stack must be canonical, mapped, writable, and have a valid top-of-stack
/// address. Call only while the target CPU cannot concurrently enter user mode.
pub unsafe fn set_ring0_stack(stack_top: u64) {
    // SAFETY: Caller guarantees valid stack and serialized per-CPU update.
    unsafe {
        core::ptr::addr_of_mut!(TSS.rsp[0]).write_unaligned(stack_top);
    }
}

/// Returns whether this CPU has loaded the kernel TSS.
pub fn is_installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

/// Selectors used by the installed kernel GDT.
pub const fn selectors() -> (u16, u16, u16) {
    (KERNEL_CODE_SELECTOR, KERNEL_DATA_SELECTOR, TSS_SELECTOR)
}

const _: [(); 104] = [(); size_of::<TaskStateSegment>()];
const _: [(); 10] = [(); size_of::<DescriptorTablePointer>()];
