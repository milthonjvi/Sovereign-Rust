//! Boot framebuffer output and software drawing primitives.
pub mod color;
pub mod console;
pub mod font;
pub mod fmt;
pub mod info;
pub mod surface;
pub use color::{Color, PixelFormat};
pub use console::FramebufferConsole;
pub use info::{FramebufferError, FramebufferInfo};
pub use surface::FramebufferSurface;
