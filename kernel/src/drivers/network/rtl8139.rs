//! Realtek RTL8139 receive-ring primitives.
//!
//! This module is isolated from PCI and platform DMA setup. It accepts only a
//! caller-provided pinned DMA mapping; it never allocates or maps hardware memory.

use super::dma::DmaBuffer;

pub const RTL8139_VENDOR_ID: u16 = 0x10ec;
pub const RTL8139_DEVICE_ID: u16 = 0x8139;
pub const REG_MAC0: u16 = 0x00;
pub const REG_TSD0: u16 = 0x10;
pub const REG_TSAD0: u16 = 0x20;
pub const REG_RBSTART: u16 = 0x30;
pub const REG_COMMAND: u16 = 0x37;
pub const REG_CAPR: u16 = 0x38;
pub const REG_CBR: u16 = 0x3a;
pub const REG_IMR: u16 = 0x3c;
pub const REG_ISR: u16 = 0x3e;
pub const REG_TCR: u16 = 0x40;
pub const REG_RCR: u16 = 0x44;
pub const REG_CONFIG1: u16 = 0x52;
pub const COMMAND_RESET: u8 = 1 << 4;
pub const COMMAND_RX_ENABLE: u8 = 1 << 3;
pub const COMMAND_TX_ENABLE: u8 = 1 << 2;
pub const COMMAND_RX_EMPTY: u8 = 1;
pub const RX_STATUS_ROK: u16 = 1;
pub const RX_STATUS_ERROR_MASK: u16 = 0x003e;
pub const RTL8139_MIN_RING_SIZE: usize = 8192;
pub const RTL8139_MAX_RING_SIZE: usize = 65536;
pub const RTL8139_RING_ALIGNMENT: usize = 256;
pub const RTL8139_MIN_FRAME_WITH_FCS: usize = 64;
pub const RTL8139_MAX_FRAME_WITH_FCS: usize = 1522;
pub const RTL8139_FCS_SIZE: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Rtl8139Error {
    InvalidRingSize,
    InvalidDmaBuffer,
    InvalidReadCursor,
    InvalidPacketLength,
    ReceiveError(u16),
    OutputTooSmall { required: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rtl8139RxPacket {
    pub status: u16,
    pub frame_length: usize,
    pub next_cursor: usize,
    start_cursor: usize,
}

/// CPU view of a pinned, device-visible RTL8139 receive ring.
pub struct Rtl8139RxRing<'a> {
    buffer: DmaBuffer<'a>,
    ring_size: usize,
    cursor: usize,
}

impl<'a> Rtl8139RxRing<'a> {
    /// Bind an existing DMA ring; this function does not establish its mapping.
    ///
    /// # Safety
    /// The buffer must be the pinned DMA region programmed into RBSTART and
    /// remain valid and exclusively owned according to the NIC RX protocol.
    pub unsafe fn new(
        buffer: DmaBuffer<'a>,
        ring_size: usize,
    ) -> Result<Self, Rtl8139Error> {
        if !(RTL8139_MIN_RING_SIZE..=RTL8139_MAX_RING_SIZE).contains(&ring_size)
            || !ring_size.is_power_of_two()
            || ring_size % RTL8139_RING_ALIGNMENT != 0
        {
            return Err(Rtl8139Error::InvalidRingSize);
        }
        if buffer.len() < ring_size
            || buffer.dma_address() % RTL8139_RING_ALIGNMENT as u64 != 0
        {
            return Err(Rtl8139Error::InvalidDmaBuffer);
        }
        Ok(Self { buffer, ring_size, cursor: 0 })
    }

    pub const fn ring_size(&self) -> usize { self.ring_size }
    pub const fn cursor(&self) -> usize { self.cursor }

    /// Inspect the next complete packet. CBR is the device's current write cursor.
    pub fn peek(&self, current_buffer_address: u16)
        -> Result<Option<Rtl8139RxPacket>, Rtl8139Error>
    {
        let device_cursor = current_buffer_address as usize;
        if device_cursor >= self.ring_size {
            return Err(Rtl8139Error::InvalidReadCursor);
        }
        let available = device_cursor.wrapping_sub(self.cursor) & (self.ring_size - 1);
        if available == 0 {
            return Ok(None);
        }
        if available < 4 {
            return Ok(None);
        }

        let status = self.read_u16(self.cursor);
        let raw_length = self.read_u16(self.cursor + 2) as usize;
        if raw_length < RTL8139_MIN_FRAME_WITH_FCS || raw_length > RTL8139_MAX_FRAME_WITH_FCS {
            return Err(Rtl8139Error::InvalidPacketLength);
        }
        let consumed = 4usize.checked_add(raw_length)
            .ok_or(Rtl8139Error::InvalidPacketLength)?;
        let aligned = consumed.checked_add(3)
            .ok_or(Rtl8139Error::InvalidPacketLength)? & !3;
        let next_cursor = (self.cursor + aligned) & (self.ring_size - 1);
        Ok(Some(Rtl8139RxPacket {
            status,
            frame_length: raw_length - RTL8139_FCS_SIZE,
            next_cursor,
            start_cursor: self.cursor,
        }))
    }

    /// Copy the Ethernet frame, excluding the RTL8139 header and FCS.
    /// The ring cursor is not advanced by this operation.
    pub fn copy_packet(
        &self,
        packet: Rtl8139RxPacket,
        output: &mut [u8],
    ) -> Result<usize, Rtl8139Error> {
        self.validate_packet(packet)?;
        if packet.status & RX_STATUS_ROK == 0 || packet.status & RX_STATUS_ERROR_MASK != 0 {
            return Err(Rtl8139Error::ReceiveError(packet.status));
        }
        if output.len() < packet.frame_length {
            return Err(Rtl8139Error::OutputTooSmall { required: packet.frame_length });
        }
        let mut offset = (self.cursor + 4) & (self.ring_size - 1);
        let mut copied = 0;
        while copied < packet.frame_length {
            let count = core::cmp::min(packet.frame_length - copied, self.ring_size - offset);
            output[copied..copied + count]
                .copy_from_slice(&self.buffer[offset..offset + count]);
            copied += count;
            offset = 0;
        }
        Ok(copied)
    }

    /// Commit only after the caller has successfully published the new CAPR.
    pub fn commit(&mut self, packet: Rtl8139RxPacket) -> Result<(), Rtl8139Error> {
        self.validate_packet(packet)?;
        self.cursor = packet.next_cursor;
        Ok(())
    }

    fn validate_packet(&self, packet: Rtl8139RxPacket) -> Result<(), Rtl8139Error> {
        if packet.start_cursor != self.cursor || packet.next_cursor >= self.ring_size {
            return Err(Rtl8139Error::InvalidReadCursor);
        }
        let raw_length = packet.frame_length.checked_add(RTL8139_FCS_SIZE)
            .ok_or(Rtl8139Error::InvalidPacketLength)?;
        if !(RTL8139_MIN_FRAME_WITH_FCS..=RTL8139_MAX_FRAME_WITH_FCS).contains(&raw_length)
            || packet.status != self.read_u16(self.cursor)
            || raw_length != self.read_u16(self.cursor + 2) as usize
        {
            return Err(Rtl8139Error::InvalidPacketLength);
        }
        let aligned = (4 + raw_length + 3) & !3;
        let expected = (self.cursor + aligned) & (self.ring_size - 1);
        if packet.next_cursor != expected {
            return Err(Rtl8139Error::InvalidReadCursor);
        }
        Ok(())
    }

    fn read_u16(&self, offset: usize) -> u16 {
        let low = self.buffer[offset & (self.ring_size - 1)];
        let high = self.buffer[(offset + 1) & (self.ring_size - 1)];
        u16::from_le_bytes([low, high])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring<'a>(bytes: &'a mut [u8]) -> Rtl8139RxRing<'a> {
        let dma = unsafe { DmaBuffer::new_for_test(0x10000, bytes).unwrap() };
        unsafe { Rtl8139RxRing::new(dma, RTL8139_MIN_RING_SIZE).unwrap() }
    }

    #[test]
    fn parses_frame_and_excludes_fcs() {
        let mut bytes = [0u8; RTL8139_MIN_RING_SIZE];
        let mut rx = ring(&mut bytes);
        rx.buffer[0..2].copy_from_slice(&RX_STATUS_ROK.to_le_bytes());
        rx.buffer[2..4].copy_from_slice(&64u16.to_le_bytes());
        for (i, byte) in rx.buffer[4..64].iter_mut().enumerate() { *byte = i as u8; }
        let packet = rx.peek(68).unwrap().unwrap();
        assert_eq!(packet.frame_length, 60);
        let mut output = [0u8; 60];
        assert_eq!(rx.copy_packet(packet, &mut output), Ok(60));
        assert_eq!(output[59], 59);
        assert_eq!(rx.cursor(), 0);
        rx.commit(packet).unwrap();
        assert_eq!(rx.cursor(), 68);
    }

    #[test]
    fn wraps_header_and_payload_at_ring_end() {
        let mut bytes = [0u8; RTL8139_MIN_RING_SIZE];
        let mut rx = ring(&mut bytes);
        rx.cursor = RTL8139_MIN_RING_SIZE - 2;
        rx.buffer[RTL8139_MIN_RING_SIZE - 2] = RX_STATUS_ROK as u8;
        rx.buffer[RTL8139_MIN_RING_SIZE - 1] = 0;
        rx.buffer[0..2].copy_from_slice(&64u16.to_le_bytes());
        for i in 0..60 { rx.buffer[(i + 2) & (rx.ring_size - 1)] = i as u8; }
        let packet = rx.peek(66).unwrap().unwrap();
        let mut output = [0u8; 60];
        rx.copy_packet(packet, &mut output).unwrap();
        assert_eq!(output[0], 0);
        assert_eq!(output[59], 59);
        assert_eq!(packet.next_cursor, 66);
    }

    #[test]
    fn rejects_corrupt_status_and_oversized_lengths() {
        let mut bytes = [0u8; RTL8139_MIN_RING_SIZE];
        let mut rx = ring(&mut bytes);
        rx.buffer[0..2].copy_from_slice(&2u16.to_le_bytes());
        rx.buffer[2..4].copy_from_slice(&64u16.to_le_bytes());
        let corrupt = rx.peek(68).unwrap().unwrap();
        assert_eq!(
            rx.copy_packet(corrupt, &mut [0u8; 60]),
            Err(Rtl8139Error::ReceiveError(2)),
        );
        rx.commit(corrupt).unwrap();

        rx.reset_cursor();
        rx.buffer[0..2].copy_from_slice(&RX_STATUS_ROK.to_le_bytes());
        rx.buffer[2..4].copy_from_slice(&2048u16.to_le_bytes());
        assert_eq!(rx.peek(68), Err(Rtl8139Error::InvalidPacketLength));
    }

    #[test]
    fn incomplete_packet_is_not_exposed_to_the_consumer() {
        let mut bytes = [0u8; RTL8139_MIN_RING_SIZE];
        let mut rx = ring(&mut bytes);
        rx.buffer[0..2].copy_from_slice(&RX_STATUS_ROK.to_le_bytes());
        rx.buffer[2..4].copy_from_slice(&64u16.to_le_bytes());
        assert_eq!(rx.peek(20).unwrap(), None);
        assert_eq!(rx.cursor(), 0);
    }

    #[test]
    fn stale_packet_token_cannot_advance_the_ring_twice() {
        let mut bytes = [0u8; RTL8139_MIN_RING_SIZE];
        let mut rx = ring(&mut bytes);
        rx.buffer[0..2].copy_from_slice(&RX_STATUS_ROK.to_le_bytes());
        rx.buffer[2..4].copy_from_slice(&64u16.to_le_bytes());
        let packet = rx.peek(68).unwrap().unwrap();
        rx.commit(packet).unwrap();
        assert_eq!(rx.commit(packet), Err(Rtl8139Error::InvalidReadCursor));
    }

    #[test]
    fn rejects_unsupported_ring_geometry() {
        let mut bytes = [0u8; RTL8139_MIN_RING_SIZE];
        let dma = unsafe { DmaBuffer::new_for_test(0x10000, &mut bytes).unwrap() };
        assert!(matches!(unsafe { Rtl8139RxRing::new(dma, 4096) }, Err(Rtl8139Error::InvalidRingSize)));
    }
}
