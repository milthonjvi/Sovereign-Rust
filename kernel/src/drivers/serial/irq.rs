//! 16550 receive IRQ integration with the kernel's legacy PIC dispatcher.
//!
//! COM1/COM3 share IRQ4; COM2/COM4 share IRQ3. One bounded handler drains
//! every attached UART on the shared line. RX bytes are published to separate
//! lock-free SPSC queues for task-context consumers.

use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

use crate::arch::x86_64::io;
use crate::interrupts::init as interrupt_init;

use super::{Uart16550, UartError, UartFifo, UartRxBuffer, UartTxBuffer};

const DATA: u16 = 0;
const LSR: u16 = 5;
const DATA_READY: u8 = 1;
const IER: u16 = 1;
const IIR: u16 = 2;
const MODEM_STATUS: u16 = 6;
const IIR_LINE_STATUS: u8 = 0x06;
const IIR_RECEIVE_DATA: u8 = 0x04;
const IIR_RECEIVE_TIMEOUT: u8 = 0x0C;
const IIR_TRANSMIT_EMPTY: u8 = 0x02;
const IIR_MODEM_STATUS: u8 = 0x00;
const THR_EMPTY: u8 = 1 << 5;
const RX_INTERRUPT_ENABLE: u8 = 1;
const LINE_STATUS_INTERRUPT_ENABLE: u8 = 1 << 2;
const RX_IRQ_ENABLES: u8 = RX_INTERRUPT_ENABLE | LINE_STATUS_INTERRUPT_ENABLE;
const THRE_INTERRUPT_ENABLE: u8 = 1 << 1;
const MODEM_STATUS_INTERRUPT_ENABLE: u8 = 1 << 3;
const CTS_INPUT: u8 = 1 << 4;

#[inline]
const fn transmit_interrupt_mask(cts_enabled: bool, cts_asserted: bool, pending: bool) -> u8 {
    RX_IRQ_ENABLES
        | (if cts_enabled { MODEM_STATUS_INTERRUPT_ENABLE } else { 0 })
        | (if pending && (!cts_enabled || cts_asserted) { THRE_INTERRUPT_ENABLE } else { 0 })
}
const MAX_TX_PER_IRQ: usize = 64;
const MAX_IRQ_CAUSES: usize = 32;
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
static COM1_TX: UartTxBuffer = UartTxBuffer::new();
static COM2_TX: UartTxBuffer = UartTxBuffer::new();
static COM3_TX: UartTxBuffer = UartTxBuffer::new();
static COM4_TX: UartTxBuffer = UartTxBuffer::new();
static COM1_TX_DEPTH: AtomicUsize = AtomicUsize::new(1);
static COM2_TX_DEPTH: AtomicUsize = AtomicUsize::new(1);
static COM3_TX_DEPTH: AtomicUsize = AtomicUsize::new(1);
static COM4_TX_DEPTH: AtomicUsize = AtomicUsize::new(1);
static COM1_TX_STATS: AtomicTxStatistics = AtomicTxStatistics::new();
static COM2_TX_STATS: AtomicTxStatistics = AtomicTxStatistics::new();
static COM3_TX_STATS: AtomicTxStatistics = AtomicTxStatistics::new();
static COM4_TX_STATS: AtomicTxStatistics = AtomicTxStatistics::new();
static COM1_CTS_FLOW: AtomicU8 = AtomicU8::new(0);
static COM2_CTS_FLOW: AtomicU8 = AtomicU8::new(0);
static COM3_CTS_FLOW: AtomicU8 = AtomicU8::new(0);
static COM4_CTS_FLOW: AtomicU8 = AtomicU8::new(0);
static COM1_RTS_WATERMARKS: AtomicUsize = AtomicUsize::new(0);
static COM2_RTS_WATERMARKS: AtomicUsize = AtomicUsize::new(0);
static COM3_RTS_WATERMARKS: AtomicUsize = AtomicUsize::new(0);
static COM4_RTS_WATERMARKS: AtomicUsize = AtomicUsize::new(0);

const MCR: u16 = 4;
const MCR_RTS: u8 = 1 << 1;
const WATERMARK_SHIFT: usize = 16;
const WATERMARK_MASK: usize = 0xFFFF;

#[inline]
const fn encode_rts_watermarks(high: usize, low: usize) -> usize {
    (high << WATERMARK_SHIFT) | low
}
#[inline]
const fn decode_rts_watermarks(encoded: usize) -> (usize, usize) {
    (encoded >> WATERMARK_SHIFT, encoded & WATERMARK_MASK)
}
#[inline]
const fn rts_should_deassert(occupancy: usize, high: usize) -> bool {
    high != 0 && occupancy >= high
}
#[inline]
const fn rts_should_assert(occupancy: usize, low: usize) -> bool {
    occupancy <= low
}
#[inline]
fn drive_rts(base: u16, asserted: bool) {
    // SAFETY: Caller serializes MCR access; preserve all non-RTS output bits.
    let current = unsafe { io::in8(base + MCR) };
    let next = if asserted { current | MCR_RTS } else { current & !MCR_RTS };
    // SAFETY: MCR is the UART modem-control output register.
    unsafe { io::out8(base + MCR, next); }
}

/// Interrupt-cause counters for one UART port.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UartIrqStatistics {
    pub interrupts: usize,
    pub line_status: usize,
    pub receive_data: usize,
    pub receive_timeout: usize,
    pub transmit_empty: usize,
    pub modem_status: usize,
    pub unknown_causes: usize,
    pub budget_exhaustions: usize,
}

struct AtomicIrqStatistics {
    interrupts: AtomicUsize,
    line_status: AtomicUsize,
    receive_data: AtomicUsize,
    receive_timeout: AtomicUsize,
    transmit_empty: AtomicUsize,
    modem_status: AtomicUsize,
    unknown_causes: AtomicUsize,
    budget_exhaustions: AtomicUsize,
}

impl AtomicIrqStatistics {
    const fn new() -> Self {
        Self {
            interrupts: AtomicUsize::new(0),
            line_status: AtomicUsize::new(0),
            receive_data: AtomicUsize::new(0),
            receive_timeout: AtomicUsize::new(0),
            transmit_empty: AtomicUsize::new(0),
            modem_status: AtomicUsize::new(0),
            unknown_causes: AtomicUsize::new(0),
            budget_exhaustions: AtomicUsize::new(0),
        }
    }

    fn snapshot(&self) -> UartIrqStatistics {
        UartIrqStatistics {
            interrupts: self.interrupts.load(Ordering::Relaxed),
            line_status: self.line_status.load(Ordering::Relaxed),
            receive_data: self.receive_data.load(Ordering::Relaxed),
            receive_timeout: self.receive_timeout.load(Ordering::Relaxed),
            transmit_empty: self.transmit_empty.load(Ordering::Relaxed),
            modem_status: self.modem_status.load(Ordering::Relaxed),
            unknown_causes: self.unknown_causes.load(Ordering::Relaxed),
            budget_exhaustions: self.budget_exhaustions.load(Ordering::Relaxed),
        }
    }
}

static COM1_IRQ_STATS: AtomicIrqStatistics = AtomicIrqStatistics::new();
static COM2_IRQ_STATS: AtomicIrqStatistics = AtomicIrqStatistics::new();
static COM3_IRQ_STATS: AtomicIrqStatistics = AtomicIrqStatistics::new();
static COM4_IRQ_STATS: AtomicIrqStatistics = AtomicIrqStatistics::new();

/// Transmission counters for one UART port.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UartTxStatistics {
    pub queued_bytes: usize,
    pub transmitted_bytes: usize,
    pub queue_full_events: usize,
}

struct AtomicTxStatistics { queued: AtomicUsize, transmitted: AtomicUsize, full: AtomicUsize }
impl AtomicTxStatistics {
    const fn new() -> Self { Self { queued: AtomicUsize::new(0), transmitted: AtomicUsize::new(0), full: AtomicUsize::new(0) } }
    fn snapshot(&self) -> UartTxStatistics { UartTxStatistics { queued_bytes: self.queued.load(Ordering::Relaxed), transmitted_bytes: self.transmitted.load(Ordering::Relaxed), queue_full_events: self.full.load(Ordering::Relaxed) } }
}

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
    TransmitPending,
    InvalidWatermarks,
}

struct PortBinding {
    irq: u8,
    bit: u8,
    shared_line_mask: u8,
    handler: extern "C" fn(u8),
    queue: &'static UartRxBuffer<RX_CAPACITY>,
    stats: &'static AtomicRxStatistics,
    tx_queue: &'static UartTxBuffer,
    tx_depth: &'static AtomicUsize,
    tx_stats: &'static AtomicTxStatistics,
    irq_stats: &'static AtomicIrqStatistics,
    cts_flow: &'static AtomicU8,
    rts_watermarks: &'static AtomicUsize,
}

fn binding(base: u16) -> Option<PortBinding> {
    match base {
        0x3F8 => Some(PortBinding {
            irq: 4, bit: COM1_BIT, shared_line_mask: IRQ4_PORTS,
            handler: irq4_handler, queue: &COM1_RX,
            stats: &COM1_STATS, tx_queue: &COM1_TX, tx_depth: &COM1_TX_DEPTH, tx_stats: &COM1_TX_STATS, irq_stats: &COM1_IRQ_STATS, cts_flow: &COM1_CTS_FLOW, rts_watermarks: &COM1_RTS_WATERMARKS,
        }),
        0x2F8 => Some(PortBinding {
            irq: 3, bit: COM2_BIT, shared_line_mask: IRQ3_PORTS,
            handler: irq3_handler, queue: &COM2_RX,
            stats: &COM2_STATS, tx_queue: &COM2_TX, tx_depth: &COM2_TX_DEPTH, tx_stats: &COM2_TX_STATS, irq_stats: &COM2_IRQ_STATS, cts_flow: &COM2_CTS_FLOW, rts_watermarks: &COM2_RTS_WATERMARKS,
        }),
        0x3E8 => Some(PortBinding {
            irq: 4, bit: COM3_BIT, shared_line_mask: IRQ4_PORTS,
            handler: irq4_handler, queue: &COM3_RX,
            stats: &COM3_STATS, tx_queue: &COM3_TX, tx_depth: &COM3_TX_DEPTH, tx_stats: &COM3_TX_STATS, irq_stats: &COM3_IRQ_STATS, cts_flow: &COM3_CTS_FLOW, rts_watermarks: &COM3_RTS_WATERMARKS,
        }),
        0x2E8 => Some(PortBinding {
            irq: 3, bit: COM4_BIT, shared_line_mask: IRQ3_PORTS,
            handler: irq3_handler, queue: &COM4_RX,
            stats: &COM4_STATS, tx_queue: &COM4_TX, tx_depth: &COM4_TX_DEPTH, tx_stats: &COM4_TX_STATS, irq_stats: &COM4_IRQ_STATS, cts_flow: &COM4_CTS_FLOW, rts_watermarks: &COM4_RTS_WATERMARKS,
        }),
        _ => None,
    }
}

/// Attach a configured UART to interrupt-driven receive.
///
/// The correct handler is registered while the PIC line is masked, UART RX
/// interrupts are enabled, and the PIC line is unmasked last. The returned
/// queue is consumed from normal kernel context. If automatic RTS is enabled,\n/// consume through receive_byte rather than calling the queue's pop directly.
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
    let depth = match uart.fifo_capability() { UartFifo::None => 1, UartFifo::Bytes16 => 16, UartFifo::Bytes64 => 64 };
    port.tx_depth.store(depth, Ordering::Release);
    if first_on_line {
        // SAFETY: Caller keeps IF clear and guarantees exclusive IRQ3/IRQ4 ownership.
        unsafe { interrupt_init::register_legacy_irq(port.irq, port.handler) }
            .map_err(|error| match error {
                interrupt_init::LegacyIrqError::HandlerAlreadyRegistered =>
                    UartIrqError::HandlerAlreadyOwned,
                _ => UartIrqError::InterruptSubsystem,
            })?;
    }

    port.cts_flow.store(0, Ordering::Release);
    port.rts_watermarks.store(0, Ordering::Release);
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

    if !port.tx_queue.is_empty() {
        return Err(UartIrqError::TransmitPending);
    }
    // Wait for the UART shift register to finish before disabling its IRQ source.
    uart.flush().map_err(UartIrqError::Uart)?;

    let last_on_line = current & port.shared_line_mask == port.bit;
    if last_on_line {
        // SAFETY: IF is clear; this driver owns the registered shared-line handler.
        unsafe { interrupt_init::mask_legacy_irq(port.irq) }
            .map_err(|_| UartIrqError::InterruptSubsystem)?;
    }

    uart.disable_interrupts().map_err(UartIrqError::Uart)?;
    ATTACHED.fetch_and(!port.bit, Ordering::AcqRel);
    port.cts_flow.store(0, Ordering::Release);
    port.rts_watermarks.store(0, Ordering::Release);

    if last_on_line {
        // SAFETY: Source is masked and IF is clear; no callback is in flight.
        unsafe { interrupt_init::unregister_legacy_irq(port.irq, port.handler) }
            .map_err(|_| UartIrqError::InterruptSubsystem)?;
    }
    Ok(())
}

/// Queue bytes for interrupt-driven transmission on an attached UART.
///
/// Returns the accepted prefix length; the caller must retry any suffix.
///
/// # Safety
/// Call on the CPU that services this legacy PIC IRQ, with IF clear, and with
/// exactly one serialized producer per UART queue. Pinning producer and IRQ to
/// the same CPU prevents a lost wakeup while the handler disables THRE.
/// The UART must already be attached through `attach_interrupts`.
pub unsafe fn transmit_async(uart: &Uart16550, bytes: &[u8]) -> Result<usize, UartIrqError> {
    uart.ensure_initialized().map_err(UartIrqError::Uart)?;
    let port = binding(uart.base_port()).ok_or(UartIrqError::UnsupportedPort)?;
    if ATTACHED.load(Ordering::Acquire) & port.bit == 0 { return Err(UartIrqError::NotAttached); }
    // SAFETY: Caller guarantees the SPSC producer contract.
    let accepted = unsafe { port.tx_queue.push_slice(bytes) };
    if accepted != 0 {
        // CTS mode also keeps modem-status IRQ enabled so a later CTS assertion
        // can resume the queued bytes without polling.
        let cts_enabled = port.cts_flow.load(Ordering::Acquire) != 0;
        // Avoid reading MSR when CTS gating is disabled: that read clears
        // modem delta flags even though this path does not own modem IRQs.
        // SAFETY: Caller owns the UART and IF is clear by this API contract.
        let cts_asserted = !cts_enabled
            || unsafe { io::in8(uart.base_port() + MODEM_STATUS) } & CTS_INPUT != 0;
        let ier = transmit_interrupt_mask(cts_enabled, cts_asserted, true);
        // SAFETY: Exclusive UART ownership is held by the caller.
        unsafe { io::out8(uart.base_port() + IER, ier); }
    }
    port.tx_stats.queued.fetch_add(accepted, Ordering::Relaxed);
    if accepted < bytes.len() { port.tx_stats.full.fetch_add(1, Ordering::Relaxed); }
    Ok(accepted)
}

/// Enables or disables hardware CTS-gated asynchronous transmission.
///
/// When enabled, THRE interrupts are suspended while CTS is deasserted and
/// resumed by the modem-status interrupt when CTS becomes asserted. RX and
/// line-status interrupts remain active.
///
/// # Safety
/// Call with IF clear on the CPU servicing this UART's PIC IRQ. The caller must
/// serialize this operation with the single TX producer and retain exclusive
/// ownership of the UART registers.
pub unsafe fn set_cts_flow_control(
    uart: &Uart16550,
    enabled: bool,
) -> Result<(), UartIrqError> {
    uart.ensure_initialized().map_err(UartIrqError::Uart)?;
    let port = binding(uart.base_port()).ok_or(UartIrqError::UnsupportedPort)?;
    if ATTACHED.load(Ordering::Acquire) & port.bit == 0 {
        return Err(UartIrqError::NotAttached);
    }

    port.cts_flow.store(enabled as u8, Ordering::Release);
    // Reading MSR acknowledges any stale modem delta before enabling its IRQ.
    // SAFETY: Caller owns this UART and executes with IF clear.
    let modem = unsafe { io::in8(uart.base_port() + MODEM_STATUS) };
    let ier = transmit_interrupt_mask(
        enabled,
        modem & CTS_INPUT != 0,
        !port.tx_queue.is_empty(),
    );
    // SAFETY: Caller owns the UART and serializes access to IER.
    unsafe { io::out8(uart.base_port() + IER, ier); }
    Ok(())
}


/// Configures automatic receive backpressure using the UART RTS output.
///
/// The IRQ producer deasserts RTS at/above high_watermark. The sole consumer
/// must use receive_byte, which reasserts RTS at/below low_watermark.
/// Set both thresholds to zero to disable automatic RTS and assert RTS.
/// Otherwise: 1 <= high_watermark <= queue capacity and low_watermark < high.
///
/// # Safety
/// Call with IF clear on the CPU servicing this UART IRQ. Serialize MCR access;
/// ensure the sole RX consumer uses receive_byte, not direct queue.pop. Do not
/// concurrently call set_rts while automatic RTS is enabled.
pub unsafe fn set_auto_rts_flow_control(
    uart: &Uart16550,
    high_watermark: usize,
    low_watermark: usize,
) -> Result<(), UartIrqError> {
    uart.ensure_initialized().map_err(UartIrqError::Uart)?;
    let port = binding(uart.base_port()).ok_or(UartIrqError::UnsupportedPort)?;
    if ATTACHED.load(Ordering::Acquire) & port.bit == 0 {
        return Err(UartIrqError::NotAttached);
    }
    let capacity = port.queue.capacity();
    let disabled = high_watermark == 0 && low_watermark == 0;
    if !disabled && (high_watermark == 0 || high_watermark > capacity || low_watermark >= high_watermark) {
        return Err(UartIrqError::InvalidWatermarks);
    }
    if disabled {
        port.rts_watermarks.store(0, Ordering::Release);
        drive_rts(uart.base_port(), true);
    } else {
        port.rts_watermarks.store(encode_rts_watermarks(high_watermark, low_watermark), Ordering::Release);
        drive_rts(uart.base_port(), !rts_should_deassert(port.queue.len(), high_watermark));
    }
    Ok(())
}

/// Removes one byte and services the low-watermark RTS transition.
///
/// # Safety
/// Exactly one consumer may call this function. Call with IF clear on the CPU
/// servicing this UART IRQ so the MCR update cannot race the IRQ producer.
pub unsafe fn receive_byte(base: u16) -> Option<u8> {
    let port = binding(base)?;
    // SAFETY: The caller upholds the SPSC single-consumer contract.
    let byte = unsafe { port.queue.pop() };
    if byte.is_some() {
        let encoded = port.rts_watermarks.load(Ordering::Acquire);
        if encoded != 0 {
            let (_, low) = decode_rts_watermarks(encoded);
            if rts_should_assert(port.queue.len(), low) {
                drive_rts(base, true);
            }
        }
    }
    byte
}

/// Returns the number of RX bytes dropped because the software queue was full.
pub fn receive_dropped_bytes(base: u16) -> Option<usize> {
    binding(base).map(|port| port.queue.dropped_bytes())
}

/// Returns a snapshot of transmit counters for a legacy UART.
pub fn transmit_statistics(base: u16) -> Option<UartTxStatistics> {
    binding(base).map(|port| port.tx_stats.snapshot())
}

/// Returns the number of bytes waiting in a legacy UART's TX queue.
pub fn transmit_pending(base: u16) -> Option<usize> {
    binding(base).map(|port| port.tx_queue.len())
}

/// Returns the static RX queue associated with a legacy UART base port.\n/// Do not consume it directly while automatic RTS is enabled; use receive_byte\n/// so the low-watermark transition can reassert the peer's transmit permission.
pub fn receive_buffer(base: u16) -> Option<&'static UartRxBuffer<RX_CAPACITY>> {
    binding(base).map(|port| port.queue)
}

/// Returns a snapshot of hardware line errors counted by the RX interrupt path.
pub fn receive_statistics(base: u16) -> Option<UartRxStatistics> {
    binding(base).map(|port| port.stats.snapshot())
}

/// Returns a snapshot of interrupt-cause counters for a legacy UART.
pub fn interrupt_statistics(base: u16) -> Option<UartIrqStatistics> {
    binding(base).map(|port| port.irq_stats.snapshot())
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
        let accepted = unsafe { queue.push(byte) };
        if accepted {
            let encoded = binding(base).map(|port| port.rts_watermarks.load(Ordering::Acquire)).unwrap_or(0);
            if encoded != 0 {
                let (high, _) = decode_rts_watermarks(encoded);
                if rts_should_deassert(queue.len(), high) {
                    drive_rts(base, false);
                }
            }
        }
    }
}

fn service_transmit(
    base: u16,
    queue: &'static UartTxBuffer,
    depth: &'static AtomicUsize,
    stats: &'static AtomicTxStatistics,
    cts_flow: &'static AtomicU8,
) {
    // A deasserted CTS must stop the TX engine and leave modem IRQ armed;
    // otherwise a level-triggered THRE source would continuously retrigger.
    if cts_flow.load(Ordering::Acquire) != 0 {
        // SAFETY: This shared handler owns the attached UART's registers.
        if unsafe { io::in8(base + MODEM_STATUS) } & CTS_INPUT == 0 {
            // SAFETY: Preserve RX and modem-status interrupts, suppress THRE.
            unsafe { io::out8(base + IER, transmit_interrupt_mask(true, false, false)); }
            return;
        }
    }
    // THRE means the transmit FIFO can accept one hardware-sized burst.
    // SAFETY: This shared handler owns the UART's registers.
    if unsafe { io::in8(base + LSR) } & THR_EMPTY == 0 {
        return;
    }
    let limit = core::cmp::min(depth.load(Ordering::Acquire), MAX_TX_PER_IRQ);
    let mut sent = 0;
    while sent < limit {
        // SAFETY: IRQ is the sole consumer of this SPSC queue.
        let Some(byte) = (unsafe { queue.pop() }) else { break; };
        // SAFETY: THRE reports that the transmitter can accept this FIFO burst.
        unsafe { io::out8(base + DATA, byte); }
        sent += 1;
    }
    stats.transmitted.fetch_add(sent, Ordering::Relaxed);
    if queue.is_empty() {
        // Disable THRE only; RX-ready and line-status IRQs remain enabled.
        // SAFETY: RX_IRQ_ENABLES preserves both receive interrupt sources.
        let cts_enabled = cts_flow.load(Ordering::Acquire) != 0;
        // SAFETY: Preserve RX and optional CTS-change interrupts.
        unsafe { io::out8(base + IER, transmit_interrupt_mask(cts_enabled, true, false)); }
    }
}

fn service_port(
    base: u16,
    rx: &'static UartRxBuffer<RX_CAPACITY>,
    rx_stats: &'static AtomicRxStatistics,
    tx: &'static UartTxBuffer,
    tx_depth: &'static AtomicUsize,
    tx_stats: &'static AtomicTxStatistics,
    irq_stats: &'static AtomicIrqStatistics,
    cts_flow: &'static AtomicU8,
) {
    // A shared IRQ may represent several UART causes. IIR reports the highest
    // priority pending cause; acknowledge it and re-read until quiescent.
    let mut handled = 0;
    while handled < MAX_IRQ_CAUSES {
        // SAFETY: The registered handler owns this UART's interrupt registers.
        let iir = unsafe { io::in8(base + IIR) };
        if iir & 1 != 0 {
            return;
        }
        handled += 1;
        irq_stats.interrupts.fetch_add(1, Ordering::Relaxed);
        match iir & 0x0E {
            IIR_LINE_STATUS => {
                irq_stats.line_status.fetch_add(1, Ordering::Relaxed);
                drain_port(base, rx, rx_stats);
            }
            IIR_RECEIVE_DATA => {
                irq_stats.receive_data.fetch_add(1, Ordering::Relaxed);
                drain_port(base, rx, rx_stats);
            }
            IIR_RECEIVE_TIMEOUT => {
                irq_stats.receive_timeout.fetch_add(1, Ordering::Relaxed);
                drain_port(base, rx, rx_stats);
            }
            IIR_TRANSMIT_EMPTY => {
                irq_stats.transmit_empty.fetch_add(1, Ordering::Relaxed);
                service_transmit(base, tx, tx_depth, tx_stats, cts_flow);
            }
            IIR_MODEM_STATUS => {
                irq_stats.modem_status.fetch_add(1, Ordering::Relaxed);
                // Reading MSR clears delta-CTS/DSR/RI/DCD and deasserts this source.
                // If CTS rose, resume queued transmission; if it fell, keep THRE
                // disabled until the next modem-status transition.
                // SAFETY: The handler exclusively owns the attached UART.
                let modem = unsafe { io::in8(base + MODEM_STATUS) };
                if cts_flow.load(Ordering::Acquire) != 0 {
                    let ier = transmit_interrupt_mask(
                        true,
                        modem & CTS_INPUT != 0,
                        !tx.is_empty(),
                    );
                    // SAFETY: Shared handler has exclusive register access.
                    unsafe { io::out8(base + IER, ier); }
                }
            }
            _ => {
                irq_stats.unknown_causes.fetch_add(1, Ordering::Relaxed);
                // Unknown/reserved IIR cause: read status registers once to
                // acknowledge any implementation-specific latched condition.
                // SAFETY: The handler exclusively owns the attached UART.
                let _ = unsafe { io::in8(base + LSR) };
                let _ = unsafe { io::in8(base + MODEM_STATUS) };
            }
        }
    }

    // If the cause budget was consumed and IIR still reports pending work,
    // leave the source enabled; the level-triggered UART IRQ will retrigger.
    // SAFETY: The registered handler owns this UART's interrupt registers.
    if unsafe { io::in8(base + IIR) } & 1 == 0 {
        irq_stats.budget_exhaustions.fetch_add(1, Ordering::Relaxed);
    }
}

extern "C" fn irq3_handler(_vector: u8) {
    let attached = ATTACHED.load(Ordering::Acquire);
    if attached & COM2_BIT != 0 {
        service_port(0x2F8, &COM2_RX, &COM2_STATS, &COM2_TX, &COM2_TX_DEPTH, &COM2_TX_STATS, &COM2_IRQ_STATS, &COM2_CTS_FLOW);
    }
    if attached & COM4_BIT != 0 {
        service_port(0x2E8, &COM4_RX, &COM4_STATS, &COM4_TX, &COM4_TX_DEPTH, &COM4_TX_STATS, &COM4_IRQ_STATS, &COM4_CTS_FLOW);
    }
}

extern "C" fn irq4_handler(_vector: u8) {
    let attached = ATTACHED.load(Ordering::Acquire);
    if attached & COM1_BIT != 0 {
        service_port(0x3F8, &COM1_RX, &COM1_STATS, &COM1_TX, &COM1_TX_DEPTH, &COM1_TX_STATS, &COM1_IRQ_STATS, &COM1_CTS_FLOW);
    }
    if attached & COM3_BIT != 0 {
        service_port(0x3E8, &COM3_RX, &COM3_STATS, &COM3_TX, &COM3_TX_DEPTH, &COM3_TX_STATS, &COM3_IRQ_STATS, &COM3_CTS_FLOW);
    }
}

/// Reports whether a UART port is currently connected to its shared IRQ line.
pub(super) fn is_attached(base: u16) -> bool {
    binding(base).is_some_and(|port| ATTACHED.load(Ordering::Acquire) & port.bit != 0)
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn rts_watermarks_encode_and_decode_without_loss() {
        assert_eq!(decode_rts_watermarks(encode_rts_watermarks(900, 300)), (900, 300));
    }

    #[test]
    fn rts_hysteresis_uses_inclusive_thresholds() {
        assert!(!rts_should_deassert(899, 900));
        assert!(rts_should_deassert(900, 900));
        assert!(!rts_should_assert(301, 300));
        assert!(rts_should_assert(300, 300));
        assert!(rts_should_assert(0, 300));
        assert!(!rts_should_deassert(1023, 0));
    }

    #[test]
    fn every_legacy_uart_exposes_independent_interrupt_statistics() {
        for base in [0x3F8, 0x2F8, 0x3E8, 0x2E8] {
            assert_eq!(interrupt_statistics(base), Some(UartIrqStatistics::default()));
        }
        assert!(interrupt_statistics(0x1234).is_none());
    }

    #[test]
    fn interrupt_statistics_snapshot_preserves_each_counter() {
        let stats = AtomicIrqStatistics::new();
        stats.interrupts.store(8, Ordering::Relaxed);
        stats.line_status.store(1, Ordering::Relaxed);
        stats.receive_data.store(2, Ordering::Relaxed);
        stats.receive_timeout.store(3, Ordering::Relaxed);
        stats.transmit_empty.store(4, Ordering::Relaxed);
        stats.modem_status.store(5, Ordering::Relaxed);
        stats.unknown_causes.store(6, Ordering::Relaxed);
        stats.budget_exhaustions.store(7, Ordering::Relaxed);
        assert_eq!(stats.snapshot(), UartIrqStatistics {
            interrupts: 8,
            line_status: 1,
            receive_data: 2,
            receive_timeout: 3,
            transmit_empty: 4,
            modem_status: 5,
            unknown_causes: 6,
            budget_exhaustions: 7,
        });
    }

    #[test]
    fn line_status_statistics_decode_each_hardware_error_bit() {
        let stats = AtomicRxStatistics::new();
        stats.record_lsr(
            OVERRUN_ERROR | PARITY_ERROR | FRAMING_ERROR | BREAK_INTERRUPT | FIFO_ERROR
        );
        assert_eq!(stats.snapshot(), UartRxStatistics {
            overrun_errors: 1,
            parity_errors: 1,
            framing_errors: 1,
            break_interrupts: 1,
            fifo_errors: 1,
        });
    }

    #[test]
    fn iir_cause_ids_match_16550_priority_encoding() {
        assert_eq!(IIR_LINE_STATUS & 0x0E, 0x06);
        assert_eq!(IIR_RECEIVE_DATA & 0x0E, 0x04);
        assert_eq!(IIR_RECEIVE_TIMEOUT & 0x0E, 0x0C);
        assert_eq!(IIR_TRANSMIT_EMPTY & 0x0E, 0x02);
        assert_eq!(IIR_MODEM_STATUS & 0x0E, 0x00);
    }

    #[test]
    fn cts_flow_control_masks_preserve_receive_sources() {
        assert_eq!(transmit_interrupt_mask(true, false, false), 0b0000_1101);
        assert_eq!(transmit_interrupt_mask(true, true, true), 0b0000_1111);
        assert_eq!(transmit_interrupt_mask(true, false, true), 0b0000_1101);
        assert_eq!(transmit_interrupt_mask(false, false, true), 0b0000_0111);
        assert_eq!(transmit_interrupt_mask(false, false, false), RX_IRQ_ENABLES);
    }

    #[test]
    fn cts_input_uses_modem_status_bit_four() {
        assert_eq!(CTS_INPUT, 0x10);
        assert_eq!(0x10 & CTS_INPUT, CTS_INPUT);
        assert_eq!(0x00 & CTS_INPUT, 0);
    }

    #[test]
    fn receive_irq_mask_keeps_line_errors_enabled_without_modem_irqs() {
        assert_eq!(RX_IRQ_ENABLES, 0b0000_0101);
        assert_eq!(RX_IRQ_ENABLES & THRE_INTERRUPT_ENABLE, 0);
        // IER bit 3 is modem-status interrupt enable and must remain clear.
        assert_eq!(RX_IRQ_ENABLES & (1 << 3), 0);
        assert_eq!(RX_IRQ_ENABLES | THRE_INTERRUPT_ENABLE, 0b0000_0111);
    }

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

    #[test]
    fn every_legacy_uart_exposes_independent_tx_statistics() {
        for base in [0x3F8, 0x2F8, 0x3E8, 0x2E8] {
            assert_eq!(transmit_statistics(base), Some(UartTxStatistics::default()));
            assert_eq!(receive_dropped_bytes(base), Some(0));
            assert_eq!(receive_high_watermark(base), Some(0));
        }
        assert!(transmit_statistics(0x1234).is_none());
        assert!(receive_dropped_bytes(0x1234).is_none());
        assert!(receive_high_watermark(0x1234).is_none());
    }

    #[test]
    fn every_legacy_uart_exposes_independent_error_statistics() {
        for base in [0x3F8, 0x2F8, 0x3E8, 0x2E8] {
            assert_eq!(receive_statistics(base), Some(UartRxStatistics::default()));
        }
        assert!(receive_statistics(0x1234).is_none());
    }
}
