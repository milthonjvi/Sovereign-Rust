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
use crate::drivers::input::ps2::{Ps2Controller, Ps2Error, Ps2MouseProtocol};
use crate::drivers::input::queue::{SpscConsumer, SpscQueue};

const KEYBOARD_VECTOR: u8 = 0x21;
const MOUSE_VECTOR: u8 = 0x2C;
/// Hard upper bound on bytes processed by one IRQ callback.
pub const MAX_IRQ_DRAIN_BUDGET: usize = 32;

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
    InvalidDrainBudget,
    InvalidQueueCapacity,
    ContextAlreadyInstalled,
    HandlerAlreadyRegistered,
    ControllerNotInitialized,
    InterruptsEnabled,
    InterruptInfrastructureUnavailable,
    ContextNotInstalled,
    NoDevicesSelected,
    DeviceUnavailable,
    Controller(Ps2Error),
}

/// Owns a permanent IRQ-side pipeline. Once registered, the service remains
/// active for the kernel lifetime. Move the returned consumer to exactly one task.
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
    /// Creates the IRQ service using the protocol actually negotiated with
    /// the mouse. This prevents a four-byte decoder from being paired with a
    /// device that still emits standard three-byte packets.
    pub fn new_with_protocol(
        controller: &'static Ps2Controller,
        queue: &'static mut SpscQueue<InputEvent, N>,
        mouse_protocol: Ps2MouseProtocol,
        drain_budget: usize,
    ) -> Result<(Self, SpscConsumer<'static, InputEvent, N>), Ps2IrqError> {
        let wheel_enabled = matches!(
            mouse_protocol,
            Ps2MouseProtocol::Wheel | Ps2MouseProtocol::FiveButton
        );
        Self::new(controller, queue, wheel_enabled, drain_budget)
    }

    /// Creates the IRQ service and its unique task-side event consumer.
    pub fn new(
        controller: &'static Ps2Controller,
        queue: &'static mut SpscQueue<InputEvent, N>,
        wheel_enabled: bool,
        drain_budget: usize,
    ) -> Result<(Self, SpscConsumer<'static, InputEvent, N>), Ps2IrqError> {
        if N < 2 {
            return Err(Ps2IrqError::InvalidQueueCapacity);
        }
        if drain_budget == 0 || drain_budget > MAX_IRQ_DRAIN_BUDGET {
            return Err(Ps2IrqError::InvalidDrainBudget);
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
    /// to the BSP and serialized there. Keep both PIC lines masked and both
    /// i8042 IRQ-enable bits clear until registration has completed. The
    /// controller must have exclusive ownership, and the dispatcher/IDT/
    /// central EOI path must already be live.
    pub unsafe fn register_legacy_irqs(&'static self) -> Result<(), Ps2IrqError> {
        if !self.controller.is_initialized() {
            return Err(Ps2IrqError::ControllerNotInitialized);
        }

        // Registration mutates the legacy PIC and must happen with IF clear.
        // Check rather than silently changing the caller's interrupt state.
        let flags: u64;
        // SAFETY: Reading RFLAGS is valid in ring 0; this API is unsafe and
        // its caller must already execute in the kernel on the BSP.
        unsafe {
            core::arch::asm!(
                "pushfq",
                "pop {flags}",
                flags = out(reg) flags,
                options(nomem, preserves_flags)
            );
        }
        if flags & (1 << 9) != 0 {
            return Err(Ps2IrqError::InterruptsEnabled);
        }

        self.header.context.store(self as *const Self as *mut (), Ordering::Relaxed);
        let header_ptr = &self.header as *const IrqContextHeader as *mut IrqContextHeader;
        if ACTIVE_PS2_CONTEXT
            .compare_exchange(ptr::null_mut(), header_ptr, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(Ps2IrqError::ContextAlreadyInstalled);
        }

        // Use the interrupt subsystem's registration API. It verifies that
        // bootstrap PIC initialization completed and masks each line before
        // publishing the callback. The caller unmasks only after the i8042
        // and device protocol are configured.
        // SAFETY: IF was verified clear above; service lifetime is static.
        if let Err(error) = unsafe {
            crate::interrupts::init::register_legacy_irq(1, ps2_irq_handler)
        } {
            ACTIVE_PS2_CONTEXT.store(ptr::null_mut(), Ordering::Release);
            return Err(match error {
                crate::interrupts::init::LegacyIrqError::HandlerAlreadyRegistered =>
                    Ps2IrqError::HandlerAlreadyRegistered,
                _ => Ps2IrqError::InterruptInfrastructureUnavailable,
            });
        }

        // SAFETY: Same BSP/IF-clear contract. Roll back IRQ1 if IRQ12 cannot
        // be registered, leaving both device lines masked.
        if let Err(error) = unsafe {
            crate::interrupts::init::register_legacy_irq(12, ps2_irq_handler)
        } {
            let _ = unsafe {
                crate::interrupts::init::unregister_legacy_irq(1, ps2_irq_handler)
            };
            ACTIVE_PS2_CONTEXT.store(ptr::null_mut(), Ordering::Release);
            return Err(match error {
                crate::interrupts::init::LegacyIrqError::HandlerAlreadyRegistered =>
                    Ps2IrqError::HandlerAlreadyRegistered,
                _ => Ps2IrqError::InterruptInfrastructureUnavailable,
            });
        }

        Ok(())
    }

    /// Enables selected PS/2 IRQ sources after both callbacks are registered
    /// and the device protocols have been configured.
    ///
    /// i8042 IRQ bits are enabled while PIC lines remain masked; PIC lines are
    /// unmasked only after controller configuration succeeds.
    ///
    /// # Safety
    /// Call on the BSP with IF clear. Device ports/protocols must be configured,
    /// and no other context may access i8042 ports.
    pub unsafe fn activate(
        &'static self,
        keyboard: bool,
        mouse: bool,
    ) -> Result<(), Ps2IrqError> {
        if !keyboard && !mouse {
            return Err(Ps2IrqError::NoDevicesSelected);
        }
        let expected = &self.header as *const IrqContextHeader as *mut IrqContextHeader;
        if ACTIVE_PS2_CONTEXT.load(Ordering::Acquire) != expected {
            return Err(Ps2IrqError::ContextNotInstalled);
        }

        let flags: u64;
        // SAFETY: The caller guarantees CPL 0; this only inspects RFLAGS.
        unsafe {
            core::arch::asm!(
                "pushfq",
                "pop {flags}",
                flags = out(reg) flags,
                options(nomem, preserves_flags)
            );
        }
        if flags & (1 << 9) != 0 {
            return Err(Ps2IrqError::InterruptsEnabled);
        }
        if (keyboard && !self.controller.keyboard_available())
            || (mouse && !self.controller.mouse_available())
        {
            return Err(Ps2IrqError::DeviceUnavailable);
        }

        if keyboard {
            // SAFETY: IF is clear and this service owns i8042 access.
            unsafe { self.controller.enable_keyboard_irq() }
                .map_err(Ps2IrqError::Controller)?;
        }
        if mouse {
            // SAFETY: Same serialized early-boot controller ownership.
            if let Err(error) = unsafe { self.controller.enable_mouse_irq() } {
                if keyboard {
                    let _ = unsafe { self.controller.disable_keyboard_irq() };
                }
                return Err(Ps2IrqError::Controller(error));
            }
        }

        if keyboard {
            // SAFETY: IRQ1 callback is registered and the i8042 source is enabled.
            if unsafe { crate::interrupts::init::unmask_legacy_irq(1) }.is_err() {
                if mouse {
                    let _ = unsafe { self.controller.disable_mouse_irq() };
                }
                let _ = unsafe { self.controller.disable_keyboard_irq() };
                return Err(Ps2IrqError::InterruptInfrastructureUnavailable);
            }
        }
        if mouse {
            // SAFETY: IRQ12 callback is registered and the i8042 source is enabled.
            if unsafe { crate::interrupts::init::unmask_legacy_irq(12) }.is_err() {
                if keyboard {
                    let _ = unsafe { crate::interrupts::init::mask_legacy_irq(1) };
                    let _ = unsafe { self.controller.disable_keyboard_irq() };
                }
                let _ = unsafe { self.controller.disable_mouse_irq() };
                return Err(Ps2IrqError::InterruptInfrastructureUnavailable);
            }
        }
        Ok(())
    }

    /// Masks PIC lines before disabling the corresponding i8042 IRQ sources.
    ///
    /// # Safety
    /// Call on the BSP with IF clear and without concurrent i8042 access.
    pub unsafe fn deactivate(
        &'static self,
        keyboard: bool,
        mouse: bool,
    ) -> Result<(), Ps2IrqError> {
        if !keyboard && !mouse {
            return Err(Ps2IrqError::NoDevicesSelected);
        }
        let expected = &self.header as *const IrqContextHeader as *mut IrqContextHeader;
        if ACTIVE_PS2_CONTEXT.load(Ordering::Acquire) != expected {
            return Err(Ps2IrqError::ContextNotInstalled);
        }

        let flags: u64;
        // SAFETY: The caller guarantees CPL 0; this only inspects RFLAGS.
        unsafe {
            core::arch::asm!(
                "pushfq",
                "pop {flags}",
                flags = out(reg) flags,
                options(nomem, preserves_flags)
            );
        }
        if flags & (1 << 9) != 0 {
            return Err(Ps2IrqError::InterruptsEnabled);
        }

        if keyboard {
            // SAFETY: IF is clear and the PIC is exclusively managed by init.
            unsafe { crate::interrupts::init::mask_legacy_irq(1) }
                .map_err(|_| Ps2IrqError::InterruptInfrastructureUnavailable)?;
        }
        if mouse {
            // SAFETY: Same serialized PIC ownership.
            unsafe { crate::interrupts::init::mask_legacy_irq(12) }
                .map_err(|_| Ps2IrqError::InterruptInfrastructureUnavailable)?;
        }
        if keyboard {
            // SAFETY: PIC line is masked before disabling the device source.
            unsafe { self.controller.disable_keyboard_irq() }
                .map_err(Ps2IrqError::Controller)?;
        }
        if mouse {
            // SAFETY: PIC line is masked before disabling the device source.
            unsafe { self.controller.disable_mouse_irq() }
                .map_err(Ps2IrqError::Controller)?;
        }
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
    // SAFETY: The registered service and header have static lifetime.
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
