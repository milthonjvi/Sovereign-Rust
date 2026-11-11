//! Allocation-free kernel logging primitives.
//!
//! A logger owns its output sink rather than relying on a global mutable
//! singleton. Early boot can therefore select UART or another formatting sink.
//! Do not call a polling UART logger from hard-interrupt context.

use core::fmt::{self, Write};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum LogLevel { Error, Warn, Info, Debug, Trace }

impl LogLevel {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "ERROR",
            Self::Warn => "WARN",
            Self::Info => "INFO",
            Self::Debug => "DEBUG",
            Self::Trace => "TRACE",
        }
    }
}

/// Synchronous logger with a caller-selected, allocation-free sink.
pub struct Logger<W> {
    writer: W,
    minimum_level: LogLevel,
}

impl<W: Write> Logger<W> {
    pub const fn new(writer: W, minimum_level: LogLevel) -> Self {
        Self { writer, minimum_level }
    }

    pub fn minimum_level(&self) -> LogLevel { self.minimum_level }

    pub fn set_minimum_level(&mut self, level: LogLevel) {
        self.minimum_level = level;
    }

    pub fn log(&mut self, level: LogLevel, arguments: fmt::Arguments<'_>) -> fmt::Result {
        if level > self.minimum_level { return Ok(()); }
        self.writer.write_str("[")?;
        self.writer.write_str(level.as_str())?;
        self.writer.write_str("] ")?;
        self.writer.write_fmt(arguments)?;
        self.writer.write_str("\r\n")
    }

    pub fn into_inner(self) -> W { self.writer }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Buffer { bytes: [u8; 128], len: usize }

    impl Buffer {
        const fn new() -> Self { Self { bytes: [0; 128], len: 0 } }
        fn as_str(&self) -> &str {
            core::str::from_utf8(&self.bytes[..self.len]).unwrap()
        }
    }

    impl Write for Buffer {
        fn write_str(&mut self, text: &str) -> fmt::Result {
            let end = self.len + text.len();
            if end > self.bytes.len() { return Err(fmt::Error); }
            self.bytes[self.len..end].copy_from_slice(text.as_bytes());
            self.len = end;
            Ok(())
        }
    }

    #[test]
    fn formats_level_and_message_without_allocation() {
        let mut logger = Logger::new(Buffer::new(), LogLevel::Info);
        logger.log(LogLevel::Warn, format_args!("device {} ready", 3)).unwrap();
        assert_eq!(logger.into_inner().as_str(), "[WARN] device 3 ready\r\n");
    }

    #[test]
    fn filters_levels_above_threshold() {
        let mut logger = Logger::new(Buffer::new(), LogLevel::Warn);
        logger.log(LogLevel::Debug, format_args!("hidden")).unwrap();
        assert_eq!(logger.into_inner().len, 0);
    }

    #[test]
    fn allows_errors_at_error_threshold() {
        let mut logger = Logger::new(Buffer::new(), LogLevel::Error);
        logger.log(LogLevel::Error, format_args!("fault")).unwrap();
        assert_eq!(logger.into_inner().as_str(), "[ERROR] fault\r\n");
    }
}
