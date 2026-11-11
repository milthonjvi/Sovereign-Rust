//! Interrupt-safe spin lock for data shared by task and interrupt contexts.
//!
//! Every acquisition disables local maskable interrupts before attempting the
//! lock. This prevents an IRQ on the same CPU from preempting a task that owns
//! the lock and then deadlocking while trying to acquire it again. The atomic
//! lock also serializes access across CPUs.

use core::cell::UnsafeCell;
use core::hint::spin_loop;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

const RFLAGS_IF: u64 = 1 << 9;

/// A spin lock that saves and restores the caller's local interrupt-enable state.
pub struct IrqSpinLock<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: Access to the protected value is serialized by the atomic lock.
unsafe impl<T: Send> Sync for IrqSpinLock<T> {}
// SAFETY: Moving the lock transfers exclusive ownership of its protected value.
unsafe impl<T: Send> Send for IrqSpinLock<T> {}

impl<T> IrqSpinLock<T> {
    /// Creates an unlocked lock without performing any hardware operation.
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }

    /// Acquires the lock after disabling local maskable interrupts.
    ///
    /// On guard drop, the lock is released before the prior IF state is restored.
    pub fn lock(&self) -> IrqSpinLockGuard<'_, T> {
        let flags: u64;
        // SAFETY: PUSHFQ reads RFLAGS and CLI is valid in ring 0. This primitive
        // is exclusively for kernel code and must never be called from ring 3.
        unsafe {
            core::arch::asm!(
                "pushfq",
                "pop {flags}",
                "cli",
                flags = out(reg) flags,
                options(nomem)
            );
        }

        while self.locked.compare_exchange_weak(
            false,
            true,
            Ordering::Acquire,
            Ordering::Relaxed,
        ).is_err() {
            while self.locked.load(Ordering::Relaxed) {
                spin_loop();
            }
        }

        IrqSpinLockGuard {
            lock: self,
            restore_interrupts: flags & RFLAGS_IF != 0,
        }
    }

    /// Attempts one acquisition without spinning. Local interrupts are still
    /// disabled for the returned guard's lifetime.
    pub fn try_lock(&self) -> Option<IrqSpinLockGuard<'_, T>> {
        let flags: u64;
        // SAFETY: Same ring-0 contract as lock().
        unsafe {
            core::arch::asm!(
                "pushfq",
                "pop {flags}",
                "cli",
                flags = out(reg) flags,
                options(nomem)
            );
        }

        if self.locked.compare_exchange(
            false,
            true,
            Ordering::Acquire,
            Ordering::Relaxed,
        ).is_ok() {
            Some(IrqSpinLockGuard {
                lock: self,
                restore_interrupts: flags & RFLAGS_IF != 0,
            })
        } else {
            // Restore IF immediately because no guard will own this transition.
            if flags & RFLAGS_IF != 0 {
                // SAFETY: IF was set on entry and remains clear since CLI above.
                unsafe { core::arch::asm!("sti", options(nomem, nostack)); }
            }
            None
        }
    }

    /// Returns true when another context currently owns the lock.
    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }

    /// Returns a shared reference to the protected value.
    ///
    /// # Safety
    /// The caller must guarantee exclusive access for the entire reference
    /// lifetime, including against interrupt handlers and other CPUs.
    pub unsafe fn get_unchecked(&self) -> &T {
        // SAFETY: The caller explicitly guarantees exclusive access.
        unsafe { &*self.value.get() }
    }
}

/// RAII lock guard. Local interrupts are restored only after unlocking.
pub struct IrqSpinLockGuard<'a, T> {
    lock: &'a IrqSpinLock<T>,
    restore_interrupts: bool,
}

impl<T> Deref for IrqSpinLockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: The guard owns the lock for its full lifetime.
        unsafe { &*self.lock.value.get() }
    }
}

impl<T> DerefMut for IrqSpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: The guard owns the lock for its full lifetime.
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T> Drop for IrqSpinLockGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
        if self.restore_interrupts {
            // SAFETY: This guard disabled IF on acquisition and restores it
            // only after releasing the lock. STI is valid in ring 0.
            unsafe { core::arch::asm!("sti", options(nomem, nostack)); }
        }
    }
}
