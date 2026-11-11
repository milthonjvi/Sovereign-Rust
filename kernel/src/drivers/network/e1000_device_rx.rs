//! E1000 receive fast paths: bounded polling, batch copy, and borrowed frames.

use crate::drivers::api::DriverError;
use super::e1000::{RxDescriptor};
use super::e1000_device::{map_queue_error, queue_error_is_fatal, E1000Device, E1000_MAX_FRAME_SIZE};
use super::e1000_descriptor::RxCompletion;
use super::e1000_hw::E1000Registers;

impl<'a, 'dma, R: E1000Registers> E1000Device<'a, 'dma, R> {
    /// Process completed RX frames directly from their DMA buffers without copying.
    ///
    /// The callback runs while the current buffer is CPU-owned. It must be bounded
    /// and nonblocking; its slice cannot outlive the call. The descriptor is recycled
    /// immediately after the callback returns. At most one complete RX ring is
    /// examined per invocation.
    pub fn receive_with<F>(&mut self, mut consume: F) -> Result<usize, DriverError>
    where
        F: FnMut(&[u8]),
    {
        if !self.ready {
            return Err(DriverError::NotReady);
        }

        let mut delivered = 0usize;
        for _ in 0..self.rx_descriptors.len() {
            let polled = match self.rx_queue.poll(self.registers, self.rx_descriptors) {
                Ok(polled) => polled,
                Err(error) => {
                    if queue_error_is_fatal(&error) {
                        self.ready = false;
                        self.faulted = true;
                    }
                    return Err(map_queue_error(error));
                }
            };
            let Some((index, completion)) = polled else {
                break;
            };

            let slot = index as usize;
            let was_fragmented = self.dropping_rx_frame;
            if !completion.end_of_packet {
                self.dropping_rx_frame = true;
            } else {
                self.dropping_rx_frame = false;
            }

            let length = completion.length as usize;
            let valid = !was_fragmented
                && completion.end_of_packet
                && completion.errors == 0
                && length != 0
                && length <= E1000_MAX_FRAME_SIZE
                && length <= self.rx_buffers[slot].len();
            if valid {
                consume(&self.rx_buffers[slot].as_slice()[..length]);
                delivered += 1;
            }
            self.recycle_rx(index)?;
        }
        Ok(delivered)
    }


    /// Receive several completed Ethernet frames with one descriptor scan
    /// barrier and one RX-tail MMIO update. Invalid, fragmented, and undersized
    /// frames are dropped; valid frames are packed into the leading output slots.
    /// All scratch storage is supplied by the caller and no allocation occurs.
    /// Process a batch of completed RX frames directly from DMA memory.
    ///
    /// This combines zero-copy delivery with one RX ownership barrier and one
    /// tail-register update for the batch. The callback must be bounded and
    /// nonblocking; each borrowed frame expires before its descriptor is recycled.
    pub fn receive_with_batch<F>(
        &mut self,
        completions: &mut [(u16, RxCompletion)],
        recycle_backup: &mut [RxDescriptor],
        mut consume: F,
    ) -> Result<usize, DriverError>
    where
        F: FnMut(&[u8]),
    {
        if !self.ready {
            return Err(DriverError::NotReady);
        }
        let limit = core::cmp::min(
            core::cmp::min(completions.len(), recycle_backup.len()),
            self.rx_descriptors.len(),
        );
        if limit == 0 {
            return Ok(0);
        }

        let received = match self.rx_queue.poll_batch(
            self.registers,
            self.rx_descriptors,
            &mut completions[..limit],
        ) {
            Ok(received) => received,
            Err(error) => {
                if queue_error_is_fatal(&error) {
                    self.ready = false;
                    self.faulted = true;
                }
                return Err(map_queue_error(error));
            }
        };
        if received == 0 {
            return Ok(0);
        }

        let mut delivered = 0usize;
        for (index, completion) in completions[..received].iter() {
            let slot = *index as usize;
            if slot >= self.rx_buffers.len() {
                self.ready = false;
                self.faulted = true;
                return Err(DriverError::HardwareFault);
            }
            let was_fragmented = self.dropping_rx_frame;
            self.dropping_rx_frame = !completion.end_of_packet;
            let length = completion.length as usize;
            let valid = !was_fragmented
                && completion.end_of_packet
                && completion.errors == 0
                && length != 0
                && length <= E1000_MAX_FRAME_SIZE
                && length <= self.rx_buffers[slot].len();
            if valid {
                consume(&self.rx_buffers[slot].as_slice()[..length]);
                delivered += 1;
            }
        }

        if let Err(error) = self.rx_queue.recycle_batch(
            self.registers,
            self.rx_descriptors,
            &completions[..received],
            &mut recycle_backup[..received],
        ) {
            self.ready = false;
            self.faulted = true;
            return Err(map_queue_error(error));
        }
        Ok(delivered)
    }

    pub fn receive_batch(
        &mut self,
        outputs: &mut [&mut [u8]],
        completions: &mut [(u16, RxCompletion)],
        recycle_backup: &mut [RxDescriptor],
    ) -> Result<usize, DriverError> {
        if !self.ready {
            return Err(DriverError::NotReady);
        }
        let limit = core::cmp::min(outputs.len(), completions.len());
        if limit == 0 {
            return Ok(0);
        }
        if recycle_backup.len() < limit {
            return Err(DriverError::InvalidConfiguration);
        }

        let received = match self.rx_queue.poll_batch(
            self.registers,
            self.rx_descriptors,
            &mut completions[..limit],
        ) {
            Ok(received) => received,
            Err(error) => {
                if queue_error_is_fatal(&error) {
                    self.ready = false;
                    self.faulted = true;
                }
                return Err(map_queue_error(error));
            }
        };
        if received == 0 {
            return Ok(0);
        }

        let mut delivered = 0usize;
        for item in &completions[..received] {
            let (index, completion) = *item;
            let slot = index as usize;
            let was_fragmented = self.dropping_rx_frame;
            self.dropping_rx_frame = !completion.end_of_packet;
            let length = completion.length as usize;
            let valid = !was_fragmented
                && completion.end_of_packet
                && completion.errors == 0
                && length != 0
                && length <= self.rx_buffers[slot].len()
                && length <= E1000_MAX_FRAME_SIZE;
            if !valid || length > outputs[delivered].len() {
                continue;
            }

            outputs[delivered][..length]
                .copy_from_slice(&self.rx_buffers[slot].as_slice()[..length]);
            delivered += 1;
        }

        if let Err(error) = self.rx_queue.recycle_batch(
            self.registers,
            self.rx_descriptors,
            &completions[..received],
            &mut recycle_backup[..received],
        ) {
            // Fail closed: the descriptors remain CPU-owned after a failed RDT
            // write, and the caller must reinitialize the device before polling.
            self.ready = false;
            self.faulted = true;
            return Err(map_queue_error(error));
        }
        Ok(delivered)
    }

    pub(super) fn recycle_rx(&mut self, index: u16) -> Result<(), DriverError> {
        match self.rx_queue.recycle(self.registers, self.rx_descriptors, index) {
            Ok(()) => Ok(()),
            Err(error) => {
                // A failed RDT write leaves software ownership unchanged. Stop
                // the adapter so a later poll cannot skip the unrecycled slot.
                self.ready = false;
                self.faulted = true;
                Err(map_queue_error(error))
            }
        }
    }

}
