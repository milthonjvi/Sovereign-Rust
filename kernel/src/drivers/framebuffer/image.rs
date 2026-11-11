//! Borrowed, allocation-free image views and framebuffer blitting.
//!
//! Pixel data is supplied by the caller as logical RGBA colors. The target
//! surface performs the required RGB/BGR channel conversion.

use super::color::Color;
use super::info::FramebufferError;
use super::surface::FramebufferSurface;

/// A validated, non-owning view over a row-strided color buffer.
#[derive(Clone, Copy)]
pub struct ImageView<'a> {
    pixels: &'a [Color],
    width: usize,
    height: usize,
    stride: usize,
}

impl<'a> ImageView<'a> {
    /// Construct an image view.
    ///
    /// The backing slice must contain every pixel in the final row, including
    /// row padding only when it is represented by the supplied stride.
    pub fn new(
        pixels: &'a [Color],
        width: usize,
        height: usize,
        stride: usize,
    ) -> Result<Self, ImageViewError> {
        if width == 0 || height == 0 {
            return Err(ImageViewError::EmptyDimensions);
        }
        if stride < width {
            return Err(ImageViewError::StrideTooSmall);
        }
        let required = (height - 1)
            .checked_mul(stride)
            .and_then(|offset| offset.checked_add(width))
            .ok_or(ImageViewError::SizeOverflow)?;
        if pixels.len() < required {
            return Err(ImageViewError::BufferTooSmall);
        }
        Ok(Self { pixels, width, height, stride })
    }

    pub const fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    pub const fn stride(&self) -> usize {
        self.stride
    }

    /// Blit at native resolution. Negative destination coordinates are clipped.
    pub fn draw(
        &self,
        target: &mut FramebufferSurface<'_>,
        destination_x: i64,
        destination_y: i64,
    ) -> Result<(), FramebufferError> {
        self.draw_scaled(
            target,
            destination_x,
            destination_y,
            self.width,
            self.height,
        )
    }

    /// Alpha-composite this image at native resolution.
    pub fn draw_alpha_blended(
        &self,
        target: &mut FramebufferSurface<'_>,
        destination_x: i64,
        destination_y: i64,
    ) -> Result<(), FramebufferError> {
        let info = *target.info();
        self.draw_alpha_blended_clipped(
            target,
            destination_x,
            destination_y,
            0,
            0,
            info.width,
            info.height,
            255,
        )
    }

    /// Alpha-composite only the intersection with a target-space clip rectangle.
    /// Global opacity is multiplied by each source pixel's alpha.
    pub(crate) fn draw_alpha_blended_clipped(
        &self,
        target: &mut FramebufferSurface<'_>,
        destination_x: i64,
        destination_y: i64,
        clip_x: usize,
        clip_y: usize,
        clip_width: usize,
        clip_height: usize,
        opacity: u8,
    ) -> Result<(), FramebufferError> {
        if opacity == 0 || clip_width == 0 || clip_height == 0 {
            return Ok(());
        }
        let target_info = *target.info();
        let left = (destination_x as i128).max(clip_x as i128).max(0);
        let top = (destination_y as i128).max(clip_y as i128).max(0);
        let right = (destination_x as i128 + self.width as i128)
            .min((clip_x as i128 + clip_width as i128).min(target_info.width as i128));
        let bottom = (destination_y as i128 + self.height as i128)
            .min((clip_y as i128 + clip_height as i128).min(target_info.height as i128));
        if left >= right || top >= bottom {
            return Ok(());
        }

        let visible_left = usize::try_from(left).map_err(|_| FramebufferError::SizeOverflow)?;
        let visible_top = usize::try_from(top).map_err(|_| FramebufferError::SizeOverflow)?;
        let visible_right = usize::try_from(right).map_err(|_| FramebufferError::SizeOverflow)?;
        let visible_bottom = usize::try_from(bottom).map_err(|_| FramebufferError::SizeOverflow)?;

        for target_y in visible_top..visible_bottom {
            let source_y = (target_y as i128 - destination_y as i128) as usize;
            let source_row = source_y * self.stride;
            for target_x in visible_left..visible_right {
                let source_x = (target_x as i128 - destination_x as i128) as usize;
                let mut color = self.pixels[source_row + source_x];
                color.alpha = ((color.alpha as u16 * opacity as u16 + 127) / 255) as u8;
                target.blend_pixel(target_x, target_y, color)?;
            }
        }
        Ok(())
    }

    /// Blit with nearest-neighbor scaling and clipping.
    ///
    /// This is an opaque copy; alpha compositing is intentionally separate so
    /// callers do not pay blending costs for ordinary framebuffer transfers.
    pub fn draw_scaled(
        &self,
        target: &mut FramebufferSurface<'_>,
        destination_x: i64,
        destination_y: i64,
        destination_width: usize,
        destination_height: usize,
    ) -> Result<(), FramebufferError> {
        if destination_width == 0 || destination_height == 0 {
            return Ok(());
        }

        let target_info = *target.info();
        let left = (destination_x as i128).max(0);
        let top = (destination_y as i128).max(0);
        let right = (destination_x as i128 + destination_width as i128)
            .min(target_info.width as i128);
        let bottom = (destination_y as i128 + destination_height as i128)
            .min(target_info.height as i128);
        if left >= right || top >= bottom {
            return Ok(());
        }

        let visible_left = usize::try_from(left).map_err(|_| FramebufferError::SizeOverflow)?;
        let visible_top = usize::try_from(top).map_err(|_| FramebufferError::SizeOverflow)?;
        let visible_right = usize::try_from(right).map_err(|_| FramebufferError::SizeOverflow)?;
        let visible_bottom = usize::try_from(bottom).map_err(|_| FramebufferError::SizeOverflow)?;

        for target_y in visible_top..visible_bottom {
            let relative_y = (target_y as i128 - destination_y as i128) as u128;
            let source_y = ((relative_y * self.height as u128)
                / destination_height as u128) as usize;
            let source_row = source_y * self.stride;

            for target_x in visible_left..visible_right {
                let relative_x = (target_x as i128 - destination_x as i128) as u128;
                let source_x = ((relative_x * self.width as u128)
                    / destination_width as u128) as usize;
                let color = self.pixels[source_row + source_x];
                let offset = target_y.checked_mul(target_info.pitch)
                    .and_then(|row| target_x.checked_mul(4).and_then(|column| row.checked_add(column)))
                    .ok_or(FramebufferError::SizeOverflow)?;
                // SAFETY: destination coordinates were clipped to the validated
                // target dimensions, and FramebufferInfo::validate checked pitch
                // and total mapping-size arithmetic at surface construction.
                unsafe { target.write_pixel_unchecked(offset, color); }
            }
        }

        Ok(())
    }
}

/// Errors detected while validating borrowed image storage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageViewError {
    EmptyDimensions,
    StrideTooSmall,
    SizeOverflow,
    BufferTooSmall,
}

#[cfg(test)]
mod tests {
    use super::{ImageView, ImageViewError};
    use super::super::color::Color;

    #[test]
    fn rejects_empty_images() {
        assert_eq!(
            ImageView::new(&[], 0, 1, 0).err(),
            Some(ImageViewError::EmptyDimensions)
        );
    }

    #[test]
    fn rejects_short_stride() {
        let pixels = [Color::BLACK; 4];
        assert_eq!(
            ImageView::new(&pixels, 2, 2, 1).err(),
            Some(ImageViewError::StrideTooSmall)
        );
    }

    #[test]
    fn validates_last_row_extent() {
        let pixels = [Color::BLACK; 5];
        assert_eq!(
            ImageView::new(&pixels, 2, 3, 2).err(),
            Some(ImageViewError::BufferTooSmall)
        );
        assert!(ImageView::new(&pixels, 2, 2, 3).is_ok());
    }
}
