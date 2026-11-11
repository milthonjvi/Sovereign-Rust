//! Bootstrap CPU interrupt infrastructure initialization.
//!
//! This stage installs the kernel GDT/TSS, all 256 IDT fallback gates, the
//! legacy PIC remap, and PIC IRQ gates. It intentionally leaves IF clear and
//! every PIC line masked. Device drivers may register handlers and unmask
//! individual lines only after their own state is ready.

use core::sync::atomic::{AtomicU8, Ordering};

use crate::arch::x86_64::tss;
use super::{exceptions, idt::Idt, irq, pic::Pic8259};

static mut BOOTSTRAP_IDT: Idt = Idt::new();
static mut LEGACY_PIC: Pic8259 = unsafe { Pic8259::new() };

const STATE_UNINITIALIZED: u8 = 0;
const STATE_INITIALIZING: u8 = 1;
const STATE_READY: u8 = 2;
const STATE_FAILED: u8 = 3;
static STATE: AtomicU8 = AtomicU8::new(STATE_UNINITIALIZED);

/// Errors from bootstrap interrupt initialization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterruptInitError {
    AlreadyStarted,
    TssAlreadyInstalled,
    IdtConfiguration,
    IrqGateConfiguration,
    PicConfiguration,
}

/// Initialize the bootstrap processor's descriptor and legacy PIC infrastructure.
///
/// This function does not enable interrupts or unmask any IRQ. All device
/// handlers must be registered before a driver unmasks its line.
///
/// # Safety
/// Must execute on the bootstrap processor at CPL 0 with IF clear. No other
/// code may concurrently access the legacy PIC ports. The kernel image,
/// descriptor tables, and IST stacks must remain mapped and writable.
pub unsafe fn initialize_bootstrap() -> Result<(), InterruptInitError> {
    STATE.compare_exchange(
        STATE_UNINITIALIZED,
        STATE_INITIALIZING,
        Ordering::AcqRel,
        Ordering::Acquire,
    ).map_err(|_| InterruptInitError::AlreadyStarted)?;

    // A partial descriptor transition is not safely retryable. Keep a
    // permanent failed state rather than allowing a second initialization.
    let result = unsafe { initialize_inner() };
    STATE.store(
        if result.is_ok() { STATE_READY } else { STATE_FAILED },
        Ordering::Release,
    );
    result
}

unsafe fn initialize_inner() -> Result<(), InterruptInitError> {
    // SAFETY: Propagated from initialize_bootstrap; IF is clear on the BSP.
    unsafe { tss::install() }.map_err(|_| InterruptInitError::TssAlreadyInstalled)?;

    // SAFETY: Exclusive early-boot access to the unshared static IDT.
    let idt = unsafe { &mut *core::ptr::addr_of_mut!(BOOTSTRAP_IDT) };
    // SAFETY: The GDT/TSS is now active; the current CS selector is valid.
    unsafe { exceptions::install_exception_gates(idt) }
        .map_err(|_| InterruptInitError::IdtConfiguration)?;
    // SAFETY: PIC vectors 0x20..=0x2F replace their fatal fallback gates.
    unsafe { irq::install_irq_gates(idt, irq::IRQ_VECTOR_OFFSET) }
        .map_err(|_| InterruptInitError::IrqGateConfiguration)?;

    // Remap and mask every legacy IRQ before loading the completed IDT.
    // SAFETY: Exclusive PIC ownership, CPL 0, and IF clear are guaranteed.
    let pic = unsafe { &mut *core::ptr::addr_of_mut!(LEGACY_PIC) };
    unsafe { pic.initialize(0x20, 0x28) }
        .map_err(|_| InterruptInitError::PicConfiguration)?;

    // SAFETY: IDT is fully populated and has static storage duration.
    let idt = unsafe { &*core::ptr::addr_of!(BOOTSTRAP_IDT) };
    unsafe { idt.load() };
    Ok(())
}

/// Whether bootstrap descriptor and PIC initialization completed successfully.
pub fn is_initialized() -> bool {
    STATE.load(Ordering::Acquire) == STATE_READY
}

/// Whether initialization failed after beginning the descriptor transition.
pub fn is_failed() -> bool {
    STATE.load(Ordering::Acquire) == STATE_FAILED
}

/// Register a legacy PIC IRQ callback while keeping its source masked.
///
/// The callback receives the remapped vector (0x20..=0x2F), must be bounded,
/// nonblocking, must not unwind, and must call `end_legacy_irq` for genuine
/// interrupts before returning.
///
/// # Safety
/// Call with IF clear and while no other CPU or code path mutates the PIC or
/// dispatch table. The callback must remain valid for the kernel lifetime.
pub unsafe fn register_legacy_irq(
    irq_line: u8,
    handler: super::dispatch::InterruptHandler,
) -> Result<(), LegacyIrqError> {
    if !is_initialized() {
        return Err(LegacyIrqError::NotInitialized);
    }
    if irq_line >= 16 {
        return Err(LegacyIrqError::InvalidIrq);
    }

    // SAFETY: Caller serializes PIC state changes with IF clear.
    let pic = unsafe { &mut *core::ptr::addr_of_mut!(LEGACY_PIC) };
    pic.mask(irq_line).map_err(|_| LegacyIrqError::PicFailure)?;
    let vector = 0x20 + irq_line;
    super::dispatch::register(vector, handler)
        .map_err(|_| LegacyIrqError::HandlerAlreadyRegistered)
}

/// Unmask a previously registered legacy PIC IRQ line.
///
/// # Safety
/// Call with IF clear while the target device is fully initialized and its
/// handler is registered. The PIC must not be accessed concurrently elsewhere.
pub unsafe fn unmask_legacy_irq(irq_line: u8) -> Result<(), LegacyIrqError> {
    if !is_initialized() {
        return Err(LegacyIrqError::NotInitialized);
    }
    if irq_line >= 16 {
        return Err(LegacyIrqError::InvalidIrq);
    }
    let vector = 0x20 + irq_line;
    if !super::dispatch::is_registered(vector) {
        return Err(LegacyIrqError::HandlerNotRegistered);
    }

    // SAFETY: Caller serializes PIC state changes with IF clear.
    let pic = unsafe { &mut *core::ptr::addr_of_mut!(LEGACY_PIC) };
    pic.unmask(irq_line).map_err(|_| LegacyIrqError::PicFailure)
}

/// Mask a legacy PIC IRQ line.
///
/// # Safety
/// Call with IF clear and without concurrent PIC access.
pub unsafe fn mask_legacy_irq(irq_line: u8) -> Result<(), LegacyIrqError> {
    if !is_initialized() {
        return Err(LegacyIrqError::NotInitialized);
    }
    if irq_line >= 16 {
        return Err(LegacyIrqError::InvalidIrq);
    }
    // SAFETY: Caller serializes PIC state changes with IF clear.
    let pic = unsafe { &mut *core::ptr::addr_of_mut!(LEGACY_PIC) };
    pic.mask(irq_line).map_err(|_| LegacyIrqError::PicFailure)
}

/// Mask an IRQ and unregister its exact callback.
///
/// # Safety
/// Call with IF clear after ensuring no handler invocation is in flight.
/// The callback must be the same function pointer previously registered.
pub unsafe fn unregister_legacy_irq(
    irq_line: u8,
    handler: super::dispatch::InterruptHandler,
) -> Result<(), LegacyIrqError> {
    if !is_initialized() {
        return Err(LegacyIrqError::NotInitialized);
    }
    if irq_line >= 16 {
        return Err(LegacyIrqError::InvalidIrq);
    }

    // SAFETY: Caller serializes PIC state changes with IF clear.
    let pic = unsafe { &mut *core::ptr::addr_of_mut!(LEGACY_PIC) };
    pic.mask(irq_line).map_err(|_| LegacyIrqError::PicFailure)?;
    super::dispatch::unregister(0x20 + irq_line, handler)
        .map_err(|_| LegacyIrqError::HandlerNotRegistered)
}

/// Send the 8259 EOI from a registered IRQ callback.
///
/// # Safety
/// Must be called exactly once for each genuine, acknowledged IRQ callback,
/// from the callback running on the BSP. Spurious IRQ7/IRQ15 are filtered by
/// the common dispatcher and must not call this function.
pub unsafe fn end_legacy_irq(irq_line: u8) -> Result<(), LegacyIrqError> {
    if !is_initialized() {
        return Err(LegacyIrqError::NotInitialized);
    }
    // SAFETY: IRQ callbacks execute on the BSP and the PIC was initialized.
    let pic = unsafe { &*core::ptr::addr_of!(LEGACY_PIC) };
    pic.end_of_interrupt(irq_line).map_err(|_| LegacyIrqError::PicFailure)
}

/// Errors from the legacy PIC registration and control API.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LegacyIrqError {
    NotInitialized,
    InvalidIrq,
    HandlerAlreadyRegistered,
    HandlerNotRegistered,
    PicFailure,
}
