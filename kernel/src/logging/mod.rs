//! Kernel diagnostics and log routing.
//!
//! Early boot output should use a minimal serial/framebuffer sink. Logging must
//! not allocate or block in interrupt context.
