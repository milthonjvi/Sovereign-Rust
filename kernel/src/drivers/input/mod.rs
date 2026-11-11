//! Keyboard, mouse, and other input-device drivers.
//!
//! This layer normalizes device data and provides a PS/2 IRQ adapter. Hardware
//! activation remains an explicit boot-time operation after IDT/PIC setup.

pub mod event;
pub mod device;
pub mod irq;
pub mod hid_report;
pub mod keyboard;
pub mod mouse;
pub mod mux;
pub mod pipeline;
pub mod ps2;
pub mod queue;
pub mod usb_hid_adapter;

pub use event::{ContactPhase, GamepadAxis, InputDeviceId, InputEvent, KeyCode, MouseButtons};
pub use irq::{MAX_IRQ_DRAIN_BUDGET, Ps2IrqError, Ps2IrqService, legacy_irqs_registered};
pub use pipeline::{InputPipelineStats, Ps2InputPipeline};
pub use queue::{EventQueue, SpscConsumer, SpscProducer, SpscQueue};

pub use mux::{InputEventMux, InputSourceId, SourcedInputEvent};

pub use usb_hid_adapter::{HidInputError, HidPublishStats, UsbBootKeyboardAdapter, UsbBootMouseAdapter};

pub use hid_report::{HidInputField, HidReportDecoder, HidReportError, HidReportMap, MAX_HID_COLLECTIONS, MAX_HID_INPUT_FIELDS, MAX_HID_VALUES};

pub use device::{InputDeviceRegistry, InputDeviceRegistryError, MAX_INPUT_DEVICES};
