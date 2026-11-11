//! Boot framebuffer output and software drawing primitives.
pub mod buffer;
pub mod color;
pub mod compositor;
pub mod console;
pub mod dirty;
pub mod font;
pub mod fmt;
pub mod image;
pub mod info;
pub mod raster;
pub mod surface;
pub use buffer::{BufferError, DoubleBuffer, SoftwareBuffer};
pub use color::{Color, PixelFormat};
pub use compositor::{Compositor, CompositorError, LayerId};
pub use console::FramebufferConsole;
pub use info::{FramebufferError, FramebufferInfo};
pub use image::{ImageView, ImageViewError};
pub use surface::FramebufferSurface;

pub use raster::{fill_triangle, stroke_triangle};
