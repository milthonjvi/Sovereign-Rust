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
    cursor_visible: bool,
    cursor_color: Color,
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
            cursor_visible: false,
            cursor_color: foreground,
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

    /// Whether the insertion cursor is currently rendered.
    pub const fn cursor_visible(&self) -> bool {
        self.cursor_visible
    }

    /// Set the insertion cursor visibility.
    pub fn set_cursor_visible(&mut self, visible: bool) {
        if self.cursor_visible == visible {
            return;
        }
        if visible {
            self.cursor_visible = true;
            self.draw_cursor();
        } else {
            self.erase_cursor();
            self.cursor_visible = false;
        }
    }

    /// Change the color used by the insertion cursor.
    pub fn set_cursor_color(&mut self, color: Color) {
        if self.cursor_visible {
            self.erase_cursor();
        }
        self.cursor_color = color;
        if self.cursor_visible {
            self.draw_cursor();
        }
    }

    /// Current text colors (foreground, background).
    pub const fn colors(&self) -> (Color, Color) {
        (self.foreground, self.background)
    }

    /// Change the colors used by subsequent output.
    ///
    /// Existing framebuffer contents are not repainted.
    pub fn set_colors(&mut self, foreground: Color, background: Color) {
        self.erase_cursor_if_visible();
        self.foreground = foreground;
        self.background = background;
        self.draw_cursor_if_visible();
    }

    /// Write a string using temporary colors, then restore the previous colors.
    ///
    /// Newline and scrolling behavior is identical to `write_str`.
    pub fn write_str_with_colors(
        &mut self,
        text: &str,
        foreground: Color,
        background: Color,
    ) {
        let previous = self.colors();
        self.set_colors(foreground, background);
        self.write_str(text);
        self.set_colors(previous.0, previous.1);
    }

    /// Write a line using temporary colors, then restore the previous colors.
    pub fn write_line_with_colors(
        &mut self,
        text: &str,
        foreground: Color,
        background: Color,
    ) {
        let previous = self.colors();
        self.set_colors(foreground, background);
        self.write_line(text);
        self.set_colors(previous.0, previous.1);
    }

    /// Write one character using temporary colors, then restore the previous colors.
    pub fn write_char_with_colors(
        &mut self,
        character: char,
        foreground: Color,
        background: Color,
    ) {
        let previous = self.colors();
        self.set_colors(foreground, background);
        self.write_char(character);
        self.set_colors(previous.0, previous.1);
    }

    /// Write a UTF-8 string. Unsupported non-ASCII characters use the font's
    /// replacement glyph.
    pub fn write_str(&mut self, text: &str) {
        self.erase_cursor_if_visible();
        for character in text.chars() {
            self.write_char_inner(character);
        }
        self.draw_cursor_if_visible();
    }

    /// Write text followed by a newline.
    pub fn write_line(&mut self, text: &str) {
        self.erase_cursor_if_visible();
        for character in text.chars() {
            self.write_char_inner(character);
        }
        self.newline();
        self.draw_cursor_if_visible();
    }

    /// Clear the display and reset the cursor.
    pub fn clear(&mut self) {
        self.erase_cursor_if_visible();
        self.surface.clear(self.background);
        self.column = 0;
        self.row = 0;
        self.draw_cursor_if_visible();
    }

    /// Write one character or process a basic terminal control character.
    pub fn write_char(&mut self, character: char) {
        self.erase_cursor_if_visible();
        self.write_char_inner(character);
        self.draw_cursor_if_visible();
    }

    fn write_char_inner(&mut self, character: char) {
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

    fn erase_cursor_if_visible(&mut self) {
        if self.cursor_visible {
            self.erase_cursor();
        }
    }

    fn draw_cursor_if_visible(&mut self) {
        if self.cursor_visible {
            self.draw_cursor();
        }
    }

    fn draw_cursor(&mut self) {
        let x = self.column * FONT_ADVANCE;
        let y = self.row * FONT_LINE_HEIGHT + FONT_HEIGHT;
        self.surface.fill_rect(x, y, FONT_ADVANCE - 1, FONT_LINE_HEIGHT - FONT_HEIGHT, self.cursor_color);
    }

    fn erase_cursor(&mut self) {
        let x = self.column * FONT_ADVANCE;
        let y = self.row * FONT_LINE_HEIGHT + FONT_HEIGHT;
        self.surface.fill_rect(x, y, FONT_ADVANCE - 1, FONT_LINE_HEIGHT - FONT_HEIGHT, self.background);
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
            FONT_LINE_HEIGHT,
            self.background,
        );
    }
}
