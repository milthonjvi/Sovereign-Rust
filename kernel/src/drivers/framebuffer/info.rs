//! Validated metadata supplied by the boot protocol.
use super::color::PixelFormat;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FramebufferInfo {
    /// Physical address; map it before dereferencing.
    pub address: u64,
    pub width: usize,
    pub height: usize,
    pub pitch: usize,
    pub bits_per_pixel: u8,
    pub pixel_format: PixelFormat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FramebufferError {
    NullAddress, EmptyDimensions, UnsupportedPixelFormat, UnsupportedBitsPerPixel,
    PitchTooSmall, SizeOverflow, BufferTooSmall, OutOfBounds,
}

impl FramebufferInfo {
    /// Validate metadata and return the required mapping length.
    pub fn validate(&self) -> Result<usize, FramebufferError> {
        if self.address == 0 { return Err(FramebufferError::NullAddress); }
        if self.width == 0 || self.height == 0 { return Err(FramebufferError::EmptyDimensions); }
        if self.bits_per_pixel != 32 { return Err(FramebufferError::UnsupportedBitsPerPixel); }
        let row_bytes = self.width.checked_mul(4).ok_or(FramebufferError::SizeOverflow)?;
        if self.pitch < row_bytes { return Err(FramebufferError::PitchTooSmall); }
        self.pitch.checked_mul(self.height).ok_or(FramebufferError::SizeOverflow)
    }
}
