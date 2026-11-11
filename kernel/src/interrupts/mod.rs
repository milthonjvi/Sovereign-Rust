//! CPU exceptions, interrupt descriptor tables, APIC routing, and IRQ dispatch.
//!
//! Architecture-specific entry stubs live under arch; policy and handler
//! registration live here. Interrupt handlers must remain bounded and nonblocking.

pub mod dispatch;
pub mod exceptions;
pub mod idt;
pub mod pic;
