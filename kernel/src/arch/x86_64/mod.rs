//! x86_64-specific CPU support.
//!
//! Keep privileged instructions and register-level operations isolated here.
//! Prefer Rust intrinsics and inline assembly only where the architecture
//! requires it.
