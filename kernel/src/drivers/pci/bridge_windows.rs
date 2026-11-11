//! Read-only decoding of PCI-to-PCI bridge forwarding windows.
//!
//! These apertures constrain BAR allocations downstream of a bridge. The
//! module only reads bridge registers; it never reprograms firmware topology.

use super::config::{ConfigAccess, ConfigError, PciFunction};
use super::resource_allocator::{AllocationError, ResourceSpace, ResourceWindow};

const COMMAND_STATUS: u16 = 0x04;
const IO_BASE_LIMIT: u16 = 0x1c;
const MEMORY_BASE_LIMIT: u16 = 0x20;
const PREFETCH_BASE_LIMIT: u16 = 0x24;
const PREFETCH_BASE_UPPER: u16 = 0x28;
const PREFETCH_LIMIT_UPPER: u16 = 0x2c;
const IO_BASE_UPPER_LIMIT: u16 = 0x30;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BridgeWindowError {
    Access(ConfigError),
    NotPciBridge,
    InvalidEncoding,
    AddressOverflow,
    InvalidWindow(AllocationError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PciBridgeWindows {
    pub io: Option<ResourceWindow>,
    pub memory: Option<ResourceWindow>,
    pub prefetchable_memory: Option<ResourceWindow>,
}

impl PciBridgeWindows {
    /// Read the I/O, non-prefetchable memory, and prefetchable memory windows.
    /// A disabled window is represented by `None`.
    pub fn read<A: ConfigAccess>(function: PciFunction, io: &A) -> Result<Self, BridgeWindowError> {
        let identity = function.identity;
        if identity.header_type & 0x7f != 1
            || identity.class_code != 0x06
            || identity.subclass != 0x04
        {
            return Err(BridgeWindowError::NotPciBridge);
        }

        // PciFunction carries an enumeration-time snapshot. Read Command
        // again so window enablement reflects the device's current decode state.
        let command = io.read_u32(function.address, COMMAND_STATUS)
            .map_err(BridgeWindowError::Access)? as u16;
        let io_base_limit = io.read_u32(function.address, IO_BASE_LIMIT)
            .map_err(BridgeWindowError::Access)?;
        let io_upper = io.read_u32(function.address, IO_BASE_UPPER_LIMIT)
            .map_err(BridgeWindowError::Access)?;
        let io_base_type = (io_base_limit & 0x0f) as u8;
        let io_limit_type = ((io_base_limit >> 8) & 0x0f) as u8;
        if io_base_type != io_limit_type || io_base_type > 1 {
            return Err(BridgeWindowError::InvalidEncoding);
        }
        let io_base_upper = if io_base_type == 1 { (io_upper & 0xffff) as u64 } else { 0 };
        let io_limit_upper = if io_base_type == 1 { (io_upper >> 16) as u64 } else { 0 };
        let io_base = (io_base_upper << 16) | (((io_base_limit & 0x00f0) as u64) << 8);
        let io_limit = (io_limit_upper << 16)
            | ((((io_base_limit >> 8) & 0x00f0) as u64) << 8)
            | 0x0fff;
        let io_window = if command & 1 != 0 && io_base <= io_limit {
            Some(make_window(ResourceSpace::Io, false, io_base, io_limit)?)
        } else { None };

        let memory = io.read_u32(function.address, MEMORY_BASE_LIMIT)
            .map_err(BridgeWindowError::Access)?;
        if memory & 0x000f_000f != 0 {
            return Err(BridgeWindowError::InvalidEncoding);
        }
        let memory_base = ((memory & 0x0000_fff0) as u64) << 16;
        let memory_limit = (((memory >> 16) & 0x0000_fff0) as u64) << 16 | 0x000f_ffff;
        let memory_window = if command & 2 != 0 && memory_base <= memory_limit {
            Some(make_window(ResourceSpace::Memory, false, memory_base, memory_limit)?)
        } else { None };

        let prefetch = io.read_u32(function.address, PREFETCH_BASE_LIMIT)
            .map_err(BridgeWindowError::Access)?;
        let prefetch_base_type = (prefetch & 0x0f) as u8;
        let prefetch_limit_type = ((prefetch >> 16) & 0x0f) as u8;
        if prefetch_base_type != prefetch_limit_type || prefetch_base_type > 1 {
            return Err(BridgeWindowError::InvalidEncoding);
        }
        let (base_upper, limit_upper) = if prefetch_base_type == 1 {
            (
                io.read_u32(function.address, PREFETCH_BASE_UPPER)
                    .map_err(BridgeWindowError::Access)? as u64,
                io.read_u32(function.address, PREFETCH_LIMIT_UPPER)
                    .map_err(BridgeWindowError::Access)? as u64,
            )
        } else { (0, 0) };
        let prefetch_base = (base_upper << 32) | (((prefetch & 0x0000_fff0) as u64) << 16);
        let prefetch_limit = (limit_upper << 32)
            | ((((prefetch >> 16) & 0x0000_fff0) as u64) << 16)
            | 0x000f_ffff;
        let prefetch_window = if command & 2 != 0 && prefetch_base <= prefetch_limit {
            Some(make_window(ResourceSpace::Memory, true, prefetch_base, prefetch_limit)?)
        } else { None };

        Ok(Self { io: io_window, memory: memory_window, prefetchable_memory: prefetch_window })
    }
}

fn make_window(
    space: ResourceSpace,
    prefetchable: bool,
    base: u64,
    inclusive_limit: u64,
) -> Result<ResourceWindow, BridgeWindowError> {
    let size = inclusive_limit.checked_sub(base)
        .and_then(|difference| difference.checked_add(1))
        .ok_or(BridgeWindowError::AddressOverflow)?;
    ResourceWindow::new(space, prefetchable, base, size)
        .map_err(BridgeWindowError::InvalidWindow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::config::{PciAddress, PciIdentity};

    struct MockConfig([u32; 64]);
    impl ConfigAccess for MockConfig {
        fn read_u32(&self, _address: PciAddress, register: u16) -> Result<u32, ConfigError> {
            self.0.get((register / 4) as usize).copied().ok_or(ConfigError::OutOfRange)
        }
        fn write_u32(&self, _address: PciAddress, _register: u16, _value: u32) -> Result<(), ConfigError> {
            Err(ConfigError::OutOfRange)
        }
    }

    fn bridge() -> PciFunction {
        PciFunction {
            address: PciAddress { segment: 0, bus: 0, device: 1, function: 0 },
            identity: PciIdentity {
                vendor_id: 1, device_id: 2, command: 3, status: 0, revision: 0,
                programming_interface: 0, subclass: 4, class_code: 6, header_type: 1,
            },
        }
    }

    #[test]
    fn decodes_32_bit_memory_and_io_bridge_windows() {
        let mut registers = [0u32; 64];
        registers[(COMMAND_STATUS / 4) as usize] = 3;
        // I/O: 0x1000..=0x1fff, 16-bit I/O window.
        registers[(IO_BASE_LIMIT / 4) as usize] = 0x0000_1010;
        // Memory: 0x8000_0000..=0x80ff_ffff.
        registers[(MEMORY_BASE_LIMIT / 4) as usize] = 0x80f0_8000;
        // Disabled prefetchable window: base greater than limit.
        registers[(PREFETCH_BASE_LIMIT / 4) as usize] = 0x0000_fff0;
        let windows = PciBridgeWindows::read(bridge(), &MockConfig(registers)).unwrap();
        let io = windows.io.unwrap();
        assert_eq!(io.base(), 0x1000);
        assert_eq!(io.end(), 0x2000);
        let memory = windows.memory.unwrap();
        assert_eq!(memory.base(), 0x8000_0000);
        assert_eq!(memory.end(), 0x8100_0000);
        assert!(windows.prefetchable_memory.is_none());
    }

    #[test]
    fn decodes_64_bit_prefetchable_bridge_window() {
        let mut registers = [0u32; 64];
        registers[(COMMAND_STATUS / 4) as usize] = 2;
        registers[(PREFETCH_BASE_LIMIT / 4) as usize] = 0x0001_0001;
        registers[(PREFETCH_BASE_UPPER / 4) as usize] = 1;
        registers[(PREFETCH_LIMIT_UPPER / 4) as usize] = 1;
        let windows = PciBridgeWindows::read(bridge(), &MockConfig(registers)).unwrap();
        let prefetch = windows.prefetchable_memory.unwrap();
        assert_eq!(prefetch.base(), 0x1_0000_0000);
        assert_eq!(prefetch.end(), 0x1_0010_0000);
    }

    #[test]
    fn uses_live_command_register_instead_of_enumeration_snapshot() {
        let mut registers = [0u32; 64];
        registers[(COMMAND_STATUS / 4) as usize] = 2;
        registers[(MEMORY_BASE_LIMIT / 4) as usize] = 0x80f0_8000;
        registers[(PREFETCH_BASE_LIMIT / 4) as usize] = 0x0001_0001;
        registers[(PREFETCH_BASE_UPPER / 4) as usize] = 1;
        registers[(PREFETCH_LIMIT_UPPER / 4) as usize] = 1;
        let mut stale = bridge();
        stale.identity.command = 0;
        let windows = PciBridgeWindows::read(stale, &MockConfig(registers)).unwrap();
        assert!(windows.memory.is_some());
        assert!(windows.prefetchable_memory.is_some());
        assert!(windows.io.is_none());

        registers[(COMMAND_STATUS / 4) as usize] = 0;
        stale.identity.command = 3;
        let windows = PciBridgeWindows::read(stale, &MockConfig(registers)).unwrap();
        assert!(windows.memory.is_none());
        assert!(windows.io.is_none());
    }

}
