//! PCI Command register control with Status-register W1C protection.
//!
//! The Command and Status registers share one dword at offset 0x04. Every write
//! here places zero in the Status half so latched W1C status bits are preserved.

use super::config::{ConfigAccess, ConfigError, PciAddress};

pub const COMMAND_IO_SPACE: u16 = 1 << 0;
pub const COMMAND_MEMORY_SPACE: u16 = 1 << 1;
pub const COMMAND_BUS_MASTER: u16 = 1 << 2;
pub const COMMAND_INTERRUPT_DISABLE: u16 = 1 << 10;
const COMMAND_STATUS_REGISTER: u16 = 0x04;

/// Read the current 16-bit PCI Command register.
pub fn read_command<A: ConfigAccess>(io: &A, address: PciAddress) -> Result<u16, ConfigError> {
    Ok(io.read_u32(address, COMMAND_STATUS_REGISTER)? as u16)
}

/// Update selected Command bits without writing ones into the Status register.
///
/// # Safety
/// The caller must own the device's configuration state and ensure the selected
/// command transition is valid for the device's current DMA and decode state.
pub unsafe fn update_command<A: ConfigAccess>(
    io: &A,
    address: PciAddress,
    mask: u16,
    value: u16,
) -> Result<u16, ConfigError> {
    let current = read_command(io, address)?;
    let updated = (current & !mask) | (value & mask);
    // Status occupies the upper 16 bits and is write-one-to-clear. Write zero.
    io.write_u32(address, COMMAND_STATUS_REGISTER, updated as u32)?;
    Ok(updated)
}

/// Enable or disable device bus mastering.
///
/// # Safety
/// Enabling requires fully initialized DMA buffers, valid device-visible
/// addresses, and any required IOMMU mappings. Disabling requires the device to
/// be stopped or otherwise safe to prevent further DMA transactions.
pub unsafe fn set_bus_master<A: ConfigAccess>(
    io: &A,
    address: PciAddress,
    enabled: bool,
) -> Result<u16, ConfigError> {
    let value = if enabled { COMMAND_BUS_MASTER } else { 0 };
    unsafe { update_command(io, address, COMMAND_BUS_MASTER, value) }
}

/// Enable or disable memory-space decoding after BAR resources are programmed.
///
/// # Safety
/// Every enabled memory BAR must have a valid, non-overlapping assignment and
/// the corresponding MMIO mappings must be established before enabling decode.
pub unsafe fn set_memory_space<A: ConfigAccess>(
    io: &A,
    address: PciAddress,
    enabled: bool,
) -> Result<u16, ConfigError> {
    let value = if enabled { COMMAND_MEMORY_SPACE } else { 0 };
    unsafe { update_command(io, address, COMMAND_MEMORY_SPACE, value) }
}

/// Enable or disable I/O-port decoding after I/O BAR resources are programmed.
///
/// # Safety
/// Every enabled I/O BAR must have a valid, non-overlapping port assignment.
pub unsafe fn set_io_space<A: ConfigAccess>(
    io: &A,
    address: PciAddress,
    enabled: bool,
) -> Result<u16, ConfigError> {
    let value = if enabled { COMMAND_IO_SPACE } else { 0 };
    unsafe { update_command(io, address, COMMAND_IO_SPACE, value) }
}

/// Set the PCI Interrupt Disable bit (INTx), without affecting MSI/MSI-X state.
///
/// # Safety
/// The caller must own the interrupt-mode transition and ensure the device's
/// active interrupt mechanism remains correctly configured.
pub unsafe fn set_intx_disabled<A: ConfigAccess>(
    io: &A,
    address: PciAddress,
    disabled: bool,
) -> Result<u16, ConfigError> {
    let value = if disabled { COMMAND_INTERRUPT_DISABLE } else { 0 };
    unsafe { update_command(io, address, COMMAND_INTERRUPT_DISABLE, value) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::RefCell;

    struct MockConfig(RefCell<u32>);
    impl ConfigAccess for MockConfig {
        fn read_u32(&self, _address: PciAddress, register: u16) -> Result<u32, ConfigError> {
            if register == COMMAND_STATUS_REGISTER { Ok(*self.0.borrow()) } else { Err(ConfigError::OutOfRange) }
        }
        fn write_u32(&self, _address: PciAddress, register: u16, value: u32) -> Result<(), ConfigError> {
            if register != COMMAND_STATUS_REGISTER { return Err(ConfigError::OutOfRange); }
            let old_status = *self.0.borrow() >> 16;
            *self.0.borrow_mut() = ((old_status & !(value >> 16)) << 16) | (value & 0xffff);
            Ok(())
        }
    }

    fn address() -> PciAddress {
        PciAddress { segment: 0, bus: 0, device: 0, function: 0 }
    }

    #[test]
    fn updates_command_bits_without_clearing_status() {
        let io = MockConfig(RefCell::new((0xa55a << 16) | 0x0003));
        let updated = unsafe { set_bus_master(&io, address(), true) }.unwrap();
        assert_eq!(updated, 0x0007);
        assert_eq!(*io.0.borrow(), (0xa55a << 16) | 0x0007);
        let updated = unsafe { set_memory_space(&io, address(), false) }.unwrap();
        assert_eq!(updated, 0x0005);
        assert_eq!(*io.0.borrow() >> 16, 0xa55a);
    }
}
