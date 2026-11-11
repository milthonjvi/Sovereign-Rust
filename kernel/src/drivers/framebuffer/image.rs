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
        let target_info = *target.info();
        let left = (destination_x as i128).max(0);
        let top = (destination_y as i128).max(0);
        let right = (destination_x as i128 + self.width as i128)
            .min(target_info.width as i128);
        let bottom = (destination_y as i128 + self.height as i128)
            .min(target_info.height as i128);
        if left >= right || top >= bottom {
            return Ok(());
        }

        let visible_left = usize::try_from(left).map_err(|_| FramebufferError::SizeOverflow)?;
        let visible_top = usize::try_from(top).map_err(|_| FramebufferError::SizeOverflow)?;
        let visible_right = usize::try_from(right).map_err(|_| FramebufferError::SizeOverflow)?;
        let visible_bottom = usize::try_from(bottom).map_err(|_| FramebufferError::SizeOverflow)?;
        let source_left = usize::try_from(left - destination_x as i128)
            .map_err(|_| FramebufferError::SizeOverflow)?;
        let row_width = visible_right - visible_left;
        let row_bytes = row_width.checked_mul(4).ok_or(FramebufferError::SizeOverflow)?;
        let column_offset = visible_left.checked_mul(4).ok_or(FramebufferError::SizeOverflow)?;

        for target_y in visible_top..visible_bottom {
            let source_y = usize::try_from(target_y as i128 - destination_y as i128)
                .map_err(|_| FramebufferError::SizeOverflow)?;
            let source_start = source_y.checked_mul(self.stride)
                .and_then(|row| row.checked_add(source_left))
                .ok_or(FramebufferError::SizeOverflow)?;
            let source_end = source_start.checked_add(row_width)
                .ok_or(FramebufferError::SizeOverflow)?;
            if source_end > self.pixels.len() {
                return Err(FramebufferError::BufferTooSmall);
            }

            let target_offset = target_y.checked_mul(target_info.pitch)
                .and_then(|row| row.checked_add(column_offset))
                .ok_or(FramebufferError::SizeOverflow)?;
            let target_end = target_offset.checked_add(row_bytes)
                .ok_or(FramebufferError::SizeOverflow)?;
            if target_end > target.mapped_len() {
                return Err(FramebufferError::BufferTooSmall);
            }

            // SAFETY: both source and destination spans were checked for this
            // complete clipped row; the surface writer handles RGB/BGR packing.
            unsafe {
                target.write_pixels_unchecked(
                    target_offset,
                    &self.pixels[source_start..source_end],
                );
            }
        }
        Ok(())
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

        let visible_width = visible_right - visible_left;
        let row_bytes = visible_width.checked_mul(4)
            .ok_or(FramebufferError::SizeOverflow)?;
        let first_column_bytes = visible_left.checked_mul(4)
            .ok_or(FramebufferError::SizeOverflow)?;
        for target_y in visible_top..visible_bottom {
            let source_y = (target_y as i128 - destination_y as i128) as usize;
            let source_row = source_y * self.stride;
            let source_start_x = (visible_left as i128 - destination_x as i128) as usize;
            let source_end_x = source_start_x + visible_width;
            let mut target_offset = target_y.checked_mul(target_info.pitch)
                .and_then(|row| row.checked_add(first_column_bytes))
                .ok_or(FramebufferError::SizeOverflow)?;
            let target_end = target_offset.checked_add(row_bytes)
                .ok_or(FramebufferError::SizeOverflow)?;
            if target_end > target.mapped_len() {
                return Err(FramebufferError::BufferTooSmall);
            }
            if opacity == 255 {
                // Preserve source alpha without a multiply in the common
                // full-opacity case. Fully opaque runs can use the surface's
                // paired-store writer instead of one volatile pixel store per
                // pixel when the destination is suitably aligned.
                let mut source_x = source_start_x;
                while source_x < source_end_x {
                    let color = self.pixels[source_row + source_x];
                    if color.alpha == 0 {
                        source_x += 1;
                        target_offset += 4;
                        continue;
                    }
                    if color.alpha == 255 {
                        let run_start = source_x;
                        while source_x < source_end_x
                            && self.pixels[source_row + source_x].alpha == 255
                        {
                            source_x += 1;
                        }
                        let run_len = source_x - run_start;
                        // SAFETY: the run is within the already validated
                        // source row and target byte span.
                        unsafe {
                            target.write_pixels_unchecked(
                                target_offset,
                                &self.pixels[source_row + run_start..source_row + source_x],
                            );
                        }
                        target_offset = target_offset.checked_add(run_len * 4)
                            .ok_or(FramebufferError::SizeOverflow)?;
                    } else {
                        // SAFETY: this pixel lies within the validated row.
                        unsafe { target.blend_pixel_unchecked(target_offset, color); }
                        source_x += 1;
                        target_offset += 4;
                    }
                }
            } else {
                for source_x in source_start_x..source_end_x {
                    let mut color = self.pixels[source_row + source_x];
                    color.alpha = ((color.alpha as u16 * opacity as u16 + 127) / 255) as u8;
                    if color.alpha != 0 {
                        // SAFETY: the clipped row range and its complete byte
                        // span were validated against the target mapping above.
                        unsafe { target.blend_pixel_unchecked(target_offset, color); }
                    }
                    target_offset = target_offset.checked_add(4)
                        .ok_or(FramebufferError::SizeOverflow)?;
                }
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

            // Seed the exact nearest-neighbor mapping at the first visible
            // destination pixel, then advance it with quotient/remainder steps.
            // This removes a 128-bit division from every pixel in the hot loop.
            let relative_x = (visible_left as i128 - destination_x as i128) as u128;
            let numerator = relative_x * self.width as u128;
            let destination_width_u128 = destination_width as u128;
            let mut source_x = (numerator / destination_width_u128) as usize;
            let mut x_remainder = numerator % destination_width_u128;
            let source_step = (self.width as u128 / destination_width_u128) as usize;
            let source_step_remainder = self.width as u128 % destination_width_u128;
            let row_offset = target_y.checked_mul(target_info.pitch)
                .and_then(|row| visible_left.checked_mul(4).and_then(|column| row.checked_add(column)))
                .ok_or(FramebufferError::SizeOverflow)?;
            for column in 0..(visible_right - visible_left) {
                let color = self.pixels[source_row + source_x];
                let offset = row_offset + column * 4;
                // SAFETY: destination coordinates were clipped to the validated
                // target dimensions, and the complete mapped extent was checked
                // when FramebufferSurface was constructed.
                unsafe { target.write_pixel_unchecked(offset, color); }

                if column + 1 < visible_right - visible_left {
                    source_x += source_step;
                    x_remainder += source_step_remainder;
                    if x_remainder >= destination_width_u128 {
                        x_remainder -= destination_width_u128;
                        source_x += 1;
                    }
                }
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
    fn scaled_blit_uses_nearest_neighbor_mapping() {
        use super::super::color::PixelFormat;
        use super::super::info::FramebufferInfo;
        use super::super::surface::FramebufferSurface;

        let source = [Color::RED, Color::BLUE];
        let image = ImageView::new(&source, 2, 1, 2).unwrap();
        let mut memory = [0u8; 8 * 4 * 4];
        let info = FramebufferInfo {
            address: 1,
            width: 8,
            height: 4,
            pitch: 8 * 4,
            bits_per_pixel: 32,
            pixel_format: PixelFormat::Rgb,
        };
        // SAFETY: the backing array is writable, correctly sized and exclusively borrowed.
        let mut target = unsafe {
            FramebufferSurface::from_mapped(memory.as_mut_ptr(), memory.len(), info).unwrap()
        };
        image.draw_scaled(&mut target, 1, 1, 4, 2).unwrap();
        drop(target);

        let pixel = |x: usize, y: usize| {
            let offset = (y * 8 + x) * 4;
            [memory[offset], memory[offset + 1], memory[offset + 2], memory[offset + 3]]
        };
        assert_eq!(pixel(1, 1), [255, 0, 0, 0]);
        assert_eq!(pixel(2, 1), [255, 0, 0, 0]);
        assert_eq!(pixel(3, 1), [0, 0, 255, 0]);
        assert_eq!(pixel(4, 1), [0, 0, 255, 0]);
        assert_eq!(pixel(4, 2), [0, 0, 255, 0]);
    }

    #[test]
    fn alpha_blit_handles_transparent_partial_and_opaque_pixels() {
        use super::super::color::PixelFormat;
        use super::super::info::FramebufferInfo;
        use super::super::surface::FramebufferSurface;

        let source = [
            Color::with_alpha(255, 0, 0, 128),
            Color::with_alpha(255, 255, 255, 0),
            Color::with_alpha(0, 255, 0, 255),
        ];
        let image = ImageView::new(&source, 3, 1, 3).unwrap();
        let mut memory = [0u8; 3 * 4];
        let info = FramebufferInfo {
            address: 1,
            width: 3,
            height: 1,
            pitch: 3 * 4,
            bits_per_pixel: 32,
            pixel_format: PixelFormat::Rgb,
        };
        // SAFETY: the backing array is writable, correctly sized, and exclusively borrowed.
        let mut target = unsafe {
            FramebufferSurface::from_mapped(memory.as_mut_ptr(), memory.len(), info).unwrap()
        };
        target.clear(Color::new(0, 0, 255));
        image.draw_alpha_blended(&mut target, 0, 0).unwrap();
        drop(target);

        assert_eq!(&memory[0..4], &[128, 0, 127, 0]);
        assert_eq!(&memory[4..8], &[0, 0, 255, 0]);
        assert_eq!(&memory[8..12], &[0, 255, 0, 0]);
    }

    #[test]
    fn opaque_alpha_runs_use_format_aware_span_writes() {
        use super::super::color::PixelFormat;
        use super::super::info::FramebufferInfo;
        use super::super::surface::FramebufferSurface;

        let source = [
            Color::opaque(10, 20, 30),
            Color::opaque(40, 50, 60),
            Color::with_alpha(0, 255, 0, 0),
            Color::opaque(70, 80, 90),
        ];
        let image = ImageView::new(&source, 4, 1, 4).unwrap();
        let mut memory = [0u8; 4 * 4];
        let info = FramebufferInfo {
            address: 1,
            width: 4,
            height: 1,
            pitch: 4 * 4,
            bits_per_pixel: 32,
            pixel_format: PixelFormat::Bgr,
        };
        // SAFETY: the backing array is writable, correctly sized and exclusively borrowed.
        let mut target = unsafe {
            FramebufferSurface::from_mapped(memory.as_mut_ptr(), memory.len(), info).unwrap()
        };
        target.clear(Color::new(1, 2, 3));
        image.draw_alpha_blended(&mut target, 0, 0).unwrap();
        drop(target);

        assert_eq!(&memory[0..4], &[30, 20, 10, 0]);
        assert_eq!(&memory[4..8], &[60, 50, 40, 0]);
        assert_eq!(&memory[8..12], &[3, 2, 1, 0]);
        assert_eq!(&memory[12..16], &[90, 80, 70, 0]);
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
