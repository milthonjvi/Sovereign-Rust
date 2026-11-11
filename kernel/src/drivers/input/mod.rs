//! Keyboard, mouse, and other input-device drivers.
//!
//! This layer normalizes device data and provides a PS/2 IRQ adapter. Hardware
//! activation remains an explicit boot-time operation after IDT/PIC setup.

pub mod event;
pub mod irq;
pub mod keyboard;
pub mod mouse;
pub mod pipeline;
pub mod ps2;
pub mod queue;

pub use event::{ContactPhase, GamepadAxis, InputDeviceId, InputEvent, KeyCode, MouseButtons};
pub use irq::{MAX_IRQ_DRAIN_BUDGET, Ps2IrqError, Ps2IrqService, legacy_irqs_registered};
pub use pipeline::{InputPipelineStats, Ps2InputPipeline};
pub use queue::{EventQueue, SpscConsumer, SpscProducer, SpscQueue};
