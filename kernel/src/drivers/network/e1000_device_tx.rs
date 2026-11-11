//! E1000 transmit fast paths: direct DMA construction and batched publication.

use crate::drivers::api::DriverError;
use super::e1000_device::{map_queue_error, queue_error_is_fatal, E1000Device, E1000_MAX_FRAME_SIZE};
use super::e1000_hw::E1000Registers;

impl<'a, 'dma, R: E1000Registers> E1000Device<'a, 'dma, R> {
    /// Fill an available DMA TX buffer directly, avoiding an intermediate frame copy.
    ///
    /// The closure receives the selected buffer and returns the number of bytes
    /// written. It must initialize every byte in the returned prefix and must not
    /// retain the buffer borrow or access the NIC reentrantly.
    pub fn transmit_with<F>(&mut self, fill: F) -> Result<(), DriverError>
    where
        F: FnOnce(&mut [u8]) -> usize,
    {
        if !self.ready {
            return Err(DriverError::NotReady);
        }

        let mut available = [u16::MAX; 1];
        let count = match self.tx_queue.reclaim_and_available_slots(
            self.registers,
            self.tx_descriptors,
            &mut available,
        ) {
            Ok(count) => count,
            Err(error) => {
                if queue_error_is_fatal(&error) {
                    self.ready = false;
                    self.faulted = true;
                }
                return Err(map_queue_error(error));
            }
        };
        if count == 0 {
            return Err(DriverError::ResourceUnavailable);
        }

        let index = available[0] as usize;
        let buffer = &mut self.tx_buffers[index];
        let length = fill(buffer.as_mut_slice());
        if length == 0 || length > E1000_MAX_FRAME_SIZE || length > buffer.len() {
            return Err(DriverError::InvalidConfiguration);
        }
        if let Err(error) = self.tx_queue.submit(
            self.registers,
            self.tx_descriptors,
            buffer.dma_address(),
            length,
        ) {
            if queue_error_is_fatal(&error) {
                self.ready = false;
                self.faulted = true;
            }
            return Err(map_queue_error(error));
        }
        Ok(())
    }


    /// Build and publish a TX batch directly in its DMA buffers.
    ///
    /// The builder is called only after the complete batch has available ring
    /// slots. If any builder rejects a frame, no descriptor in the batch is
    /// published. Each callback must initialize its returned prefix, be bounded,
    /// and must not re-enter this device.
    pub fn transmit_batch_with<F>(
        &mut self,
        count: usize,
        slots: &mut [u16],
        submissions: &mut [(u64, usize)],
        mut fill: F,
    ) -> Result<usize, DriverError>
    where
        F: FnMut(usize, &mut [u8]) -> Result<usize, DriverError>,
    {
        if !self.ready {
            return Err(DriverError::NotReady);
        }
        if count == 0 {
            return Ok(0);
        }
        if count > self.tx_descriptors.len() - 1 {
            return Err(DriverError::ResourceUnavailable);
        }
        if count > slots.len() || count > submissions.len() {
            return Err(DriverError::InvalidConfiguration);
        }

        let available = match self.tx_queue.reclaim_and_available_slots(
            self.registers,
            self.tx_descriptors,
            &mut slots[..count],
        ) {
            Ok(available) => available,
            Err(error) => {
                if queue_error_is_fatal(&error) {
                    self.ready = false;
                    self.faulted = true;
                }
                return Err(map_queue_error(error));
            }
        };
        if available != count {
            return Err(DriverError::ResourceUnavailable);
        }

        for slot in slots[..count].iter().copied() {
            let index = slot as usize;
            if index >= self.tx_buffers.len() {
                self.ready = false;
                self.faulted = true;
                return Err(DriverError::HardwareFault);
            }
            if self.tx_buffers[index].len() < E1000_MAX_FRAME_SIZE {
                return Err(DriverError::InvalidConfiguration);
            }
        }

        for index in 0..count {
            let slot = slots[index] as usize;
            let buffer = &mut self.tx_buffers[slot];
            let length = fill(index, buffer.as_mut_slice())?;
            if length == 0 || length > E1000_MAX_FRAME_SIZE || length > buffer.len() {
                return Err(DriverError::InvalidConfiguration);
            }
            submissions[index] = (buffer.dma_address(), length);
        }

        match self.tx_queue.submit_batch(
            self.registers,
            self.tx_descriptors,
            &submissions[..count],
        ) {
            Ok(submitted) => Ok(submitted),
            Err(error) => {
                if queue_error_is_fatal(&error) {
                    self.ready = false;
                    self.faulted = true;
                }
                Err(map_queue_error(error))
            }
        }
    }

    /// Copy and publish several frames as one hardware batch.
    ///
    /// The slots and submissions arguments are caller-owned scratch arrays.
    /// The complete batch is preflighted before any frame is copied into DMA.
    pub fn transmit_batch(
        &mut self,
        frames: &[&[u8]],
        slots: &mut [u16],
        submissions: &mut [(u64, usize)],
    ) -> Result<usize, DriverError> {
        if !self.ready {
            return Err(DriverError::NotReady);
        }
        if frames.is_empty() {
            return Ok(0);
        }
        if frames.len() > self.tx_descriptors.len() - 1 {
            return Err(DriverError::ResourceUnavailable);
        }
        if frames.iter().any(|frame| {
            frame.is_empty() || frame.len() > E1000_MAX_FRAME_SIZE
        }) {
            return Err(DriverError::InvalidConfiguration);
        }

        self.transmit_batch_with(
            frames.len(),
            slots,
            submissions,
            |index, buffer| {
                let frame = frames[index];
                buffer[..frame.len()].copy_from_slice(frame);
                Ok(frame.len())
            },
        )
    }


}
