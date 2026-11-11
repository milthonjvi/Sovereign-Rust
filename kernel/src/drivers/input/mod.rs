//! Keyboard, mouse, and other input-device drivers.
//!
//! This layer normalizes device data. Hardware IRQ registration and PS/2
//! controller activation remain deferred until the interrupt/resource services
//! are ready; decoders can already be exercised independently.

pub mod event;
pub mod keyboard;
pub mod mouse;
pub mod pipeline;
pub mod ps2;
pub mod queue;

pub use event::{InputEvent, KeyCode, MouseButtons};
pub use pipeline::{InputPipelineStats, Ps2InputPipeline};
pub use queue::{EventQueue, SpscConsumer, SpscProducer, SpscQueue};
