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
