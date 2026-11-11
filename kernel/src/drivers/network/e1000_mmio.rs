//! Bounds-checked volatile MMIO accessor for an already-mapped E1000 BAR0 window.
//!
//! The platform owns PCI BAR mapping, cache attributes, and lifetime. This
//! wrapper performs only bounded register accesses and x86 coherent-DMA fences.

use core::ptr::NonNull;
use core::sync::atomic::{compiler_fence, fence, Ordering};

use super::e1000::REG_RAH0;
use super::e1000_hw::E1000Registers;

/// Minimum BAR window needed for the registers used by the E1000 driver.
pub const E1000_MMIO_MIN_LENGTH: usize = REG_RAH0 as usize + core::mem::size_of::<u32>();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum E1000MmioError {
    NullBase,
    MisalignedBase,
    WindowTooSmall,
    AddressRangeOverflow,
    MisalignedOffset,
    OutOfBounds,
}

/// Volatile accessor over a platform-mapped, uncached/device-memory BAR window.
///
/// # Safety contract
/// The supplied pointer must refer to the E1000 BAR0 mapping, remain mapped for
/// this object's lifetime, and use a device-safe cache attribute (UC on x86).
/// The mapping must not be concurrently unmapped or aliased by another mutable
/// accessor. DMA memory must be coherent with this CPU or maintained by the
/// platform's DMA allocator before queue operations.
pub struct VolatileE1000Registers {
    base: NonNull<u8>,
    length: usize,
}

impl VolatileE1000Registers {
    /// Wrap an existing MMIO mapping. This does not map or own the BAR.
    ///
    /// # Safety
    /// The supplied region must be a valid, live E1000 BAR0 MMIO mapping with
    /// device/uncached attributes and exclusive mutable access for this wrapper.
    pub unsafe fn new(base: *mut u8, length: usize) -> Result<Self, E1000MmioError> {
        let base = NonNull::new(base).ok_or(E1000MmioError::NullBase)?;
        if base.as_ptr() as usize & 0x3 != 0 {
            return Err(E1000MmioError::MisalignedBase);
        }
        if length < E1000_MMIO_MIN_LENGTH {
            return Err(E1000MmioError::WindowTooSmall);
        }
        (base.as_ptr() as usize).checked_add(length)
            .ok_or(E1000MmioError::AddressRangeOverflow)?;
        if length > isize::MAX as usize {
            return Err(E1000MmioError::AddressRangeOverflow);
        }
        Ok(Self { base, length })
    }

    pub const fn length(&self) -> usize {
        self.length
    }

    fn register_pointer(&self, offset: u32) -> Result<*mut u32, E1000MmioError> {
        let offset = offset as usize;
        if offset & 0x3 != 0 {
            return Err(E1000MmioError::MisalignedOffset);
        }
        let end = offset.checked_add(core::mem::size_of::<u32>())
            .ok_or(E1000MmioError::OutOfBounds)?;
        if end > self.length {
            return Err(E1000MmioError::OutOfBounds);
        }
        // SAFETY: constructor contract guarantees the full mapping remains live;
        // bounds above keep the register access inside that mapping.
        Ok(unsafe { self.base.as_ptr().add(offset).cast::<u32>() })
    }
}

unsafe impl E1000Registers for VolatileE1000Registers {
    type Error = E1000MmioError;

    fn read32(&mut self, offset: u32) -> Result<u32, Self::Error> {
        let register = self.register_pointer(offset)?;
        // SAFETY: register_pointer validates alignment and mapping bounds.
        Ok(unsafe { core::ptr::read_volatile(register) })
    }

    fn write32(&mut self, offset: u32, value: u32) -> Result<(), Self::Error> {
        let register = self.register_pointer(offset)?;
        // SAFETY: register_pointer validates alignment and mapping bounds.
        unsafe { core::ptr::write_volatile(register, value); }
        Ok(())
    }

    fn dma_write_barrier(&mut self) {
        // x86 coherent DMA plus a UC BAR mapping preserves device ordering.
        compiler_fence(Ordering::Release);
        fence(Ordering::Release);
    }

    fn dma_read_barrier(&mut self) {
        fence(Ordering::Acquire);
        compiler_fence(Ordering::Acquire);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::e1000::REG_STATUS;

    #[repr(align(4096))]
    struct MockMmio([u8; E1000_MMIO_MIN_LENGTH]);

    #[test]
    fn rejects_invalid_mmio_mapping_arguments() {
        assert_eq!(unsafe { VolatileE1000Registers::new(core::ptr::null_mut(), E1000_MMIO_MIN_LENGTH) }.err(),
            Some(E1000MmioError::NullBase));
        let mut bytes = [0u8; E1000_MMIO_MIN_LENGTH + 8];
        assert_eq!(unsafe { VolatileE1000Registers::new(bytes.as_mut_ptr().wrapping_add(1), E1000_MMIO_MIN_LENGTH) }.err(),
            Some(E1000MmioError::MisalignedBase));
        assert_eq!(unsafe { VolatileE1000Registers::new(bytes.as_mut_ptr(), E1000_MMIO_MIN_LENGTH - 1) }.err(),
            Some(E1000MmioError::WindowTooSmall));
    }

    #[test]
    fn performs_bounded_volatile_register_access() {
        let mut memory = MockMmio([0; E1000_MMIO_MIN_LENGTH]);
        // SAFETY: this aligned test buffer models a live MMIO window.
        let mut registers = unsafe {
            VolatileE1000Registers::new(memory.0.as_mut_ptr(), memory.0.len())
        }.unwrap();
        registers.write32(REG_STATUS, 0x1234_5678).unwrap();
        assert_eq!(registers.read32(REG_STATUS), Ok(0x1234_5678));
        assert_eq!(registers.read32(REG_STATUS + 1), Err(E1000MmioError::MisalignedOffset));
        assert_eq!(registers.read32(E1000_MMIO_MIN_LENGTH as u32), Err(E1000MmioError::OutOfBounds));
    }
}
