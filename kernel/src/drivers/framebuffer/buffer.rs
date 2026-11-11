//! Caller-backed off-screen surfaces and synchronized software double buffering.
//!
//! The kernel supplies both pixel slices. This module never allocates memory,
//! maps physical pages, or assumes ownership of a memory allocator.

use super::color::Color;
use super::dirty::Rect;
use super::info::FramebufferError;
use super::surface::FramebufferSurface;

/// Errors produced while validating off-screen pixel storage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BufferError {
    EmptyDimensions,
    StrideTooSmall,
    SizeOverflow,
    StorageTooSmall,
    DimensionsMismatch,
    Framebuffer(FramebufferError),
}

impl From<FramebufferError> for BufferError {
    fn from(error: FramebufferError) -> Self {
        Self::Framebuffer(error)
    }
}

/// Mutable off-screen pixel storage borrowed from its owner.
pub struct SoftwareBuffer<'a> {
    pixels: &'a mut [Color],
    width: usize,
    height: usize,
    stride: usize,
}

impl<'a> SoftwareBuffer<'a> {
    /// Validate and wrap caller-provided pixel storage.
    pub fn new(
        pixels: &'a mut [Color],
        width: usize,
        height: usize,
        stride: usize,
    ) -> Result<Self, BufferError> {
        if width == 0 || height == 0 {
            return Err(BufferError::EmptyDimensions);
        }
        if stride < width {
            return Err(BufferError::StrideTooSmall);
        }
        let required = (height - 1)
            .checked_mul(stride)
            .and_then(|offset| offset.checked_add(width))
            .ok_or(BufferError::SizeOverflow)?;
        if pixels.len() < required {
            return Err(BufferError::StorageTooSmall);
        }
        Ok(Self { pixels, width, height, stride })
    }

    pub const fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    pub const fn stride(&self) -> usize {
        self.stride
    }

    pub fn clear(&mut self, color: Color) {
        for y in 0..self.height {
            let row = y * self.stride;
            self.pixels[row..row + self.width].fill(color);
        }
    }

    pub fn put_pixel(&mut self, x: usize, y: usize, color: Color) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        self.pixels[y * self.stride + x] = color;
        true
    }

    /// Fill a clipped rectangle. Saturating endpoints prevent coordinate wrap.
    pub fn fill_rect(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        color: Color,
    ) {
        let left = x.min(self.width);
        let top = y.min(self.height);
        let right = x.saturating_add(width).min(self.width);
        let bottom = y.saturating_add(height).min(self.height);
        if left >= right || top >= bottom {
            return;
        }
        for row in top..bottom {
            let start = row * self.stride + left;
            self.pixels[start..start + (right - left)].fill(color);
        }
    }

    /// Alpha-composite a color over an off-screen pixel.
    pub fn blend_pixel(&mut self, x: usize, y: usize, source: Color) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        if source.alpha == 0 {
            return true;
        }
        let index = y * self.stride + x;
        if source.alpha == 255 {
            self.pixels[index] = Color::new(source.red, source.green, source.blue);
            return true;
        }
        let destination = self.pixels[index];
        let alpha = source.alpha as u32;
        let inverse = 255 - alpha;
        let blend = |foreground: u8, background: u8| -> u8 {
            ((foreground as u32 * alpha + background as u32 * inverse + 127) / 255) as u8
        };
        self.pixels[index] = Color::new(
            blend(source.red, destination.red),
            blend(source.green, destination.green),
            blend(source.blue, destination.blue),
        );
        true
    }

    /// Present a clipped region to the mapped hardware framebuffer.
    pub fn present_region_to(
        &self,
        target: &mut FramebufferSurface<'_>,
        region: Rect,
    ) -> Result<(), BufferError> {
        let info = *target.info();
        if info.width != self.width || info.height != self.height {
            return Err(BufferError::DimensionsMismatch);
        }
        let left = region.x.min(self.width);
        let top = region.y.min(self.height);
        let right = region.x.saturating_add(region.width).min(self.width);
        let bottom = region.y.saturating_add(region.height).min(self.height);
        if left >= right || top >= bottom {
            return Ok(());
        }

        for y in top..bottom {
            let source_row = y * self.stride;
            let target_row = y * info.pitch;
            for x in left..right {
                let offset = target_row
                    .checked_add(x.checked_mul(4).ok_or(BufferError::SizeOverflow)?)
                    .ok_or(BufferError::SizeOverflow)?;
                // SAFETY: x/y are clipped to the framebuffer dimensions;
                // FramebufferSurface validated the complete mapped extent.
                unsafe {
                    target.write_pixel_unchecked(offset, self.pixels[source_row + x]);
                }
            }
        }
        Ok(())
    }

    /// Present every visible pixel to the mapped hardware framebuffer.
    pub fn present_to(
        &self,
        target: &mut FramebufferSurface<'_>,
    ) -> Result<(), BufferError> {
        self.present_region_to(target, Rect {
            x: 0,
            y: 0,
            width: self.width,
            height: self.height,
        })
    }

    fn copy_regions_from(
        &mut self,
        source: &SoftwareBuffer<'_>,
        regions: &[Rect],
    ) {
        debug_assert_eq!(self.width, source.width);
        debug_assert_eq!(self.height, source.height);
        for region in regions {
            let left = region.x.min(self.width);
            let top = region.y.min(self.height);
            let right = region.x.saturating_add(region.width).min(self.width);
            let bottom = region.y.saturating_add(region.height).min(self.height);
            if left >= right || top >= bottom {
                continue;
            }
            let row_width = right - left;
            for y in top..bottom {
                let destination_start = y * self.stride + left;
                let source_start = y * source.stride + left;
                self.pixels[destination_start..destination_start + row_width]
                    .copy_from_slice(&source.pixels[source_start..source_start + row_width]);
            }
        }
    }
}

/// Two caller-owned off-screen surfaces with explicit present-and-swap semantics.
///
/// After each successful swap, both buffers contain the newly presented frame.
/// This preserves the previous frame as the next back-buffer baseline, allowing
/// subsequent partial updates without exposing stale pixels.
pub struct DoubleBuffer<'a> {
    front: SoftwareBuffer<'a>,
    back: SoftwareBuffer<'a>,
}

impl<'a> DoubleBuffer<'a> {
    pub fn new(
        first: &'a mut [Color],
        second: &'a mut [Color],
        width: usize,
        height: usize,
        stride: usize,
    ) -> Result<Self, BufferError> {
        let front = SoftwareBuffer::new(first, width, height, stride)?;
        let back = SoftwareBuffer::new(second, width, height, stride)?;
        Ok(Self { front, back })
    }

    pub const fn dimensions(&self) -> (usize, usize) {
        self.front.dimensions()
    }

    pub fn front(&self) -> &SoftwareBuffer<'a> {
        &self.front
    }

    pub fn back_mut(&mut self) -> &mut SoftwareBuffer<'a> {
        &mut self.back
    }

    /// Present selected regions, swap roles, then synchronize those regions.
    ///
    /// The caller must ensure every modified pixel in the back buffer is covered
    /// by one of `regions`. Changes outside them are intentionally not presented
    /// or synchronized. Use `present_and_swap` after a full-frame update.
    pub fn present_regions_and_swap(
        &mut self,
        target: &mut FramebufferSurface<'_>,
        regions: &[Rect],
    ) -> Result<(), BufferError> {
        for region in regions {
            self.back.present_region_to(target, *region)?;
        }
        core::mem::swap(&mut self.front, &mut self.back);
        let (front, back) = (&self.front, &mut self.back);
        back.copy_regions_from(front, regions);
        Ok(())
    }

    /// Present the complete frame, swap roles, and synchronize the next back buffer.
    pub fn present_and_swap(
        &mut self,
        target: &mut FramebufferSurface<'_>,
    ) -> Result<(), BufferError> {
        let (width, height) = self.dimensions();
        self.present_regions_and_swap(
            target,
            &[Rect { x: 0, y: 0, width, height }],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{BufferError, DoubleBuffer, SoftwareBuffer};
    use super::super::dirty::Rect;
    use crate::drivers::framebuffer::color::Color;

    #[test]
    fn validates_storage_and_stride() {
        let mut pixels = [Color::BLACK; 8];
        assert!(matches!(
            SoftwareBuffer::new(&mut pixels, 3, 2, 2),
            Err(BufferError::StrideTooSmall)
        ));
        assert!(matches!(
            SoftwareBuffer::new(&mut pixels, 3, 3, 3),
            Err(BufferError::StorageTooSmall)
        ));
    }

    #[test]
    fn fill_respects_row_stride() {
        let mut pixels = [Color::BLACK; 8];
        let mut buffer = SoftwareBuffer::new(&mut pixels, 2, 2, 4).unwrap();
        buffer.fill_rect(0, 0, 2, 2, Color::RED);
        assert_eq!(buffer.pixels[0], Color::RED);
        assert_eq!(buffer.pixels[1], Color::RED);
        assert_eq!(buffer.pixels[2], Color::BLACK);
        assert_eq!(buffer.pixels[4], Color::RED);
        assert_eq!(buffer.pixels[5], Color::RED);
    }

    #[test]
    fn partial_sync_copies_only_requested_region() {
        let mut first = [Color::BLACK; 8];
        let mut second = [Color::BLACK; 8];
        {
            let mut buffers = DoubleBuffer::new(&mut first, &mut second, 4, 2, 4).unwrap();
            buffers.back_mut().put_pixel(0, 0, Color::RED);
            buffers.back_mut().put_pixel(3, 1, Color::BLUE);
            let (front, back) = (&buffers.back, &mut buffers.front);
            back.copy_regions_from(front, &[Rect { x: 0, y: 0, width: 1, height: 1 }]);
            assert_eq!(buffers.front.pixels[0], Color::RED);
            assert_eq!(buffers.front.pixels[7], Color::BLACK);
        }
    }

    #[test]
    fn double_buffer_keeps_dimensions() {
        let mut first = [Color::BLACK; 16];
        let mut second = [Color::BLACK; 16];
        let buffers = DoubleBuffer::new(&mut first, &mut second, 4, 4, 4).unwrap();
        assert_eq!(buffers.dimensions(), (4, 4));
    }
}
