//! Bounded-polling driver for the legacy i8042 PS/2 controller.
//!
//! Initialization leaves device IRQs disabled. Install interrupt handlers and
//! configure the PIC/IOAPIC before enabling either device interrupt.

use crate::arch::x86_64::io;
use core::sync::atomic::{AtomicU8, Ordering};

const DATA: u16 = 0x60;
const STATUS_COMMAND: u16 = 0x64;
const INPUT_FULL: u8 = 1 << 1;
const OUTPUT_FULL: u8 = 1;
const DEFAULT_POLL_LIMIT: usize = 100_000;

const CMD_READ_CONFIG: u8 = 0x20;
const CMD_WRITE_CONFIG: u8 = 0x60;
const CMD_DISABLE_KEYBOARD: u8 = 0xAD;
const CMD_ENABLE_KEYBOARD: u8 = 0xAE;
const CMD_DISABLE_MOUSE: u8 = 0xA7;
const CMD_ENABLE_MOUSE: u8 = 0xA8;
const CMD_TEST_KEYBOARD: u8 = 0xAB;
const CMD_TEST_MOUSE: u8 = 0xA9;
const CMD_SELF_TEST: u8 = 0xAA;
const CMD_WRITE_MOUSE: u8 = 0xD4;

const CONFIG_IRQ_KEYBOARD: u8 = 1;
const CONFIG_IRQ_MOUSE: u8 = 1 << 1;
const CONFIG_TRANSLATION: u8 = 1 << 6;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ps2Error {
    Timeout,
    ControllerSelfTestFailed(u8),
    KeyboardPortTestFailed(u8),
    MousePortTestFailed(u8),
    NotInitialized,
    DeviceRejected(u8),
    UnsupportedMouseId(u8),
}

/// Result of controller diagnostics. A port test value of zero means success.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ps2Diagnostics {
    pub keyboard_port: u8,
    pub mouse_port: u8,
}

/// Protocol negotiated with a PS/2 mouse.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ps2MouseProtocol {
    /// Three-byte standard PS/2 packets.
    Standard,
    /// Four-byte IntelliMouse packets with a signed wheel delta.
    Wheel,
    /// Four-byte five-button IntelliMouse Explorer packets.
    FiveButton,
}

/// One byte removed from the i8042 output buffer, including its status snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ps2DataByte {
    /// Status captured before reading the data port.
    pub status: u8,
    /// Byte returned by data port 0x60.
    pub value: u8,
}

/// Exclusive owner of the legacy i8042 controller.
pub struct Ps2Controller {
    poll_limit: usize,
    config: AtomicU8,
    initialized: bool,
    keyboard_available: bool,
    mouse_available: bool,
}

impl Ps2Controller {
    /// Creates a controller handle without touching hardware.
    ///
    /// # Safety
    /// The caller must guarantee exclusive ownership of ports 0x60 and 0x64.
    pub const unsafe fn new() -> Self {
        Self {
            poll_limit: DEFAULT_POLL_LIMIT,
            config: AtomicU8::new(0),
            initialized: false,
            keyboard_available: false,
            mouse_available: false,
        }
    }

    pub fn set_poll_limit(&mut self, limit: usize) {
        self.poll_limit = limit;
    }

    fn wait_input_empty(&self) -> Result<(), Ps2Error> {
        for _ in 0..self.poll_limit {
            // SAFETY: The constructor contract grants ownership of the status port.
            if unsafe { io::in8(STATUS_COMMAND) } & INPUT_FULL == 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(Ps2Error::Timeout)
    }

    fn wait_output_full(&self) -> Result<(), Ps2Error> {
        for _ in 0..self.poll_limit {
            // SAFETY: The constructor contract grants ownership of the status port.
            if unsafe { io::in8(STATUS_COMMAND) } & OUTPUT_FULL != 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(Ps2Error::Timeout)
    }

    fn command(&self, value: u8) -> Result<(), Ps2Error> {
        self.wait_input_empty()?;
        // SAFETY: The controller command port is exclusively owned by this instance.
        unsafe { io::out8(STATUS_COMMAND, value) };
        Ok(())
    }

    fn write_data(&self, value: u8) -> Result<(), Ps2Error> {
        self.wait_input_empty()?;
        // SAFETY: The controller data port is exclusively owned by this instance.
        unsafe { io::out8(DATA, value) };
        Ok(())
    }

    fn read_data(&self) -> Result<u8, Ps2Error> {
        self.wait_output_full()?;
        // SAFETY: A full output buffer guarantees a byte can be read.
        Ok(unsafe { io::in8(DATA) })
    }

    fn drain_output(&self) {
        for _ in 0..self.poll_limit {
            // SAFETY: The controller status/data ports are exclusively owned.
            if unsafe { io::in8(STATUS_COMMAND) } & OUTPUT_FULL == 0 {
                break;
            }
            // SAFETY: The output buffer is known to contain stale data.
            let _ = unsafe { io::in8(DATA) };
        }
    }

    /// Disables both ports, runs controller diagnostics and configures Set 1
    /// translation while keeping device IRQ generation disabled.
    ///
    /// # Safety
    /// Must execute at CPL 0 with CPU interrupts disabled and exclusive
    /// ownership of the i8042 ports. Firmware/platform code must have left an
    /// operational legacy-compatible controller.
    pub unsafe fn initialize(&mut self) -> Result<Ps2Diagnostics, Ps2Error> {
        self.command(CMD_DISABLE_KEYBOARD)?;
        self.command(CMD_DISABLE_MOUSE)?;
        self.drain_output();

        self.command(CMD_SELF_TEST)?;
        let self_test = self.read_data()?;
        if self_test != 0x55 {
            return Err(Ps2Error::ControllerSelfTestFailed(self_test));
        }

        // Controller self-test may reset the configuration byte.
        self.command(CMD_READ_CONFIG)?;
        let mut config = self.read_data()?;
        config &= !(CONFIG_IRQ_KEYBOARD | CONFIG_IRQ_MOUSE);
        config |= CONFIG_TRANSLATION;
        self.command(CMD_WRITE_CONFIG)?;
        self.write_data(config)?;
        self.config.store(config, Ordering::Release);

        self.command(CMD_TEST_KEYBOARD)?;
        let keyboard_port = self.read_data()?;
        self.command(CMD_TEST_MOUSE)?;
        let mouse_port = self.read_data()?;

        self.keyboard_available = keyboard_port == 0;
        self.mouse_available = mouse_port == 0;
        self.initialized = true;

        Ok(Ps2Diagnostics { keyboard_port, mouse_port })
    }

    fn ensure_initialized(&self) -> Result<(), Ps2Error> {
        if self.initialized { Ok(()) } else { Err(Ps2Error::NotInitialized) }
    }

    /// Enables the keyboard clock/port but deliberately leaves IRQ1 disabled.
    pub fn enable_keyboard_port(&mut self) -> Result<(), Ps2Error> {
        self.ensure_initialized()?;
        if !self.keyboard_available { return Err(Ps2Error::KeyboardPortTestFailed(1)); }
        self.command(CMD_ENABLE_KEYBOARD)?;
        Ok(())
    }

    /// Enables the auxiliary mouse clock/port but deliberately leaves IRQ12 disabled.
    pub fn enable_mouse_port(&mut self) -> Result<(), Ps2Error> {
        self.ensure_initialized()?;
        if !self.mouse_available { return Err(Ps2Error::MousePortTestFailed(1)); }
        self.command(CMD_ENABLE_MOUSE)?;
        Ok(())
    }

    fn send_device_byte(&self, auxiliary: bool, value: u8) -> Result<(), Ps2Error> {
        self.ensure_initialized()?;
        for _ in 0..3 {
            if auxiliary { self.command(CMD_WRITE_MOUSE)?; }
            self.write_data(value)?;
            match self.read_data()? {
                0xFA => return Ok(()),
                0xFE => continue,
                response => return Err(Ps2Error::DeviceRejected(response)),
            }
        }
        Err(Ps2Error::DeviceRejected(0xFE))
    }

    /// Sends one keyboard command and validates ACK/RESEND responses.
    pub fn send_keyboard_command(&self, value: u8) -> Result<(), Ps2Error> {
        self.send_device_byte(false, value)
    }

    /// Sends one auxiliary mouse command and validates ACK/RESEND responses.
    pub fn send_mouse_command(&self, value: u8) -> Result<(), Ps2Error> {
        self.send_device_byte(true, value)
    }

    /// Reads one pending output byte and preserves its controller status.
    /// The status snapshot must be passed to the protocol pipeline so mouse
    /// bytes, parity errors, and timeout errors are not misclassified.
    pub fn try_read_with_status(&self) -> Result<Option<Ps2DataByte>, Ps2Error> {
        self.ensure_initialized()?;
        // SAFETY: The controller status/data ports are exclusively owned.
        let status = unsafe { io::in8(STATUS_COMMAND) };
        if status & OUTPUT_FULL == 0 { return Ok(None); }
        // SAFETY: OUTPUT_FULL guarantees a byte is available.
        let value = unsafe { io::in8(DATA) };
        Ok(Some(Ps2DataByte { status, value }))
    }

    /// Compatibility helper when the caller intentionally does not need status.
    pub fn try_read_byte(&self) -> Result<Option<u8>, Ps2Error> {
        Ok(self.try_read_with_status()?.map(|byte| byte.value))
    }

    /// Resets keyboard scanning state and enables scan-code reporting.
    /// The controller's translation bit keeps the device's default Set 2 stream
    /// compatible with the Set 1 decoder used by this driver.
    pub fn configure_keyboard(&self) -> Result<(), Ps2Error> {
        self.send_keyboard_command(0xF5)?;
        self.send_keyboard_command(0xF4)
    }

    /// Restores mouse defaults and enables packet reporting.
    pub fn configure_mouse(&self) -> Result<(), Ps2Error> {
        self.send_mouse_command(0xF6)?;
        self.send_mouse_command(0xF4)
    }

    /// Negotiates the mouse packet format using the IntelliMouse sample-rate
    /// sequence (200, 100, 80), queries the device ID, then enables reporting.
    ///
    /// The returned protocol must be used to construct `Ps2MouseDecoder`;
    /// enabling four-byte decoding without this negotiation desynchronizes
    /// standard three-byte mouse streams.
    pub fn configure_mouse_with_wheel(&self) -> Result<Ps2MouseProtocol, Ps2Error> {
        self.send_mouse_command(0xF6)?;
        for rate in [200, 100, 80] {
            self.send_mouse_command(0xF3)?;
            self.send_mouse_command(rate)?;
        }
        self.send_mouse_command(0xF2)?;
        let device_id = self.read_data()?;
        let protocol = match device_id {
            0 => Ps2MouseProtocol::Standard,
            3 => Ps2MouseProtocol::Wheel,
            4 => Ps2MouseProtocol::FiveButton,
            other => return Err(Ps2Error::UnsupportedMouseId(other)),
        };
        self.send_mouse_command(0xF4)?;
        Ok(protocol)
    }

    fn update_irq_config(&self, set: u8, clear: u8) -> Result<(), Ps2Error> {
        self.ensure_initialized()?;
        let current = self.config.load(Ordering::Acquire);
        let next = (current | set) & !clear;
        self.command(CMD_WRITE_CONFIG)?;
        self.write_data(next)?;
        self.config.store(next, Ordering::Release);
        Ok(())
    }

    /// Enables IRQ1 after its handler has been registered and its PIC line
    /// remains masked. The caller unmasks the PIC only after all setup finishes.
    ///
    /// # Safety
    /// Must be serialized with every other i8042 operation and run with IF clear.
    pub unsafe fn enable_keyboard_irq(&self) -> Result<(), Ps2Error> {
        self.update_irq_config(CONFIG_IRQ_KEYBOARD, 0)
    }

    /// Enables IRQ12 after its handler has been registered and its PIC line
    /// remains masked. The caller unmasks the PIC only after all setup finishes.
    ///
    /// # Safety
    /// Must be serialized with every other i8042 operation and run with IF clear.
    pub unsafe fn enable_mouse_irq(&self) -> Result<(), Ps2Error> {
        self.update_irq_config(CONFIG_IRQ_MOUSE, 0)
    }

    /// Disables IRQ1 without disabling the keyboard port.
    ///
    /// # Safety
    /// Must be serialized with every other i8042 operation and run with IF clear.
    pub unsafe fn disable_keyboard_irq(&self) -> Result<(), Ps2Error> {
        self.update_irq_config(0, CONFIG_IRQ_KEYBOARD)
    }

    /// Disables IRQ12 without disabling the auxiliary mouse port.
    ///
    /// # Safety
    /// Must be serialized with every other i8042 operation and run with IF clear.
    pub unsafe fn disable_mouse_irq(&self) -> Result<(), Ps2Error> {
        self.update_irq_config(0, CONFIG_IRQ_MOUSE)
    }

    pub const fn keyboard_available(&self) -> bool { self.keyboard_available }
    pub const fn mouse_available(&self) -> bool { self.mouse_available }
    pub const fn is_initialized(&self) -> bool { self.initialized }
}
