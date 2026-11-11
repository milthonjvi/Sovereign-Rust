//! Drawing surface for an already mapped boot framebuffer.
use core::ptr::{self, NonNull};
use super::color::Color;
use super::info::{FramebufferError, FramebufferInfo};

pub struct FramebufferSurface<'a> {
    base: NonNull<u8>,
    info: FramebufferInfo,
    mapped_len: usize,
    _mapping: core::marker::PhantomData<&'a mut [u8]>,
}

impl<'a> FramebufferSurface<'a> {
    /// Construct a surface over a writable virtual mapping.
    ///
    /// # Safety
    /// The pointer must refer to a writable mapping of the framebuffer described
    /// by info, valid for mapped_len bytes, and exclusively borrowed for 'a.
    pub unsafe fn from_mapped(
        mapped_address: *mut u8, mapped_len: usize, info: FramebufferInfo,
    ) -> Result<Self, FramebufferError> {
        let required = info.validate()?;
        if mapped_address.is_null() { return Err(FramebufferError::NullAddress); }
        if mapped_len < required { return Err(FramebufferError::BufferTooSmall); }
        // SAFETY: non-null checked above; mapping validity is the caller's contract.
        let base = unsafe { NonNull::new_unchecked(mapped_address) };
        Ok(Self { base, info, mapped_len, _mapping: core::marker::PhantomData })
    }

    pub const fn info(&self) -> &FramebufferInfo { &self.info }
    pub const fn mapped_len(&self) -> usize { self.mapped_len }

    pub fn put_pixel(&mut self, x: usize, y: usize, color: Color) -> Result<(), FramebufferError> {
        if x >= self.info.width || y >= self.info.height {
            return Err(FramebufferError::OutOfBounds);
        }
        let offset = y.checked_mul(self.info.pitch)
            .and_then(|row| x.checked_mul(4).and_then(|column| row.checked_add(column)))
            .ok_or(FramebufferError::SizeOverflow)?;
        if offset.checked_add(4).ok_or(FramebufferError::SizeOverflow)? > self.mapped_len {
            return Err(FramebufferError::BufferTooSmall);
        }
        let bytes = color.memory_bytes(self.info.pixel_format);
        // SAFETY: the computed pixel range lies inside the validated mapping.
        unsafe {
            let destination = self.base.as_ptr().add(offset);
            for (index, byte) in bytes.iter().enumerate() {
                ptr::write_volatile(destination.add(index), *byte);
            }
        }
        Ok(())
    }

    pub fn clear(&mut self, color: Color) {
        self.fill_rect(0, 0, self.info.width, self.info.height, color);
    }

    /// Draw a line, clipping its major axis to the visible framebuffer.
    ///
    /// The major-axis iteration is bounded by the display dimensions, even when
    /// callers provide endpoints far outside the screen.
    pub fn draw_line(
        &mut self,
        x0: i64,
        y0: i64,
        x1: i64,
        y1: i64,
        color: Color,
    ) {
        let dx = (x1 as i128 - x0 as i128).abs();
        let dy = (y1 as i128 - y0 as i128).abs();
        if dx == 0 && dy == 0 {
            if x0 >= 0 && y0 >= 0 {
                let _ = self.put_pixel(x0 as usize, y0 as usize, color);
            }
            return;
        }

        if dx >= dy {
            let (start, end) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
            let visible_start = start.max(0);
            let visible_end = end.min(self.info.width.saturating_sub(1) as i64);
            if visible_start > visible_end { return; }
            let direction = if x1 >= x0 { 1i128 } else { -1i128 };
            let y_direction = if y1 >= y0 { 1i128 } else { -1i128 };
            for x in visible_start..=visible_end {
                let distance = (x as i128 - x0 as i128).abs();
                let offset = (distance * dy + dx / 2) / dx;
                let y = y0 as i128 + y_direction * offset;
                if y >= 0 && y < self.info.height as i128 {
                    let _ = self.put_pixel(x as usize, y as usize, color);
                }
            }
            let _ = direction;
        } else {
            let (start, end) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };
            let visible_start = start.max(0);
            let visible_end = end.min(self.info.height.saturating_sub(1) as i64);
            if visible_start > visible_end { return; }
            let x_direction = if x1 >= x0 { 1i128 } else { -1i128 };
            let y_direction = if y1 >= y0 { 1i128 } else { -1i128 };
            for y in visible_start..=visible_end {
                let distance = (y as i128 - y0 as i128).abs();
                let offset = (distance * dx + dy / 2) / dy;
                let x = x0 as i128 + x_direction * offset;
                if x >= 0 && x < self.info.width as i128 {
                    let _ = self.put_pixel(x as usize, y as usize, color);
                }
            }
            let _ = y_direction;
        }
    }

    /// Draw the four edges of a rectangle, clipped to the display.
    pub fn stroke_rect(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        color: Color,
    ) {
        if width == 0 || height == 0 { return; }
        let right = x.saturating_add(width - 1);
        let bottom = y.saturating_add(height - 1);
        self.draw_line(x as i64, y as i64, right as i64, y as i64, color);
        if height > 1 {
            self.draw_line(x as i64, bottom as i64, right as i64, bottom as i64, color);
        }
        if height > 2 {
            self.draw_line(x as i64, y.saturating_add(1) as i64, x as i64, bottom.saturating_sub(1) as i64, color);
            if width > 1 {
                self.draw_line(right as i64, y.saturating_add(1) as i64, right as i64, bottom.saturating_sub(1) as i64, color);
            }
        }
    }

    /// Draw a rectangle clipped to the visible display area.
    pub fn fill_rect(&mut self, x: usize, y: usize, width: usize, height: usize, color: Color) {
        let end_x = x.saturating_add(width).min(self.info.width);
        let end_y = y.saturating_add(height).min(self.info.height);
        let start_x = x.min(self.info.width);
        let start_y = y.min(self.info.height);
        for row in start_y..end_y {
            for column in start_x..end_x {
                let _ = self.put_pixel(column, row, color);
            }
        }
    }
}
