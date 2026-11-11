//! Safe, allocation-free helpers for constructing and inspecting legacy E1000 descriptors.
//!
//! These helpers do not access device memory. The caller remains responsible
//! for DMA mapping, descriptor ownership, and memory-ordering requirements.

use super::e1000::{
    E1000Error, RxDescriptor, TxDescriptor, RXD_STATUS_DD, RXD_STATUS_EOP,
    TXD_CMD_EOP, TXD_CMD_IFCS, TXD_CMD_RS, TXD_STATUS_DD,
};
use super::ethernet::MacAddress;

pub const RXD_ERROR_CE: u8 = 1 << 0;
pub const RXD_ERROR_SE: u8 = 1 << 1;
pub const RXD_ERROR_SEQ: u8 = 1 << 2;
pub const RXD_ERROR_CXE: u8 = 1 << 4;
pub const RXD_ERROR_TCPE: u8 = 1 << 5;
pub const RXD_ERROR_IPE: u8 = 1 << 6;
pub const RXD_ERROR_RXE: u8 = 1 << 7;
pub const RAH0_AV: u32 = 1 << 31;

/// Extract the programmed MAC address from RAL0/RAH0.
/// The address is unavailable until the hardware sets RAH0.AV.
pub const fn mac_address_from_registers(ral: u32, rah: u32) -> Option<MacAddress> {
    if rah & RAH0_AV == 0 {
        return None;
    }
    Some(MacAddress([
        ral as u8,
        (ral >> 8) as u8,
        (ral >> 16) as u8,
        (ral >> 24) as u8,
        rah as u8,
        (rah >> 8) as u8,
    ]))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RxCompletion {
    pub length: u16,
    pub end_of_packet: bool,
    pub errors: u8,
}

impl RxDescriptor {
    /// Initialize a descriptor with a DMA buffer address.
    /// The buffer must remain pinned and device-accessible until completion.
    pub const fn for_buffer(buffer_address: u64) -> Self {
        Self {
            buffer_address,
            length: 0,
            checksum: 0,
            status: 0,
            errors: 0,
            special: 0,
        }
    }

    /// Return metadata only after the device sets the descriptor-done bit.
    pub const fn completion(&self) -> Option<RxCompletion> {
        if self.status & RXD_STATUS_DD == 0 {
            return None;
        }
        Some(RxCompletion {
            length: self.length,
            end_of_packet: self.status & RXD_STATUS_EOP != 0,
            errors: self.errors,
        })
    }

    pub const fn has_errors(&self) -> bool {
        self.errors != 0
    }
}

impl TxDescriptor {
    /// Build a single-buffer transmit descriptor requesting FCS and write-back.
    /// The caller must retain the DMA buffer until the device sets DD.
    pub fn for_frame(buffer_address: u64, length: usize) -> Result<Self, E1000Error> {
        if length == 0 || length > u16::MAX as usize {
            return Err(E1000Error::InvalidDescriptorLength);
        }
        Ok(Self {
            buffer_address,
            length: length as u16,
            checksum_offset: 0,
            command: TXD_CMD_EOP | TXD_CMD_IFCS | TXD_CMD_RS,
            status: 0,
            checksum_start: 0,
            special: 0,
        })
    }

    pub const fn is_complete(&self) -> bool {
        self.status & TXD_STATUS_DD != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rx_descriptor_reports_only_completed_buffers() {
        let mut descriptor = RxDescriptor::for_buffer(0x2000);
        assert_eq!(descriptor.completion(), None);
        descriptor.length = 128;
        descriptor.status = RXD_STATUS_DD | RXD_STATUS_EOP;
        assert_eq!(descriptor.completion(), Some(RxCompletion {
            length: 128, end_of_packet: true, errors: 0,
        }));
    }

    #[test]
    fn tx_descriptor_sets_required_command_bits_and_checks_length() {
        let descriptor = TxDescriptor::for_frame(0x3000, 64).unwrap();
        assert_eq!(descriptor.length, 64);
        assert_eq!(descriptor.command, TXD_CMD_EOP | TXD_CMD_IFCS | TXD_CMD_RS);
        assert!(!descriptor.is_complete());
        assert_eq!(TxDescriptor::for_frame(0x3000, 0), Err(E1000Error::InvalidDescriptorLength));
        assert_eq!(TxDescriptor::for_frame(0x3000, u16::MAX as usize + 1), Err(E1000Error::InvalidDescriptorLength));
    }

    #[test]
    fn mac_address_requires_valid_bit_and_uses_register_byte_order() {
        assert_eq!(mac_address_from_registers(0x44332211, 0x80006655),
            Some(MacAddress([0x11, 0x22, 0x33, 0x44, 0x55, 0x66])));
        assert_eq!(mac_address_from_registers(0x44332211, 0x00006655), None);
    }
}
