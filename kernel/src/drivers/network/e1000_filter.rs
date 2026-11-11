//! Allocation-free E1000 multicast hash filter (128 x 32-bit MTA).
//!
//! The legacy E1000 multicast hash mode 0 uses the upper twelve bits of the
//! final two MAC octets. The resulting 12-bit hash selects one bit in the
//! 4096-bit Multicast Table Array.

use super::e1000::E1000_MTA_REGISTER_COUNT;
use super::ethernet::MacAddress;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum E1000MulticastError {
    NotMulticast,
    BroadcastAddress,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct E1000MulticastFilter {
    registers: [u32; E1000_MTA_REGISTER_COUNT],
}

impl E1000MulticastFilter {
    pub const fn empty() -> Self {
        Self { registers: [0; E1000_MTA_REGISTER_COUNT] }
    }

    /// Build a complete filter without allocation.
    pub fn from_addresses(addresses: &[MacAddress]) -> Result<Self, E1000MulticastError> {
        let mut filter = Self::empty();
        for address in addresses {
            filter.insert(*address)?;
        }
        Ok(filter)
    }

    /// Add one multicast address to the MTA image.
    pub fn insert(&mut self, address: MacAddress) -> Result<(), E1000MulticastError> {
        if address.is_broadcast() {
            return Err(E1000MulticastError::BroadcastAddress);
        }
        if !address.is_multicast() {
            return Err(E1000MulticastError::NotMulticast);
        }

        let hash = Self::hash(address) as usize;
        self.registers[hash >> 5] |= 1u32 << (hash & 0x1f);
        Ok(())
    }

    /// Intel E1000 multicast hash mode 0.
    ///
    /// Example: 01:AA:00:12:34:56 produces 0x563.
    pub const fn hash(address: MacAddress) -> u16 {
        (((address.0[4] as u16) >> 4) | ((address.0[5] as u16) << 4)) & 0x0fff
    }

    pub const fn registers(&self) -> &[u32; E1000_MTA_REGISTER_COUNT] {
        &self.registers
    }

    pub const fn is_empty(&self) -> bool {
        let mut index = 0;
        while index < E1000_MTA_REGISTER_COUNT {
            if self.registers[index] != 0 {
                return false;
            }
            index += 1;
        }
        true
    }

    pub fn accepts(&self, address: MacAddress) -> bool {
        if !address.is_multicast() || address.is_broadcast() {
            return false;
        }
        let hash = Self::hash(address) as usize;
        self.registers[hash >> 5] & (1u32 << (hash & 0x1f)) != 0
    }
}

impl Default for E1000MulticastFilter {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_intel_mode_zero_hash_and_sets_expected_mta_bit() {
        let address = MacAddress([0x01, 0xaa, 0x00, 0x12, 0x34, 0x56]);
        assert_eq!(E1000MulticastFilter::hash(address), 0x563);

        let mut filter = E1000MulticastFilter::empty();
        assert_eq!(filter.insert(address), Ok(()));
        assert!(filter.accepts(address));
        let register = 0x563usize >> 5;
        let bit = 1u32 << (0x563usize & 0x1f);
        assert_eq!(filter.registers()[register] & bit, bit);
    }

    #[test]
    fn rejects_unicast_and_broadcast_addresses() {
        let mut filter = E1000MulticastFilter::empty();
        assert_eq!(
            filter.insert(MacAddress([0x02, 0, 0, 0, 0, 1])),
            Err(E1000MulticastError::NotMulticast),
        );
        assert_eq!(
            filter.insert(MacAddress::BROADCAST),
            Err(E1000MulticastError::BroadcastAddress),
        );
        assert!(filter.is_empty());
    }

    #[test]
    fn duplicate_addresses_do_not_change_the_filter_image() {
        let address = MacAddress([0x33, 0x33, 0, 0, 0, 1]);
        let once = E1000MulticastFilter::from_addresses(&[address]).unwrap();
        let twice = E1000MulticastFilter::from_addresses(&[address, address]).unwrap();
        assert_eq!(once, twice);
    }
}
