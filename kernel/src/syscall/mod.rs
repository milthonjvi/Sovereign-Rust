//! User/kernel ABI and system-call dispatch.
//!
//! Define a versioned ABI only after privilege transitions, task isolation, and
//! user address spaces are implemented. Keep syscall numbers centralized.
