//! E1000 receive descriptor-ring polling and recycling.

use super::*;

impl E1000RxQueue {
    pub(crate) fn new(descriptor_count: u16) -> Result<Self, E1000Error> {
        Ok(Self { cursor: RingCursor::new(descriptor_count)?, recycle: RingCursor::new(descriptor_count)?, outstanding: 0 })
    }

    /// Consume one completed descriptor without returning its buffer to hardware.
    /// The caller owns the packet buffer until recycle() succeeds.
    pub(crate) fn poll<R: E1000Registers>(
        &mut self,
        registers: &mut R,
        descriptors: &[RxDescriptor],
    ) -> Result<Option<(u16, super::e1000_descriptor::RxCompletion)>, E1000QueueError<R::Error>> {
        let mut result = [(0u16, super::e1000_descriptor::RxCompletion {
            length: 0, end_of_packet: false, errors: 0,
        }); 1];
        if self.poll_batch(registers, descriptors, &mut result)? == 0 {
            Ok(None)
        } else {
            Ok(Some(result[0]))
        }
    }

    /// Drain a contiguous batch of completed RX descriptors with one DMA
    /// read barrier. Output is caller-owned; no heap allocation is performed.
    pub(crate) fn poll_batch<R: E1000Registers>(
        &mut self,
        registers: &mut R,
        descriptors: &[RxDescriptor],
        output: &mut [(u16, super::e1000_descriptor::RxCompletion)],
    ) -> Result<usize, E1000QueueError<R::Error>> {
        if descriptors.len() != self.cursor_count() {
            return Err(E1000QueueError::InvalidRing);
        }
        let available = descriptors.len() - self.outstanding as usize;
        let limit = core::cmp::min(output.len(), available);
        if limit == 0 {
            return Ok(0);
        }
        let mut scan_cursor = self.cursor;
        let mut count = 0;
        while count < limit {
            let index = scan_cursor.index() as usize;
            let descriptor = unsafe { descriptors.as_ptr().add(index) };
            // SAFETY: read the device-written DD byte from the pinned ring.
            let status = unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*descriptor).status)) };
            if status & super::e1000::RXD_STATUS_DD == 0 {
                break;
            }
            output[count].0 = index as u16;
            scan_cursor.advance();
            count += 1;
        }
        if count == 0 {
            return Ok(0);
        }
        // DD is written after the packet and descriptor metadata. Acquire the
        // device writes before reading lengths/errors or exposing RX buffers.
        registers.dma_read_barrier();
        for item in output.iter_mut().take(count) {
            let index = item.0 as usize;
            // SAFETY: DD was observed and the DMA read barrier has completed.
            let descriptor = unsafe { core::ptr::read_volatile(descriptors.as_ptr().add(index)) };
            let completion = descriptor.completion().ok_or(E1000QueueError::InvalidRing)?;
            item.1 = completion;
        }
        self.cursor = scan_cursor;
        self.outstanding += count as u16;
        Ok(count)
    }

    /// Recycle a contiguous batch of processed RX descriptors with one RDT write.
    ///
    /// `backup` must have room for every completion. The descriptors are saved
    /// before rearming so a failed MMIO tail update can restore CPU-visible
    /// completion metadata and leave queue ownership unchanged.
    pub(crate) fn recycle_batch<R: E1000Registers>(
        &mut self,
        registers: &mut R,
        descriptors: &mut [RxDescriptor],
        completed: &[(u16, super::e1000_descriptor::RxCompletion)],
        backup: &mut [RxDescriptor],
    ) -> Result<usize, E1000QueueError<R::Error>> {
        if descriptors.len() != self.cursor_count()
            || completed.len() > backup.len()
            || completed.len() > self.outstanding as usize
        {
            return Err(E1000QueueError::InvalidRing);
        }
        if completed.is_empty() {
            return Ok(0);
        }

        let count = descriptors.len();
        for (offset, (index, _)) in completed.iter().enumerate() {
            let expected = (self.recycle.index() as usize + offset) & (count - 1);
            if *index as usize != expected {
                return Err(E1000QueueError::InvalidRing);
            }
        }

        // Preflight every descriptor before changing any of them. This prevents
        // callers from recycling a slot that hardware has not completed yet.
        for (offset, (index, _)) in completed.iter().enumerate() {
            let slot = *index as usize;
            // SAFETY: read-only inspection of a pinned DMA descriptor owned by CPU.
            let original = unsafe { core::ptr::read_volatile(descriptors.as_ptr().add(slot)) };
            if original.status & super::e1000::RXD_STATUS_DD == 0 {
                return Err(E1000QueueError::InvalidRing);
            }
            backup[offset] = original;
        }
        for (offset, (index, _)) in completed.iter().enumerate() {
            let slot = *index as usize;
            let rearmed = RxDescriptor::for_buffer(backup[offset].buffer_address);
            // SAFETY: rearm only descriptors whose packets have already been consumed.
            unsafe { core::ptr::write_volatile(descriptors.as_mut_ptr().add(slot), rearmed); }
        }
        registers.dma_write_barrier();
        let tail = completed[completed.len() - 1].0 as u32;
        if let Err(error) = registers.write32(REG_RDT, tail) {
            for (offset, (index, _)) in completed.iter().enumerate() {
                // SAFETY: failed tail publication leaves all descriptors CPU-owned.
                unsafe {
                    core::ptr::write_volatile(
                        descriptors.as_mut_ptr().add(*index as usize), backup[offset],
                    );
                }
            }
            return Err(E1000QueueError::Register(error));
        }

        for _ in completed {
            self.recycle.advance();
        }
        self.outstanding -= completed.len() as u16;
        Ok(completed.len())
    }

    /// Return a processed RX buffer to hardware. Recycling must follow descriptor
    /// order so RDT never advances past an unreleased buffer.
    pub(crate) fn recycle<R: E1000Registers>(
        &mut self,
        registers: &mut R,
        descriptors: &mut [RxDescriptor],
        index: u16,
    ) -> Result<(), E1000QueueError<R::Error>> {
        if descriptors.len() != self.cursor_count() || self.outstanding == 0
            || index != self.recycle.index()
        {
            return Err(E1000QueueError::InvalidRing);
        }
        let slot = index as usize;
        // SAFETY: this slot is pinned DMA memory and is CPU-owned until recycled.
        let completed_descriptor = unsafe { core::ptr::read_volatile(descriptors.as_ptr().add(slot)) };
        if completed_descriptor.status & super::e1000::RXD_STATUS_DD == 0 {
            return Err(E1000QueueError::InvalidRing);
        }
        let rearmed = RxDescriptor::for_buffer(completed_descriptor.buffer_address);
        // SAFETY: write the reset descriptor before advancing the hardware tail.
        unsafe { core::ptr::write_volatile(descriptors.as_mut_ptr().add(slot), rearmed); }
        registers.dma_write_barrier();
        if let Err(error) = registers.write32(REG_RDT, index as u32) {
            // SAFETY: restore the completed descriptor because hardware did not accept the tail update.
            unsafe { core::ptr::write_volatile(descriptors.as_mut_ptr().add(slot), completed_descriptor); }
            return Err(E1000QueueError::Register(error));
        }
        self.recycle.advance();
        self.outstanding -= 1;
        Ok(())
    }

    const fn cursor_count(&self) -> usize {
        self.cursor.mask() as usize + 1
    }
}
