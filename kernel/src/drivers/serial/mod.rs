//! Polling driver for 16550-compatible UART serial ports.
//! Allocation-free COM1–COM4 support; IRQ delivery is owned by the interrupt subsystem.

use crate::arch::x86_64::io;
use crate::drivers::api::{BusAddress, DeviceClass, DeviceInfo, Driver, DriverError};

const DATA: u16 = 0;
const IER: u16 = 1;
const FCR: u16 = 2;
const LCR: u16 = 3;
const MCR: u16 = 4;
const LSR: u16 = 5;
const DATA_READY: u8 = 1;
const THR_EMPTY: u8 = 1 << 5;
const DEFAULT_POLL_LIMIT: usize = 1_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Parity { None, Odd, Even }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UartConfig {
    pub baud_rate: u32,
    pub data_bits: u8,
    pub stop_bits: u8,
    pub parity: Parity,
}

impl Default for UartConfig {
    fn default() -> Self {
        Self { baud_rate: 115_200, data_bits: 8, stop_bits: 1, parity: Parity::None }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UartError {
    InvalidBasePort, InvalidBaudRate, InvalidFrameFormat, NotInitialized, Timeout,
}

/// Exclusive owner of one legacy 16550 UART I/O-port range.
pub struct Uart16550 {
    base: u16,
    config: UartConfig,
    initialized: bool,
    poll_limit: usize,
}

impl Uart16550 {
    /// Creates a UART handle without touching hardware.
    ///
    /// # Safety
    /// The caller must guarantee exclusive ownership of the selected port range.
    pub const unsafe fn new(base: u16, config: UartConfig) -> Self {
        Self { base, config, initialized: false, poll_limit: DEFAULT_POLL_LIMIT }
    }

    pub fn set_poll_limit(&mut self, limit: usize) { self.poll_limit = limit; }
    #[inline]
    fn port(&self, register: u16) -> u16 { self.base + register }

    fn validate(&self) -> Result<u16, UartError> {
        if !matches!(self.base, 0x3F8 | 0x2F8 | 0x3E8 | 0x2E8) {
            return Err(UartError::InvalidBasePort);
        }
        if self.config.baud_rate == 0 || 115_200 % self.config.baud_rate != 0 {
            return Err(UartError::InvalidBaudRate);
        }
        if !(5..=8).contains(&self.config.data_bits) || !(1..=2).contains(&self.config.stop_bits) {
            return Err(UartError::InvalidFrameFormat);
        }
        Ok((115_200 / self.config.baud_rate) as u16)
    }

    /// Programs baud divisor, FIFO, line format and modem control.
    pub fn initialize(&mut self) -> Result<(), UartError> {
        let divisor = self.validate()?;
        // SAFETY: The constructor contract guarantees exclusive port ownership.
        unsafe {
            io::out8(self.port(IER), 0);
            io::out8(self.port(LCR), 0x80);
            io::out8(self.port(DATA), divisor as u8);
            io::out8(self.port(IER), (divisor >> 8) as u8);
            let mut line = self.config.data_bits - 5;
            if self.config.stop_bits == 2 { line |= 1 << 2; }
            line |= match self.config.parity {
                Parity::None => 0, Parity::Odd => 0b001 << 3, Parity::Even => 0b011 << 3,
            };
            io::out8(self.port(LCR), line);
            io::out8(self.port(FCR), 0xC7);
            io::out8(self.port(MCR), 0x0B);
        }
        self.initialized = true;
        Ok(())
    }

    fn ensure_initialized(&self) -> Result<(), UartError> {
        if self.initialized { Ok(()) } else { Err(UartError::NotInitialized) }
    }

    pub fn transmitter_ready(&self) -> Result<bool, UartError> {
        self.ensure_initialized()?;
        // SAFETY: This instance owns the UART.
        Ok(unsafe { io::in8(self.port(LSR)) } & THR_EMPTY != 0)
    }

    /// Non-blocking receive; None means no byte is available.
    pub fn try_read_byte(&self) -> Result<Option<u8>, UartError> {
        self.ensure_initialized()?;
        // SAFETY: This instance owns the UART.
        let status = unsafe { io::in8(self.port(LSR)) };
        if status & DATA_READY == 0 { return Ok(None); }
        // SAFETY: The data-ready status indicates a byte is available.
        Ok(Some(unsafe { io::in8(self.port(DATA)) }))
    }

    /// Sends one byte, using bounded polling to prevent an infinite kernel hang.
    pub fn write_byte(&self, byte: u8) -> Result<(), UartError> {
        self.ensure_initialized()?;
        for _ in 0..self.poll_limit {
            // SAFETY: This instance owns the UART.
            if unsafe { io::in8(self.port(LSR)) } & THR_EMPTY != 0 {
                // SAFETY: The transmitter holding register is ready.
                unsafe { io::out8(self.port(DATA), byte) };
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(UartError::Timeout)
    }

    pub fn write(&self, bytes: &[u8]) -> Result<usize, (usize, UartError)> {
        let mut written = 0;
        for &byte in bytes {
            if let Err(error) = self.write_byte(byte) { return Err((written, error)); }
            written += 1;
        }
        Ok(written)
    }

    /// Drains currently available input into the supplied buffer.
    pub fn read_available(&self, buffer: &mut [u8]) -> Result<usize, UartError> {
        self.ensure_initialized()?;
        let mut read = 0;
        while read < buffer.len() {
            match self.try_read_byte()? {
                Some(byte) => { buffer[read] = byte; read += 1; }
                None => break,
            }
        }
        Ok(read)
    }

    pub fn shutdown(&mut self) -> Result<(), UartError> {
        self.ensure_initialized()?;
        // SAFETY: This instance owns the UART.
        unsafe { io::out8(self.port(IER), 0); io::out8(self.port(MCR), 0); }
        self.initialized = false;
        Ok(())
    }

    pub const fn base_port(&self) -> u16 { self.base }
}

impl Driver for Uart16550 {
    fn name(&self) -> &'static str { "16550-compatible UART" }
    fn supports(&self, device: &DeviceInfo) -> bool {
        device.class == DeviceClass::Serial
            && matches!(device.address, BusAddress::Platform { .. })
    }
    fn initialize(&mut self, _device: &DeviceInfo) -> Result<(), DriverError> {
        Uart16550::initialize(self).map_err(|error| match error {
            UartError::InvalidBasePort | UartError::InvalidBaudRate | UartError::InvalidFrameFormat =>
                DriverError::InvalidConfiguration,
            UartError::NotInitialized | UartError::Timeout => DriverError::NotReady,
        })
    }
    fn shutdown(&mut self) -> Result<(), DriverError> {
        Uart16550::shutdown(self).map_err(|_| DriverError::HardwareFault)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_115200_8n1() {
        assert_eq!(UartConfig::default(), UartConfig {
            baud_rate: 115_200, data_bits: 8, stop_bits: 1, parity: Parity::None,
        });
    }

    #[test]
    fn validates_ports_baud_and_frame_format() {
        // SAFETY: Validation does not access hardware.
        let bad_port = unsafe { Uart16550::new(0x1234, UartConfig::default()) };
        assert_eq!(bad_port.validate(), Err(UartError::InvalidBasePort));
        // SAFETY: Validation does not access hardware.
        let valid = unsafe { Uart16550::new(0x3F8, UartConfig {
            baud_rate: 9_600, ..UartConfig::default()
        }) };
        assert_eq!(valid.validate(), Ok(12));
        // SAFETY: Validation does not access hardware.
        let bad_baud = unsafe { Uart16550::new(0x3F8, UartConfig {
            baud_rate: 10_000, ..UartConfig::default()
        }) };
        assert_eq!(bad_baud.validate(), Err(UartError::InvalidBaudRate));
        // SAFETY: Validation does not access hardware.
        let bad_frame = unsafe { Uart16550::new(0x3F8, UartConfig {
            data_bits: 9, ..UartConfig::default()
        }) };
        assert_eq!(bad_frame.validate(), Err(UartError::InvalidFrameFormat));
    }

    #[test]
    fn refuses_io_before_initialization() {
        // SAFETY: The method returns before touching hardware.
        let uart = unsafe { Uart16550::new(0x3F8, UartConfig::default()) };
        assert_eq!(uart.try_read_byte(), Err(UartError::NotInitialized));
    }
}


impl core::fmt::Write for Uart16550 {
    /// Writes UTF-8 text to the UART, normalizing LF to CRLF for terminals.
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        let mut previous_was_cr = false;
        for byte in text.bytes() {
            if byte == b'\n' && !previous_was_cr {
                self.write_byte(b'\r').map_err(|_| core::fmt::Error)?;
            }
            self.write_byte(byte).map_err(|_| core::fmt::Error)?;
            previous_was_cr = byte == b'\r';
        }
        Ok(())
    }
}
