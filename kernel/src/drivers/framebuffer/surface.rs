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
        // SAFETY: the computed pixel range lies inside the validated mapping.
        unsafe { self.write_pixel_unchecked(offset, color); }
        Ok(())
    }

    /// Write one pixel at a previously validated byte offset.
    ///
    /// # Safety
    /// The four bytes beginning at offset must be inside the mapped framebuffer.
    #[inline]
    pub(crate) unsafe fn write_pixel_unchecked(&mut self, offset: usize, color: Color) {
        let bytes = color.memory_bytes(self.info.pixel_format);
        // SAFETY: guaranteed by the caller. Use one native 32-bit store when
        // the framebuffer address is aligned; retain byte stores for unusual
        // unaligned mappings. x86_64 is little-endian.
        unsafe {
            let destination = self.base.as_ptr().add(offset);
            if (destination as usize) & (core::mem::align_of::<u32>() - 1) == 0 {
                let pixel = u32::from_le_bytes(bytes);
                ptr::write_volatile(destination.cast::<u32>(), pixel);
            } else {
                ptr::write_volatile(destination, bytes[0]);
                ptr::write_volatile(destination.add(1), bytes[1]);
                ptr::write_volatile(destination.add(2), bytes[2]);
                ptr::write_volatile(destination.add(3), bytes[3]);
            }
        }
    }

    /// Alpha-composite a source color over the destination pixel.
    ///
    /// Source alpha uses the conventional range: 0 is transparent and 255 is
    /// opaque. The destination's reserved fourth byte is written as zero.
    pub fn blend_pixel(&mut self, x: usize, y: usize, source: Color) -> Result<(), FramebufferError> {
        if x >= self.info.width || y >= self.info.height {
            return Err(FramebufferError::OutOfBounds);
        }
        if source.alpha == 0 {
            return Ok(());
        }
        let offset = y.checked_mul(self.info.pitch)
            .and_then(|row| x.checked_mul(4).and_then(|column| row.checked_add(column)))
            .ok_or(FramebufferError::SizeOverflow)?;
        if offset.checked_add(4).ok_or(FramebufferError::SizeOverflow)? > self.mapped_len {
            return Err(FramebufferError::BufferTooSmall);
        }
        if source.alpha == 255 {
            // SAFETY: the complete destination pixel was checked above.
            unsafe { self.write_pixel_unchecked(offset, source); }
            return Ok(());
        }

        // SAFETY: the complete pixel range was checked above.
        let destination = unsafe { self.base.as_ptr().add(offset) };
        let mut bytes = [0u8; 4];
        unsafe {
            for (index, byte) in bytes.iter_mut().enumerate() {
                *byte = ptr::read_volatile(destination.add(index));
            }
        }
        let (red, green, blue) = match self.info.pixel_format {
            super::color::PixelFormat::Rgb => (bytes[0], bytes[1], bytes[2]),
            super::color::PixelFormat::Bgr => (bytes[2], bytes[1], bytes[0]),
        };
        let alpha = source.alpha as u32;
        let inverse = 255 - alpha;
        let blend = |foreground: u8, background: u8| -> u8 {
            ((foreground as u32 * alpha + background as u32 * inverse + 127) / 255) as u8
        };
        let output = Color::new(
            blend(source.red, red),
            blend(source.green, green),
            blend(source.blue, blue),
        );
        // SAFETY: offset remains within the mapping validated above.
        unsafe { self.write_pixel_unchecked(offset, output); }
        Ok(())
    }

    pub fn clear(&mut self, color: Color) {
        self.fill_rect(0, 0, self.info.width, self.info.height, color);
    }

    /// Scroll the visible framebuffer upward by whole pixel rows.
    /// The newly exposed rows are filled with the supplied background color.
    pub fn scroll_up(&mut self, rows: usize, background: Color) {
        if rows == 0 || rows >= self.info.height {
            if rows >= self.info.height && rows != 0 {
                self.clear(background);
            }
            return;
        }

        let byte_rows = rows.saturating_mul(self.info.pitch);
        let copy_len = self.info.height.saturating_sub(rows).saturating_mul(self.info.pitch);
        // SAFETY: source and destination ranges are within the validated
        // framebuffer mapping. Forward copying is safe because destination
        // addresses precede their corresponding source addresses. Use native
        // 64-bit volatile transfers only when every row and the base are
        // aligned; otherwise retain the alignment-independent byte path.
        unsafe {
            let base = self.base.as_ptr();
            if (base as usize) & (core::mem::align_of::<u64>() - 1) == 0
                && self.info.pitch % core::mem::size_of::<u64>() == 0
                && byte_rows % core::mem::size_of::<u64>() == 0
                && copy_len % core::mem::size_of::<u64>() == 0
            {
                let words = copy_len / core::mem::size_of::<u64>();
                let source = base.add(byte_rows).cast::<u64>();
                let destination = base.cast::<u64>();
                for index in 0..words {
                    let value = ptr::read_volatile(source.add(index));
                    ptr::write_volatile(destination.add(index), value);
                }
            } else {
                for offset in 0..copy_len {
                    let value = ptr::read_volatile(base.add(byte_rows + offset));
                    ptr::write_volatile(base.add(offset), value);
                }
            }
        }
        self.fill_rect(
            0,
            self.info.height - rows,
            self.info.width,
            rows,
            background,
        );
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
            let visible_end = end.min(i64::try_from(self.info.width.saturating_sub(1)).unwrap_or(i64::MAX));
            if visible_start > visible_end { return; }
            let y_direction = if y1 >= y0 { 1i128 } else { -1i128 };
            for x in visible_start..=visible_end {
                let distance = (x as i128 - x0 as i128).unsigned_abs();
                let offset = (distance * dy as u128 + dx as u128 / 2) / dx as u128;
                let y = y0 as i128 + y_direction * offset as i128;
                if y >= 0 && y < self.info.height as i128 {
                    let _ = self.put_pixel(x as usize, y as usize, color);
                }
            }
        } else {
            let (start, end) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };
            let visible_start = start.max(0);
            let visible_end = end.min(i64::try_from(self.info.height.saturating_sub(1)).unwrap_or(i64::MAX));
            if visible_start > visible_end { return; }
            let x_direction = if x1 >= x0 { 1i128 } else { -1i128 };
            for y in visible_start..=visible_end {
                let distance = (y as i128 - y0 as i128).unsigned_abs();
                let offset = (distance * dx as u128 + dy as u128 / 2) / dy as u128;
                let x = x0 as i128 + x_direction * offset as i128;
                if x >= 0 && x < self.info.width as i128 {
                    let _ = self.put_pixel(x as usize, y as usize, color);
                }
            }
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
        let left_i = i64::try_from(x).unwrap_or(i64::MAX);
        let top_i = i64::try_from(y).unwrap_or(i64::MAX);
        let right_i = i64::try_from(right).unwrap_or(i64::MAX);
        let bottom_i = i64::try_from(bottom).unwrap_or(i64::MAX);
        self.draw_line(left_i, top_i, right_i, top_i, color);
        if height > 1 {
            self.draw_line(left_i, bottom_i, right_i, bottom_i, color);
        }
        if height > 2 {
            let inner_top = i64::try_from(y.saturating_add(1)).unwrap_or(i64::MAX);
            let inner_bottom = i64::try_from(bottom.saturating_sub(1)).unwrap_or(i64::MAX);
            self.draw_line(left_i, inner_top, left_i, inner_bottom, color);
            if width > 1 {
                self.draw_line(right_i, inner_top, right_i, inner_bottom, color);
            }
        }
    }

    /// Render one 5x7 ASCII glyph. A background color fills the full cell;
    /// None leaves unset glyph pixels transparent.
    pub fn draw_char(
        &mut self,
        x: usize,
        y: usize,
        character: char,
        foreground: Color,
        background: Option<Color>,
    ) {
        use super::font::{glyph_rows, FONT_WIDTH};
        let rows = glyph_rows(character);
        for (row, bits) in rows.iter().enumerate() {
            for column in 0..FONT_WIDTH {
                let set = (bits & (1 << (FONT_WIDTH - 1 - column))) != 0;
                let color = if set { Some(foreground) } else { background };
                if let Some(pixel_color) = color {
                    let px = x.saturating_add(column);
                    let py = y.saturating_add(row);
                    if px < self.info.width && py < self.info.height {
                        // SAFETY: clipping guarantees the pixel is inside the
                        // validated framebuffer dimensions and mapping.
                        let offset = py * self.info.pitch + px * 4;
                        unsafe { self.write_pixel_unchecked(offset, pixel_color); }
                    }
                }
            }
        }
    }

    /// Render ASCII text, handling newlines, tabs, and automatic line wrapping.
    /// Returns the cursor position immediately after the final character.
    pub fn draw_text(
        &mut self,
        x: usize,
        y: usize,
        text: &str,
        foreground: Color,
        background: Option<Color>,
    ) -> (usize, usize) {
        use super::font::{FONT_ADVANCE, FONT_LINE_HEIGHT};
        let mut cursor_x = x;
        let mut cursor_y = y;
        let origin_x = x;

        for character in text.chars() {
            match character {
                '\\n' => {
                    cursor_x = origin_x;
                    cursor_y = cursor_y.saturating_add(FONT_LINE_HEIGHT);
                }
                '\\r' => cursor_x = origin_x,
                '\\t' => {
                    let relative = cursor_x.saturating_sub(origin_x);
                    let tab_width = FONT_ADVANCE * 4;
                    let next = (relative / tab_width).saturating_add(1).saturating_mul(tab_width);
                    cursor_x = origin_x.saturating_add(next);
                }
                _ => {
                    if cursor_x.saturating_add(FONT_ADVANCE) > self.info.width {
                        cursor_x = origin_x;
                        cursor_y = cursor_y.saturating_add(FONT_LINE_HEIGHT);
                    }
                    if cursor_y.saturating_add(super::font::FONT_HEIGHT) <= self.info.height {
                        self.draw_char(cursor_x, cursor_y, character, foreground, background);
                    }
                    cursor_x = cursor_x.saturating_add(FONT_ADVANCE);
                }
            }
            if cursor_y >= self.info.height { break; }
        }
        (cursor_x, cursor_y)
    }

    /// Draw a rectangle clipped to the visible display area.
    pub fn fill_rect(&mut self, x: usize, y: usize, width: usize, height: usize, color: Color) {
        let end_x = x.saturating_add(width).min(self.info.width);
        let end_y = y.saturating_add(height).min(self.info.height);
        let start_x = x.min(self.info.width);
        let start_y = y.min(self.info.height);
        if start_x == end_x || start_y == end_y {
            return;
        }

        // Clipping and FramebufferInfo::validate() guarantee every computed
        // pixel offset is inside the mapping. Keep the hot loop free of
        // per-pixel Result handling and repeated bounds checks.
        let first_column_offset = start_x * 4;
        let pixel_count = end_x - start_x;
        let row_bytes = pixel_count * 4;
        let bytes = color.memory_bytes(self.info.pixel_format);
        let packed_pair = u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3],
            bytes[0], bytes[1], bytes[2], bytes[3],
        ]);

        for row in start_y..end_y {
            let row_offset = row * self.info.pitch + first_column_offset;
            // A pair store halves the number of volatile transactions for
            // large fills. The fallback handles odd alignment and final pixels.
            let row_address = unsafe { self.base.as_ptr().add(row_offset) };
            let mut pixel = 0;
            if (row_address as usize) & (core::mem::align_of::<u64>() - 1) == 0 {
                let pairs = pixel_count / 2;
                // SAFETY: each pair covers two clipped pixels in this row.
                unsafe {
                    let destination = row_address.cast::<u64>();
                    for pair in 0..pairs {
                        ptr::write_volatile(destination.add(pair), packed_pair);
                    }
                }
                pixel = pairs * 2;
            }

            for column in pixel..pixel_count {
                // SAFETY: row and column ranges are clipped to validated
                // dimensions; pitch * height was checked at construction.
                unsafe {
                    self.write_pixel_unchecked(row_offset + column * 4, color);
                }
            }

            // row_bytes is retained in the calculation above to make the
            // clipped byte span explicit and guard future format extensions.
            let _ = row_bytes;
        }
    }
}
