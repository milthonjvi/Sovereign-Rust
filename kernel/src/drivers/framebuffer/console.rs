//! Text console layered over the boot framebuffer.
//!
//! This is an early single-writer console. It deliberately does not depend on
//! interrupts, allocation, terminal escape sequences, or an input driver.

use super::color::Color;
use super::font::{FONT_ADVANCE, FONT_HEIGHT, FONT_LINE_HEIGHT};
use super::surface::FramebufferSurface;

/// Fixed-cell console for early kernel diagnostics.
pub struct FramebufferConsole<'a> {
    surface: FramebufferSurface<'a>,
    foreground: Color,
    background: Color,
    columns: usize,
    rows: usize,
    column: usize,
    row: usize,
}

impl<'a> FramebufferConsole<'a> {
    /// Create and clear a console. Returns None if the display is too small
    /// to contain at least one complete text cell.
    pub fn new(
        mut surface: FramebufferSurface<'a>,
        foreground: Color,
        background: Color,
    ) -> Option<Self> {
        let info = *surface.info();
        let columns = info.width / FONT_ADVANCE;
        let rows = info.height / FONT_LINE_HEIGHT;
        if columns == 0 || rows == 0 {
            return None;
        }
        surface.clear(background);
        Some(Self {
            surface,
            foreground,
            background,
            columns,
            rows,
            column: 0,
            row: 0,
        })
    }

    /// Current cursor position in character cells (column, row).
    pub const fn cursor_position(&self) -> (usize, usize) {
        (self.column, self.row)
    }

    /// Console dimensions in character cells (columns, rows).
    pub const fn dimensions(&self) -> (usize, usize) {
        (self.columns, self.rows)
    }

    /// Write a UTF-8 string. Unsupported non-ASCII characters use the font's
    /// replacement glyph.
    pub fn write_str(&mut self, text: &str) {
        for character in text.chars() {
            self.write_char(character);
        }
    }

    /// Write text followed by a newline.
    pub fn write_line(&mut self, text: &str) {
        self.write_str(text);
        self.newline();
    }

    /// Clear the display and reset the cursor.
    pub fn clear(&mut self) {
        self.surface.clear(self.background);
        self.column = 0;
        self.row = 0;
    }

    /// Write one character or process a basic terminal control character.
    pub fn write_char(&mut self, character: char) {
        match character {
            '\n' => self.newline(),
            '\r' => self.column = 0,
            '\x08' => self.backspace(),
            '\t' => {
                let next = (self.column / 4).saturating_add(1).saturating_mul(4);
                let spaces = next.saturating_sub(self.column);
                for _ in 0..spaces {
                    self.write_printable(' ');
                }
            }
            '\x20'..='\x7e' => self.write_printable(character),
            c if !c.is_control() => self.write_printable('?'),
            _ => {}
        }
    }

    fn write_printable(&mut self, character: char) {
        if self.column >= self.columns {
            self.newline();
        }
        if self.row >= self.rows {
            self.scroll();
        }

        let x = self.column * FONT_ADVANCE;
        let y = self.row * FONT_LINE_HEIGHT;
        self.surface.fill_rect(
            x,
            y,
            FONT_ADVANCE,
            FONT_LINE_HEIGHT,
            self.background,
        );
        // The complete cell was cleared immediately above. Rendering the
        // glyph transparently avoids painting its background pixels twice.
        self.surface.draw_char(
            x,
            y,
            character,
            self.foreground,
            None,
        );

        self.column += 1;
        if self.column >= self.columns {
            self.newline();
        }
    }

    fn newline(&mut self) {
        self.column = 0;
        self.row = self.row.saturating_add(1);
        if self.row >= self.rows {
            self.scroll();
        }
    }

    fn scroll(&mut self) {
        self.surface.scroll_up(FONT_LINE_HEIGHT, self.background);
        self.row = self.rows - 1;
    }

    fn backspace(&mut self) {
        if self.column == 0 {
            if self.row == 0 {
                return;
            }
            self.row -= 1;
            self.column = self.columns - 1;
        } else {
            self.column -= 1;
        }

        let x = self.column * FONT_ADVANCE;
        let y = self.row * FONT_LINE_HEIGHT;
        self.surface.fill_rect(
            x,
            y,
            FONT_ADVANCE,
            FONT_LINE_HEIGHT.max(FONT_HEIGHT),
            self.background,
        );
    }
}
