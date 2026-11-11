//! Core formatting integration for the framebuffer console.

use core::fmt;

use super::console::FramebufferConsole;

impl fmt::Write for FramebufferConsole<'_> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        FramebufferConsole::write_str(self, text);
        Ok(())
    }
}
