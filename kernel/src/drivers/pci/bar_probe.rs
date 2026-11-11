//! Explicit, destructive PCI BAR aperture sizing.
//!
//! Normal enumeration must use `read_bars`; this module is only for a
//! platform-controlled phase where the function is quiescent and exclusively
//! owned by the caller. BAR sizing temporarily disables I/O and memory decode,
//! writes all ones to BAR registers, reads the implemented address mask, and
//! restores every original value before returning.

use super::config::{ConfigAccess, ConfigError, PciAddress, PciFunction};

const COMMAND_STATUS: u16 = 0x04;
const COMMAND_IO_SPACE: u16 = 1 << 0;
const COMMAND_MEMORY_SPACE: u16 = 1 << 1;
const BAR0: u16 = 0x10;
const MAX_BAR_REGISTERS: usize = 6;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BarProbeError {
    Access(ConfigError),
    BufferTooSmall,
    InvalidBar,
    InvalidMask,
    RestoreFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BarSize {
    pub address: PciAddress,
    pub index: u8,
    pub size: u64,
}

/// Destructively determine the implemented aperture size of each BAR.
///
/// `out` is indexed by BAR register number. A 64-bit BAR writes its size at
/// the low register index and leaves the consumed high-register slot `None`;
/// an unimplemented BAR also remains `None`.
///
/// # Safety
/// The caller must guarantee that this PCI function is quiescent, that no
/// driver or other CPU can access it, and that this operation is permitted by
/// the platform. BAR registers are temporarily overwritten. The function must
/// not be actively decoding transactions or participating in DMA while probed.
pub unsafe fn probe_bar_sizes<A: ConfigAccess>(
    io: &A,
    function: PciFunction,
    out: &mut [Option<BarSize>],
) -> Result<usize, BarProbeError> {
    let count = function.identity.bar_register_count() as usize;
    if count > MAX_BAR_REGISTERS || out.len() < count {
        return Err(BarProbeError::BufferTooSmall);
    }

    let mut original = [0u32; MAX_BAR_REGISTERS];
    for (index, slot) in original.iter_mut().enumerate().take(count) {
        *slot = io.read_u32(function.address, BAR0 + (index as u16) * 4)
            .map_err(BarProbeError::Access)?;
    }
    let command_status = io.read_u32(function.address, COMMAND_STATUS)
        .map_err(BarProbeError::Access)?;
    let original_command = command_status as u16;

    // The upper half is PCI Status (write-one-to-clear). Never echo it.
    let disable_decode = (original_command & !(COMMAND_IO_SPACE | COMMAND_MEMORY_SPACE)) as u32;
    if let Err(error) = io.write_u32(function.address, COMMAND_STATUS, disable_decode) {
        return if io.write_u32(function.address, COMMAND_STATUS, original_command as u32).is_err() {
            Err(BarProbeError::RestoreFailed)
        } else {
            Err(BarProbeError::Access(error))
        };
    }

    let mut measured = [None; MAX_BAR_REGISTERS];
    let mut index = 0usize;
    let mut operation_error = None;
    let mut bar_restore_failed = false;

    while index < count {
        let low = original[index];
        let is_io = low & 1 != 0;
        let memory_type = (low >> 1) & 0x3;
        let is_64 = !is_io && memory_type == 0x2;
        if !is_io && memory_type == 0x3 {
            operation_error = Some(BarProbeError::InvalidBar);
            break;
        }
        if is_64 && index + 1 >= count {
            operation_error = Some(BarProbeError::InvalidBar);
            break;
        }

        let low_register = BAR0 + (index as u16) * 4;
        let high_original = if is_64 { original[index + 1] } else { 0 };
        let probe = (|| -> Result<Option<u64>, BarProbeError> {
            io.write_u32(function.address, low_register, u32::MAX)
                .map_err(BarProbeError::Access)?;
            if is_64 {
                io.write_u32(function.address, low_register + 4, u32::MAX)
                    .map_err(BarProbeError::Access)?;
            }
            let mask_low = io.read_u32(function.address, low_register)
                .map_err(BarProbeError::Access)?;
            let mask_high = if is_64 {
                io.read_u32(function.address, low_register + 4)
                    .map_err(BarProbeError::Access)?
            } else { 0 };

            let size = if is_io {
                let mask = mask_low & !0x3;
                if mask == 0 {
                    if mask_low & 1 != 0 { Some(1u64 << 32) } else { None }
                } else { Some((!mask).wrapping_add(1) as u64) }
            } else if memory_type == 0x1 {
                let mask = mask_low & 0x000f_fff0;
                if mask == 0 {
                    if mask_low & 0x6 == 0x2 { Some(1u64 << 20) } else { None }
                } else { Some((!(mask as u64) & 0x000f_ffff) + 1) }
            } else if is_64 {
                let mask = ((mask_high as u64) << 32) | ((mask_low & !0xf) as u64);
                if mask == 0 {
                    if mask_low & 0x6 == 0x4 { return Err(BarProbeError::InvalidMask); }
                    None
                } else { Some((!mask).wrapping_add(1)) }
            } else {
                let mask = mask_low & !0xf;
                if mask == 0 {
                    if mask_low & 0x8 != 0 { Some(1u64 << 32) } else { None }
                } else { Some((!mask).wrapping_add(1) as u64) }
            };
            if let Some(value) = size {
                if value == 0 || !value.is_power_of_two() {
                    return Err(BarProbeError::InvalidMask);
                }
            }
            Ok(size)
        })();

        // Restore both halves even if probing failed partway through.
        if is_64 && io.write_u32(function.address, low_register + 4, high_original).is_err() {
            bar_restore_failed = true;
        }
        if io.write_u32(function.address, low_register, low).is_err() {
            bar_restore_failed = true;
        }
        if bar_restore_failed {
            operation_error = Some(BarProbeError::RestoreFailed);
            break;
        }

        match probe {
            Ok(Some(size)) => measured[index] = Some(BarSize { address: function.address, index: index as u8, size }),
            Ok(None) => {},
            Err(error) => { operation_error = Some(error); break; }
        }
        index += if is_64 { 2 } else { 1 };
    }

    // If a BAR could not be restored, leave decode disabled. Re-enabling it
    // with an unknown aperture would be less safe than keeping the function off.
    if bar_restore_failed {
        return Err(BarProbeError::RestoreFailed);
    }

    // Restore only command bits; status is write-one-to-clear.
    if io.write_u32(function.address, COMMAND_STATUS, original_command as u32).is_err() {
        return Err(BarProbeError::RestoreFailed);
    }
    if let Some(error) = operation_error {
        return Err(error);
    }

    out[..count].copy_from_slice(&measured[..count]);
    Ok(measured[..count].iter().filter(|entry| entry.is_some()).count())
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::RefCell;
    use super::super::config::PciIdentity;

    struct MockConfig {
        registers: RefCell<[u32; 64]>,
        masks: [u32; MAX_BAR_REGISTERS],
    }

    impl ConfigAccess for MockConfig {
        fn read_u32(&self, _address: PciAddress, register: u16) -> Result<u32, ConfigError> {
            let slot = (register / 4) as usize;
            let value = self.registers.borrow()[slot];
            if (BAR0..BAR0 + 24).contains(&register) && value == u32::MAX {
                Ok(self.masks[((register - BAR0) / 4) as usize])
            } else {
                Ok(value)
            }
        }

        fn write_u32(&self, _address: PciAddress, register: u16, value: u32) -> Result<(), ConfigError> {
            let slot = (register / 4) as usize;
            let mut registers = self.registers.borrow_mut();
            if register == COMMAND_STATUS {
                let old_status = registers[slot] >> 16;
                let status = old_status & !(value >> 16);
                registers[slot] = (status << 16) | (value & 0xffff);
            } else {
                registers[slot] = value;
            }
            Ok(())
        }
    }

    fn function() -> PciFunction {
        PciFunction {
            address: PciAddress { segment: 0, bus: 0, device: 1, function: 0 },
            identity: PciIdentity {
                vendor_id: 0x1234, device_id: 0x5678, command: 7, status: 0,
                revision: 0, programming_interface: 0, subclass: 0, class_code: 0,
                header_type: 0,
            },
        }
    }

    #[test]
    fn sizes_32_bit_bar_and_restores_command_and_status() {
        let mut registers = [0u32; 64];
        registers[1] = (0xa55a << 16) | 0x0007;
        registers[4] = 0x8000_0000;
        let mock = MockConfig { registers: RefCell::new(registers), masks: [0xffff_f000, 0, 0, 0, 0, 0] };
        let mut sizes = [None; MAX_BAR_REGISTERS];
        let count = unsafe { probe_bar_sizes(&mock, function(), &mut sizes) }.unwrap();
        assert_eq!(count, 1);
        assert_eq!(sizes[0], Some(BarSize { address: function().address, index: 0, size: 0x1000 }));
        let restored = mock.registers.borrow();
        assert_eq!(restored[4], 0x8000_0000);
        assert_eq!(restored[1] as u16, 0x0007);
        assert_eq!(restored[1] >> 16, 0xa55a);
    }

    #[test]
    fn sizes_below_one_megabyte_bar() {
        let mut registers = [0u32; 64];
        registers[1] = 0x0003;
        registers[4] = 0x0008_1002;
        let mock = MockConfig {
            registers: RefCell::new(registers),
            masks: [0x000f_f002, 0, 0, 0, 0, 0],
        };
        let mut sizes = [None; MAX_BAR_REGISTERS];
        let count = unsafe { probe_bar_sizes(&mock, function(), &mut sizes) }.unwrap();
        assert_eq!(count, 1);
        assert_eq!(sizes[0], Some(BarSize { address: function().address, index: 0, size: 0x1000 }));
        assert_eq!(mock.registers.borrow()[4], 0x0008_1002);
    }

    #[test]
    fn sizes_64_bit_bar_as_one_resource_and_restores_both_halves() {
        let mut registers = [0u32; 64];
        registers[1] = 0x0003;
        registers[4] = 0x0000_0004;
        registers[5] = 0x0000_0001;
        let mock = MockConfig {
            registers: RefCell::new(registers),
            masks: [0xffff_e004, 0xffff_ffff, 0, 0, 0, 0],
        };
        let mut sizes = [None; MAX_BAR_REGISTERS];
        let count = unsafe { probe_bar_sizes(&mock, function(), &mut sizes) }.unwrap();
        assert_eq!(count, 1);
        assert_eq!(sizes[0], Some(BarSize { address: function().address, index: 0, size: 0x2000 }));
        assert_eq!(sizes[1], None);
        let restored = mock.registers.borrow();
        assert_eq!(restored[4], 0x4);
        assert_eq!(restored[5], 0x1);
        assert_eq!(restored[1] as u16, 0x3);
    }

    struct FaultInjectingConfig {
        registers: RefCell<[u32; 64]>,
        masks: [u32; MAX_BAR_REGISTERS],
        accesses: core::cell::Cell<usize>,
        fail_at: usize,
    }

    impl FaultInjectingConfig {
        fn should_fail(&self) -> bool {
            let next = self.accesses.get() + 1;
            self.accesses.set(next);
            next == self.fail_at
        }
    }

    impl ConfigAccess for FaultInjectingConfig {
        fn read_u32(&self, _address: PciAddress, register: u16) -> Result<u32, ConfigError> {
            if self.should_fail() {
                return Err(ConfigError::OutOfRange);
            }
            let slot = (register / 4) as usize;
            let value = self.registers.borrow()[slot];
            if (BAR0..BAR0 + 24).contains(&register) && value == u32::MAX {
                Ok(self.masks[((register - BAR0) / 4) as usize])
            } else {
                Ok(value)
            }
        }

        fn write_u32(&self, _address: PciAddress, register: u16, value: u32) -> Result<(), ConfigError> {
            if self.should_fail() {
                return Err(ConfigError::OutOfRange);
            }
            let slot = (register / 4) as usize;
            let mut registers = self.registers.borrow_mut();
            if register == COMMAND_STATUS {
                let old_status = registers[slot] >> 16;
                let status = old_status & !(value >> 16);
                registers[slot] = (status << 16) | (value & 0xffff);
            } else {
                registers[slot] = value;
            }
            Ok(())
        }
    }

    #[test]
    fn restores_bar_command_and_status_when_mask_read_fails() {
        let mut registers = [0u32; 64];
        registers[1] = (0xa55a << 16) | 0x0007;
        registers[4] = 0x8000_0000;
        let mock = FaultInjectingConfig {
            registers: RefCell::new(registers),
            masks: [0xffff_f000, 0, 0, 0, 0, 0],
            accesses: core::cell::Cell::new(0),
            // Six BAR reads, one Command/Status read, decode-disable write,
            // BAR all-ones write, then the failing mask read.
            fail_at: 10,
        };
        let mut sizes = [None; MAX_BAR_REGISTERS];
        let result = unsafe { probe_bar_sizes(&mock, function(), &mut sizes) };
        assert_eq!(result, Err(BarProbeError::Access(ConfigError::OutOfRange)));

        let restored = mock.registers.borrow();
        assert_eq!(restored[4], 0x8000_0000);
        assert_eq!(restored[1] as u16, 0x0007);
        assert_eq!(restored[1] >> 16, 0xa55a);
    }

}
