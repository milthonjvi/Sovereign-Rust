//! Legacy PIC integration for the PS/2 input pipeline.
//!
//! Registers input-owned callbacks with the existing interrupt dispatcher.
//! It does not edit IDT/PIC code or unmask hardware lines; those remain the
//! responsibility of the interrupt-controller initialization layer.

use core::cell::UnsafeCell;
use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};

use crate::drivers::input::event::InputEvent;
use crate::drivers::input::pipeline::Ps2InputPipeline;
use crate::drivers::input::ps2::Ps2Controller;
use crate::drivers::input::queue::{SpscConsumer, SpscQueue};
use crate::interrupts::dispatch;

const KEYBOARD_VECTOR: u8 = 0x21;
const MOUSE_VECTOR: u8 = 0x2C;

struct IrqContextHeader {
    context: AtomicPtr<()>,
    invoke: unsafe fn(*mut (), usize),
    drain_budget: usize,
}

// SAFETY: Immutable after construction except for the atomic context pointer.
unsafe impl Sync for IrqContextHeader {}

static ACTIVE_PS2_CONTEXT: AtomicPtr<IrqContextHeader> = AtomicPtr::new(ptr::null_mut());

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ps2IrqError {
    ZeroDrainBudget,
    ContextAlreadyInstalled,
    HandlerAlreadyRegistered,
    HandlerNotRegistered,
}

/// Owns the IRQ-side pipeline. Move the returned consumer to exactly one task.
pub struct Ps2IrqService<const N: usize> {
    header: IrqContextHeader,
    controller: &'static Ps2Controller,
    pipeline: UnsafeCell<Ps2InputPipeline<'static, N>>,
}

// SAFETY: Both legacy IRQ routes must target the BSP. Interrupt gates clear IF,
// serializing IRQ1/IRQ12 there. Tasks never access the pipeline; the consumer
// endpoint accesses only the independent queue consumer side.
unsafe impl<const N: usize> Sync for Ps2IrqService<N> {}

impl<const N: usize> Ps2IrqService<N> {
    /// Creates the IRQ service and its unique task-side event consumer.
    pub fn new(
        controller: &'static Ps2Controller,
        queue: &'static mut SpscQueue<InputEvent, N>,
        wheel_enabled: bool,
        drain_budget: usize,
    ) -> Result<(Self, SpscConsumer<'static, InputEvent, N>), Ps2IrqError> {
        if drain_budget == 0 {
            return Err(Ps2IrqError::ZeroDrainBudget);
        }
        let (pipeline, consumer) = Ps2InputPipeline::new(queue, wheel_enabled);
        Ok((
            Self {
                header: IrqContextHeader {
                    context: AtomicPtr::new(ptr::null_mut()),
                    invoke: invoke_service::<N>,
                    drain_budget,
                },
                controller,
                pipeline: UnsafeCell::new(pipeline),
            },
            consumer,
        ))
    }

    /// Registers IRQ1 and IRQ12 callbacks. Does not unmask the PIC or enable
    /// the i8042 IRQ bits; do those only after this registration succeeds.
    ///
    /// # Safety
    /// The service must have a stable static address. Both IRQs must be routed
    /// to the BSP and serialized there. The controller must have exclusive
    /// ownership, and the dispatcher/IDT/central EOI path must already be live.
    pub unsafe fn register_legacy_irqs(&'static self) -> Result<(), Ps2IrqError> {
        self.header.context.store(self as *const Self as *mut (), Ordering::Relaxed);
        let header_ptr = &self.header as *const IrqContextHeader as *mut IrqContextHeader;
        ACTIVE_PS2_CONTEXT
            .compare_exchange(ptr::null_mut(), header_ptr, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Ps2IrqError::ContextAlreadyInstalled)?;

        if dispatch::register(KEYBOARD_VECTOR, ps2_irq_handler).is_err() {
            ACTIVE_PS2_CONTEXT.store(ptr::null_mut(), Ordering::Release);
            return Err(Ps2IrqError::HandlerAlreadyRegistered);
        }
        if dispatch::register(MOUSE_VECTOR, ps2_irq_handler).is_err() {
            let _ = dispatch::unregister(KEYBOARD_VECTOR, ps2_irq_handler);
            ACTIVE_PS2_CONTEXT.store(ptr::null_mut(), Ordering::Release);
            return Err(Ps2IrqError::HandlerAlreadyRegistered);
        }
        Ok(())
    }

    /// Unregisters both callbacks after the caller has quiesced the hardware.
    ///
    /// # Safety
    /// Mask IRQ1/IRQ12, disable both i8042 IRQ-enable bits, and guarantee no
    /// callback is executing before calling this method.
    pub unsafe fn unregister_legacy_irqs(&'static self) -> Result<(), Ps2IrqError> {
        dispatch::unregister(KEYBOARD_VECTOR, ps2_irq_handler)
            .map_err(|_| Ps2IrqError::HandlerNotRegistered)?;
        if dispatch::unregister(MOUSE_VECTOR, ps2_irq_handler).is_err() {
            let _ = dispatch::register(KEYBOARD_VECTOR, ps2_irq_handler);
            return Err(Ps2IrqError::HandlerNotRegistered);
        }

        let expected = &self.header as *const IrqContextHeader as *mut IrqContextHeader;
        ACTIVE_PS2_CONTEXT
            .compare_exchange(expected, ptr::null_mut(), Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Ps2IrqError::ContextAlreadyInstalled)?;
        self.header.context.store(ptr::null_mut(), Ordering::Release);
        Ok(())
    }
}

unsafe fn invoke_service<const N: usize>(context: *mut (), budget: usize) {
    // SAFETY: Registration publishes a stable static service pointer. The
    // contract guarantees BSP-only serialized IRQ delivery and quiesced removal.
    let service = unsafe { &*(context as *const Ps2IrqService<N>) };
    // SAFETY: The service is the sole owner of the IRQ-side pipeline.
    let pipeline = unsafe { &mut *service.pipeline.get() };
    let _ = pipeline.drain_controller(service.controller, budget);
}

extern "C" fn ps2_irq_handler(_vector: u8) {
    let header = ACTIVE_PS2_CONTEXT.load(Ordering::Acquire);
    if header.is_null() {
        return;
    }
    // SAFETY: Header remains valid until callbacks are quiesced and unregistered.
    let context = unsafe { (*header).context.load(Ordering::Acquire) };
    if context.is_null() {
        return;
    }
    // SAFETY: Published header contains the matching monomorphized invocation.
    unsafe { ((*header).invoke)(context, (*header).drain_budget) };
}

pub fn legacy_irqs_registered() -> bool {
    !ACTIVE_PS2_CONTEXT.load(Ordering::Acquire).is_null()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_assignments_match_legacy_pic_layout() {
        assert_eq!(KEYBOARD_VECTOR, 0x21);
        assert_eq!(MOUSE_VECTOR, 0x2C);
    }
}
