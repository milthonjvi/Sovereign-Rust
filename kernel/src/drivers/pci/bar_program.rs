//! Transactional PCI BAR programming from validated resource allocations.
//!
//! Resource assignment and platform aperture discovery are separate. This
//! module writes only the BAR selected by the caller and restores the original
//! BAR values if programming or read-back validation fails.

use super::bar_probe::BarSize;
use super::config::{read_bars, Bar, BarKind, ConfigAccess, ConfigError, PciFunction};
use super::resource_allocator::ResourceAllocation;
use super::resources::{BarResource, ResourceError};

const COMMAND_STATUS: u16 = 0x04;
const COMMAND_IO_SPACE: u16 = 1 << 0;
const COMMAND_MEMORY_SPACE: u16 = 1 << 1;
const BAR0: u16 = 0x10;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BarProgramError {
    Access(ConfigError),
    BarNotFound,
    TypeMismatch,
    AllocationMismatch,
    InvalidResource(ResourceError),
    ReadbackMismatch,
    RestoreFailed,
}

/// Program one BAR from a previously measured and reserved allocation.
///
/// The routine disables I/O and memory decoding while changing the BAR, writes
/// the upper dword first for 64-bit BARs, verifies the programmed base, and
/// restores the original command bits. PCI Status is never echoed into its
/// write-one-to-clear register half.
///
/// # Safety
/// The function must be quiescent and exclusively owned by the caller. No
/// driver, other CPU, or DMA activity may use it during the operation. The
/// supplied allocation must be reserved exclusively for this function and
/// must fit the host-bridge and every upstream bridge aperture. `measured_size`
/// must come from a successful probe of this exact function and BAR.
pub unsafe fn program_bar<A: ConfigAccess>(
    io: &A,
    function: PciFunction,
    allocation: ResourceAllocation,
    measured_size: BarSize,
) -> Result<Bar, BarProgramError> {
    if allocation.address != function.address
        || measured_size.index != allocation.bar_index
        || measured_size.size != allocation.size
    {
        return Err(BarProgramError::AllocationMismatch);
    }

    let mut bars = [Bar {
        index: 0, kind: BarKind::Memory32, prefetchable: false,
        base: 0, raw_low: 0, raw_high: None,
    }; 6];
    let count = read_bars(io, function, &mut bars).map_err(BarProgramError::Access)?;
    let original = bars[..count].iter().find(|bar| bar.index == allocation.bar_index)
        .copied().ok_or(BarProgramError::BarNotFound)?;
    if original.kind != allocation.kind || original.prefetchable != allocation.prefetchable {
        return Err(BarProgramError::TypeMismatch);
    }

    let validated = BarResource::new(
        Bar {
            base: allocation.base,
            raw_low: original.raw_low,
            raw_high: original.raw_high,
            ..original
        },
        allocation.size,
    ).map_err(BarProgramError::InvalidResource)?;
    if validated.end != allocation.end {
        return Err(BarProgramError::AllocationMismatch);
    }

    let command_status = io.read_u32(function.address, COMMAND_STATUS)
        .map_err(BarProgramError::Access)?;
    let original_command = command_status as u16;
    let disable_decode = (original_command & !(COMMAND_IO_SPACE | COMMAND_MEMORY_SPACE)) as u32;
    if let Err(error) = io.write_u32(function.address, COMMAND_STATUS, disable_decode) {
        return if io.write_u32(function.address, COMMAND_STATUS, original_command as u32).is_err() {
            Err(BarProgramError::RestoreFailed)
        } else {
            Err(BarProgramError::Access(error))
        };
    }

    let low_register = BAR0 + (allocation.bar_index as u16) * 4;
    let is_64 = original.kind == BarKind::Memory64;
    let new_low = match original.kind {
        BarKind::Io => (allocation.base as u32 & !0x3) | (original.raw_low & 0x3),
        BarKind::MemoryBelow1M => (allocation.base as u32 & 0x000f_fff0) | (original.raw_low & 0xf),
        BarKind::Memory32 | BarKind::Memory64 => (allocation.base as u32 & !0xf) | (original.raw_low & 0xf),
    };
    let new_high = (allocation.base >> 32) as u32;

    let mut operation_error = None;
    if is_64 {
        if let Err(error) = io.write_u32(function.address, low_register + 4, new_high) {
            operation_error = Some(BarProgramError::Access(error));
        }
    }
    if operation_error.is_none() {
        if let Err(error) = io.write_u32(function.address, low_register, new_low) {
            operation_error = Some(BarProgramError::Access(error));
        }
    }
    if operation_error.is_none() {
        let read_low = io.read_u32(function.address, low_register).map_err(BarProgramError::Access);
        let read_high = if is_64 {
            io.read_u32(function.address, low_register + 4).map(Some).map_err(BarProgramError::Access)
        } else {
            Ok(None)
        };
        match (read_low, read_high) {
            (Ok(low), Ok(high)) => match Bar::decode(allocation.bar_index, low, high) {
                Some(actual) if actual.kind == allocation.kind
                    && actual.prefetchable == allocation.prefetchable
                    && actual.base == allocation.base => {},
                _ => operation_error = Some(BarProgramError::ReadbackMismatch),
            },
            (Err(error), _) | (_, Err(error)) => operation_error = Some(error),
        }
    }

    if operation_error.is_some() {
        let mut restore_failed = false;
        if let Some(high) = original.raw_high {
            if io.write_u32(function.address, low_register + 4, high).is_err() {
                restore_failed = true;
            }
        }
        if io.write_u32(function.address, low_register, original.raw_low).is_err() {
            restore_failed = true;
        }
        if restore_failed {
            return Err(BarProgramError::RestoreFailed);
        }
    }

    if io.write_u32(function.address, COMMAND_STATUS, original_command as u32).is_err() {
        return Err(BarProgramError::RestoreFailed);
    }
    if let Some(error) = operation_error {
        return Err(error);
    }

    Bar::decode(allocation.bar_index, new_low, if is_64 { Some(new_high) } else { None })
        .ok_or(BarProgramError::ReadbackMismatch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::RefCell;
    use super::super::config::{PciAddress, PciIdentity};

    struct MockConfig { registers: RefCell<[u32; 64]> }
    impl ConfigAccess for MockConfig {
        fn read_u32(&self, _address: PciAddress, register: u16) -> Result<u32, ConfigError> {
            self.registers.borrow().get((register / 4) as usize).copied().ok_or(ConfigError::OutOfRange)
        }
        fn write_u32(&self, _address: PciAddress, register: u16, value: u32) -> Result<(), ConfigError> {
            let mut registers = self.registers.borrow_mut();
            let slot = registers.get_mut((register / 4) as usize).ok_or(ConfigError::OutOfRange)?;
            if register == COMMAND_STATUS {
                let old_status = *slot >> 16;
                *slot = ((old_status & !(value >> 16)) << 16) | (value & 0xffff);
            } else { *slot = value; }
            Ok(())
        }
    }

    fn function() -> PciFunction {
        PciFunction {
            address: PciAddress { segment: 0, bus: 0, device: 1, function: 0 },
            identity: PciIdentity {
                vendor_id: 1, device_id: 2, command: 7, status: 0,
                revision: 0, programming_interface: 0, subclass: 0, class_code: 0,
                header_type: 0,
            },
        }
    }

    #[test]
    fn programs_32_bit_bar_and_preserves_command_status() {
        let mut registers = [0u32; 64];
        registers[1] = (0xa55a << 16) | 7;
        registers[4] = 0x8000_0008;
        let io = MockConfig { registers: RefCell::new(registers) };
        let allocation = ResourceAllocation {
            address: function().address,
            bar_index: 0, kind: BarKind::Memory32, prefetchable: true,
            base: 0x9000_0000, size: 0x1000, end: 0x9000_1000,
        };
        let size = BarSize { index: 0, size: 0x1000 };
        let result = unsafe { program_bar(&io, function(), allocation, size) }.unwrap();
        assert_eq!(result.base, 0x9000_0000);
        let registers = io.registers.borrow();
        assert_eq!(registers[4], 0x9000_0008);
        assert_eq!(registers[1] as u16, 7);
        assert_eq!(registers[1] >> 16, 0xa55a);
    }

    #[test]
    fn programs_64_bit_bar_as_one_pair() {
        let mut registers = [0u32; 64];
        registers[1] = 3;
        registers[4] = 0x4;
        registers[5] = 1;
        let io = MockConfig { registers: RefCell::new(registers) };
        let allocation = ResourceAllocation {
            address: function().address,
            bar_index: 0, kind: BarKind::Memory64, prefetchable: false,
            base: 0x1_0000_2000, size: 0x2000, end: 0x1_0000_4000,
        };
        let size = BarSize { index: 0, size: 0x2000 };
        let result = unsafe { program_bar(&io, function(), allocation, size) }.unwrap();
        assert_eq!(result.base, allocation.base);
        let registers = io.registers.borrow();
        assert_eq!(registers[4], 0x2004);
        assert_eq!(registers[5], 1);
    }
}
