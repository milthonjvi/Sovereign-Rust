//! Ethernet address and frame parsing primitives (no allocation).

pub const ETHERNET_ADDRESS_LEN: usize = 6;
pub const ETHERNET_HEADER_LEN: usize = 14;
pub const VLAN_HEADER_LEN: usize = 4;
pub const ETHERTYPE_IPV4: u16 = 0x0800;
pub const ETHERTYPE_ARP: u16 = 0x0806;
pub const ETHERTYPE_IPV6: u16 = 0x86dd;
pub const ETHERTYPE_VLAN: u16 = 0x8100;
pub const ETHERTYPE_QINQ: u16 = 0x88a8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MacAddress(pub [u8; ETHERNET_ADDRESS_LEN]);

impl MacAddress {
    pub const BROADCAST: Self = Self([0xff; ETHERNET_ADDRESS_LEN]);
    pub const ZERO: Self = Self([0; ETHERNET_ADDRESS_LEN]);
    pub const fn is_broadcast(self) -> bool { self.0 == [0xff; ETHERNET_ADDRESS_LEN] }
    pub const fn is_multicast(self) -> bool { self.0[0] & 1 != 0 }
    pub fn is_unicast(self) -> bool { !self.is_multicast() && self != Self::ZERO }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EthernetFrameError { TruncatedHeader, TruncatedVlanTag, NestedVlanUnsupported }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EthernetFrame<'a> {
    pub destination: MacAddress,
    pub source: MacAddress,
    pub ether_type: u16,
    pub vlan_id: Option<u16>,
    pub payload: &'a [u8],
}

impl<'a> EthernetFrame<'a> {
    /// Parse an Ethernet II frame, accepting at most one 802.1Q/802.1ad tag.
    /// The Ethernet FCS is expected to have been stripped by the NIC.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, EthernetFrameError> {
        if bytes.len() < ETHERNET_HEADER_LEN { return Err(EthernetFrameError::TruncatedHeader); }
        let destination = MacAddress(bytes[0..6].try_into().expect("validated Ethernet header"));
        let source = MacAddress(bytes[6..12].try_into().expect("validated Ethernet header"));
        let mut ether_type = u16::from_be_bytes([bytes[12], bytes[13]]);
        let mut offset = ETHERNET_HEADER_LEN;
        let mut vlan_id = None;
        if ether_type == ETHERTYPE_VLAN || ether_type == ETHERTYPE_QINQ {
            if bytes.len() < ETHERNET_HEADER_LEN + VLAN_HEADER_LEN {
                return Err(EthernetFrameError::TruncatedVlanTag);
            }
            let tci = u16::from_be_bytes([bytes[14], bytes[15]]);
            vlan_id = Some(tci & 0x0fff);
            ether_type = u16::from_be_bytes([bytes[16], bytes[17]]);
            offset += VLAN_HEADER_LEN;
            if ether_type == ETHERTYPE_VLAN || ether_type == ETHERTYPE_QINQ {
                return Err(EthernetFrameError::NestedVlanUnsupported);
            }
        }
        Ok(Self { destination, source, ether_type, vlan_id, payload: &bytes[offset..] })
    }
}