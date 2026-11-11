//! 16550 receive IRQ integration with the kernel's legacy PIC dispatcher.
//!
//! COM1/COM3 share IRQ4; COM2/COM4 share IRQ3. One bounded handler drains
//! every attached UART on the shared line. RX bytes are published to separate
//! lock-free SPSC queues for task-context consumers.

use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

use crate::arch::x86_64::io;
use crate::interrupts::init as interrupt_init;

use super::{Uart16550, UartError, UartRxBuffer};

const DATA: u16 = 0;
const LSR: u16 = 5;
const DATA_READY: u8 = 1;
const OVERRUN_ERROR: u8 = 1 << 1;
const PARITY_ERROR: u8 = 1 << 2;
const FRAMING_ERROR: u8 = 1 << 3;
const BREAK_INTERRUPT: u8 = 1 << 4;
const FIFO_ERROR: u8 = 1 << 7;
const MAX_DRAIN_PER_PORT: usize = 64;
const RX_CAPACITY: usize = 1024;

const COM1_BIT: u8 = 1 << 0;
const COM2_BIT: u8 = 1 << 1;
const COM3_BIT: u8 = 1 << 2;
const COM4_BIT: u8 = 1 << 3;
const IRQ3_PORTS: u8 = COM2_BIT | COM4_BIT;
const IRQ4_PORTS: u8 = COM1_BIT | COM3_BIT;

static ATTACHED: AtomicU8 = AtomicU8::new(0);
static COM1_RX: UartRxBuffer<RX_CAPACITY> = UartRxBuffer::new();
static COM2_RX: UartRxBuffer<RX_CAPACITY> = UartRxBuffer::new();
static COM3_RX: UartRxBuffer<RX_CAPACITY> = UartRxBuffer::new();
static COM4_RX: UartRxBuffer<RX_CAPACITY> = UartRxBuffer::new();

/// Hardware receive errors observed by the UART interrupt handler.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UartRxStatistics {
    pub overrun_errors: usize,
    pub parity_errors: usize,
    pub framing_errors: usize,
    pub break_interrupts: usize,
    pub fifo_errors: usize,
}

struct AtomicRxStatistics {
    overrun: AtomicUsize,
    parity: AtomicUsize,
    framing: AtomicUsize,
    breaks: AtomicUsize,
    fifo: AtomicUsize,
}

impl AtomicRxStatistics {
    const fn new() -> Self {
        Self {
            overrun: AtomicUsize::new(0),
            parity: AtomicUsize::new(0),
            framing: AtomicUsize::new(0),
            breaks: AtomicUsize::new(0),
            fifo: AtomicUsize::new(0),
        }
    }

    fn snapshot(&self) -> UartRxStatistics {
        UartRxStatistics {
            overrun_errors: self.overrun.load(Ordering::Relaxed),
            parity_errors: self.parity.load(Ordering::Relaxed),
            framing_errors: self.framing.load(Ordering::Relaxed),
            break_interrupts: self.breaks.load(Ordering::Relaxed),
            fifo_errors: self.fifo.load(Ordering::Relaxed),
        }
    }

    fn record_lsr(&self, status: u8) {
        if status & OVERRUN_ERROR != 0 { self.overrun.fetch_add(1, Ordering::Relaxed); }
        if status & PARITY_ERROR != 0 { self.parity.fetch_add(1, Ordering::Relaxed); }
        if status & FRAMING_ERROR != 0 { self.framing.fetch_add(1, Ordering::Relaxed); }
        if status & BREAK_INTERRUPT != 0 { self.breaks.fetch_add(1, Ordering::Relaxed); }
        if status & FIFO_ERROR != 0 { self.fifo.fetch_add(1, Ordering::Relaxed); }
    }
}

static COM1_STATS: AtomicRxStatistics = AtomicRxStatistics::new();
static COM2_STATS: AtomicRxStatistics = AtomicRxStatistics::new();
static COM3_STATS: AtomicRxStatistics = AtomicRxStatistics::new();
static COM4_STATS: AtomicRxStatistics = AtomicRxStatistics::new();

/// Errors returned by UART interrupt attachment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UartIrqError {
    UnsupportedPort,
    AlreadyAttached,
    NotAttached,
    Uart(UartError),
    InterruptSubsystem,
    HandlerAlreadyOwned,
}

struct PortBinding {
    irq: u8,
    bit: u8,
    shared_line_mask: u8,
    handler: extern "C" fn(u8),
    queue: &'static UartRxBuffer<RX_CAPACITY>,
}

fn binding(base: u16) -> Option<PortBinding> {
    match base {
        0x3F8 => Some(PortBinding {
            irq: 4, bit: COM1_BIT, shared_line_mask: IRQ4_PORTS,
            handler: irq4_handler, queue: &COM1_RX,\n            stats: &COM1_STATS,
        }),
        0x2F8 => Some(PortBinding {
            irq: 3, bit: COM2_BIT, shared_line_mask: IRQ3_PORTS,
            handler: irq3_handler, queue: &COM2_RX,\n            stats: &COM2_STATS,
        }),
        0x3E8 => Some(PortBinding {
            irq: 4, bit: COM3_BIT, shared_line_mask: IRQ4_PORTS,
            handler: irq4_handler, queue: &COM3_RX,\n            stats: &COM3_STATS,
        }),
        0x2E8 => Some(PortBinding {
            irq: 3, bit: COM4_BIT, shared_line_mask: IRQ3_PORTS,
            handler: irq3_handler, queue: &COM4_RX,\n            stats: &COM4_STATS,
        }),
        _ => None,
    }
}

/// Attach a configured UART to interrupt-driven receive.
///
/// The correct handler is registered while the PIC line is masked, UART RX
/// interrupts are enabled, and the PIC line is unmasked last. The returned
/// queue is consumed from normal kernel context.
///
/// # Safety
/// Call at CPL 0 with IF clear. No other subsystem may own IRQ3/IRQ4. The UART
/// must remain exclusively owned by this driver while attached. The queue must
/// have exactly one consumer, and that consumer must use its unsafe SPSC pop.
pub unsafe fn attach_interrupts(
    uart: &mut Uart16550,
) -> Result<&'static UartRxBuffer<RX_CAPACITY>, UartIrqError> {
    uart.ensure_initialized().map_err(UartIrqError::Uart)?;
    let port = binding(uart.base_port()).ok_or(UartIrqError::UnsupportedPort)?;
    let current = ATTACHED.load(Ordering::Acquire);
    if current & port.bit != 0 {
        return Err(UartIrqError::AlreadyAttached);
    }

    let first_on_line = current & port.shared_line_mask == 0;
    if first_on_line {
        // SAFETY: Caller keeps IF clear and guarantees exclusive IRQ3/IRQ4 ownership.
        unsafe { interrupt_init::register_legacy_irq(port.irq, port.handler) }
            .map_err(|error| match error {
                interrupt_init::LegacyIrqError::HandlerAlreadyRegistered =>
                    UartIrqError::HandlerAlreadyOwned,
                _ => UartIrqError::InterruptSubsystem,
            })?;
    }

    ATTACHED.fetch_or(port.bit, Ordering::Release);

    if let Err(error) = uart.enable_receive_interrupt() {
        ATTACHED.fetch_and(!port.bit, Ordering::AcqRel);
        if first_on_line {
            // SAFETY: The line remains masked and IF is clear.
            let _ = unsafe { interrupt_init::unregister_legacy_irq(port.irq, port.handler) };
        }
        return Err(UartIrqError::Uart(error));
    }

    // SAFETY: Handler is registered, the UART IER is configured, and IF is clear.
    if unsafe { interrupt_init::unmask_legacy_irq(port.irq) }.is_err() {
        let _ = uart.disable_interrupts();
        ATTACHED.fetch_and(!port.bit, Ordering::AcqRel);
        if first_on_line {
            // SAFETY: The PIC line is masked by the failed/unmask rollback path.
            let _ = unsafe { interrupt_init::unregister_legacy_irq(port.irq, port.handler) };
        }
        return Err(UartIrqError::InterruptSubsystem);
    }

    Ok(port.queue)
}

/// Detach a UART and stop its receive interrupts.
///
/// # Safety
/// Call at CPL 0 with IF clear and after the consumer has stopped accessing the
/// queue. No handler may be executing on another CPU.
pub unsafe fn detach_interrupts(uart: &mut Uart16550) -> Result<(), UartIrqError> {
    uart.ensure_initialized().map_err(UartIrqError::Uart)?;
    let port = binding(uart.base_port()).ok_or(UartIrqError::UnsupportedPort)?;
    let current = ATTACHED.load(Ordering::Acquire);
    if current & port.bit == 0 {
        return Err(UartIrqError::NotAttached);
    }

    let last_on_line = current & port.shared_line_mask == port.bit;
    if last_on_line {
        // SAFETY: IF is clear; this driver owns the registered shared-line handler.
        unsafe { interrupt_init::mask_legacy_irq(port.irq) }
            .map_err(|_| UartIrqError::InterruptSubsystem)?;
    }

    uart.disable_interrupts().map_err(UartIrqError::Uart)?;
    ATTACHED.fetch_and(!port.bit, Ordering::AcqRel);

    if last_on_line {
        // SAFETY: Source is masked and IF is clear; no callback is in flight.
        unsafe { interrupt_init::unregister_legacy_irq(port.irq, port.handler) }
            .map_err(|_| UartIrqError::InterruptSubsystem)?;
    }
    Ok(())
}

/// Returns the static RX queue associated with a legacy UART base port.
pub fn receive_buffer(base: u16) -> Option<&'static UartRxBuffer<RX_CAPACITY>> {
    binding(base).map(|port| port.queue)
}

/// Returns a snapshot of hardware line errors counted by the RX interrupt path.
pub fn receive_statistics(base: u16) -> Option<UartRxStatistics> {
    binding(base).map(|port| port.stats.snapshot())
}

fn drain_port(
    base: u16,
    queue: &'static UartRxBuffer<RX_CAPACITY>,
    stats: &'static AtomicRxStatistics,
) {
    for _ in 0..MAX_DRAIN_PER_PORT {
        // SAFETY: The attached UART owns this I/O range and the IRQ handler is
        // the sole producer for its SPSC queue. Reading LSR acknowledges latched
        // line-status errors on 16550-compatible devices.
        let status = unsafe { io::in8(base + LSR) };
        stats.record_lsr(status);
        if status & DATA_READY == 0 {
            break;
        }
        // SAFETY: LSR reports that the receive data register contains a byte.
        let byte = unsafe { io::in8(base + DATA) };
        // SAFETY: This shared-line handler is the only producer for this queue.
        let _ = unsafe { queue.push(byte) };
    }
}

extern "C" fn irq3_handler(_vector: u8) {
    let attached = ATTACHED.load(Ordering::Acquire);
    if attached & COM2_BIT != 0 { drain_port(0x2F8, &COM2_RX, &COM2_STATS); }
    if attached & COM4_BIT != 0 { drain_port(0x2E8, &COM4_RX, &COM4_STATS); }
}

extern "C" fn irq4_handler(_vector: u8) {
    let attached = ATTACHED.load(Ordering::Acquire);
    if attached & COM1_BIT != 0 { drain_port(0x3F8, &COM1_RX, &COM1_STATS); }
    if attached & COM3_BIT != 0 { drain_port(0x3E8, &COM3_RX, &COM3_STATS); }
}

/// Reports whether a UART port is currently connected to its shared IRQ line.
pub(super) fn is_attached(base: u16) -> bool {
    binding(base).is_some_and(|port| ATTACHED.load(Ordering::Acquire) & port.bit != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_uart_irq_sharing_is_correct() {
        assert_eq!(binding(0x3F8).map(|port| port.irq).unwrap_or(0), 4);
        assert_eq!(binding(0x3E8).map(|port| port.irq).unwrap_or(0), 4);
        assert_eq!(binding(0x2F8).map(|port| port.irq).unwrap_or(0), 3);
        assert_eq!(binding(0x2E8).map(|port| port.irq).unwrap_or(0), 3);
        assert!(binding(0x1234).is_none());
    }

    #[test]
    fn every_legacy_uart_has_a_static_rx_queue() {
        assert_eq!(receive_buffer(0x3F8).map(|queue| queue.capacity()).unwrap_or(0), RX_CAPACITY - 1);
        assert_eq!(receive_buffer(0x2F8).map(|queue| queue.capacity()).unwrap_or(0), RX_CAPACITY - 1);
        assert_eq!(receive_buffer(0x3E8).map(|queue| queue.capacity()).unwrap_or(0), RX_CAPACITY - 1);
        assert_eq!(receive_buffer(0x2E8).map(|queue| queue.capacity()).unwrap_or(0), RX_CAPACITY - 1);
        assert!(receive_buffer(0x1234).is_none());
    }
}
