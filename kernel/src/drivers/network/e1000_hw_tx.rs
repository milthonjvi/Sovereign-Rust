//! E1000 transmit descriptor-ring ownership and publication.

use super::{
    E1000Error, E1000QueueError, E1000Registers, E1000TxQueue, RingCursor,
    TxDescriptor, REG_TDT, TXD_STATUS_DD,
};

impl E1000TxQueue {
    pub(crate) fn new(descriptor_count: usize) -> Result<Self, E1000Error> {
        Ok(Self { cursor: RingCursor::new(descriptor_count)?, clean: RingCursor::new(descriptor_count)?, pending: 0 })
    }

    /// Submit one DMA-mapped frame and publish its descriptor to hardware.
    /// Frame bytes must already be present in the buffer and remain immutable
    /// until reap_one() returns the descriptor index.
    pub(crate) fn submit<R: E1000Registers>(
        &mut self,
        registers: &mut R,
        descriptors: &mut [TxDescriptor],
        buffer_dma: u64,
        length: usize,
    ) -> Result<u16, E1000QueueError<R::Error>> {
        let index = self.cursor.index();
        self.submit_batch(registers, descriptors, &[(buffer_dma, length)])?;
        Ok(index)
    }

    /// Submit a batch with one DMA write barrier and one TDT write.
    /// Before calling, the producer must have established CPU ownership of all
    /// target slots and executed dma_read_barrier() after observing DD. The
    /// internal E1000Device path obtains that ownership through slot discovery.
    pub(crate) fn submit_batch<R: E1000Registers>(
        &mut self,
        registers: &mut R,
        descriptors: &mut [TxDescriptor],
        frames: &[(u64, usize)],
    ) -> Result<usize, E1000QueueError<R::Error>> {
        if descriptors.len() != self.cursor_count() {
            return Err(E1000QueueError::InvalidRing);
        }
        if frames.is_empty() {
            return Ok(0);
        }
        // Bound validation work by the actual queue capacity before walking
        // caller-provided frame metadata.
        if frames.len() > descriptors.len() - 1 - self.pending as usize {
            return Err(E1000QueueError::TxRingFull);
        }
        if frames.iter().any(|(address, length)| {
            *address == 0 || *address & 0xf != 0 || *length == 0 || *length > u16::MAX as usize
        }) {
            return Err(E1000QueueError::InvalidFrameLength);
        }

        let count = descriptors.len();
        let start = self.cursor.index() as usize;
        for offset in 0..frames.len() {
            let index = (start + offset) & (count - 1);
            let descriptor = unsafe { descriptors.as_ptr().add(index) };
            // SAFETY: inspect only the device-written ownership byte in the pinned ring.
            let status = unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*descriptor).status)) };
            if status & TXD_STATUS_DD == 0 {
                return Err(E1000QueueError::TxRingFull);
            }
        }
        // Slot discovery already acquired device writes before the caller
        // copied packet bytes into these buffers. Recheck DD before staging.
        for (offset, &(buffer_dma, length)) in frames.iter().enumerate() {
            let index = (start + offset) & (count - 1);
            let descriptor = super::e1000_descriptor::TxDescriptor::for_frame(buffer_dma, length)
                .map_err(|_| E1000QueueError::InvalidFrameLength)?;
            // SAFETY: this descriptor belongs to the pinned DMA ring supplied by the caller.
            unsafe { core::ptr::write_volatile(descriptors.as_mut_ptr().add(index), descriptor); }
        }
        registers.dma_write_barrier();
        let next = (start + frames.len()) & (count - 1);
        if let Err(error) = registers.write32(REG_TDT, next as u32) {
            for offset in 0..frames.len() {
                let index = (start + offset) & (count - 1);
                // SAFETY: hardware was not notified, so these descriptors remain CPU-owned.
                unsafe {
                    let mut descriptor = core::ptr::read_volatile(descriptors.as_ptr().add(index));
                    descriptor.status = TXD_STATUS_DD;
                    core::ptr::write_volatile(descriptors.as_mut_ptr().add(index), descriptor);
                }
            }
            return Err(E1000QueueError::Register(error));
        }
        for _ in 0..frames.len() {
            self.cursor.advance();
        }
        self.pending += frames.len() as u16;
        Ok(frames.len())
    }

    /// Inspect the next producer slot without transferring ownership.
    ///
    /// The caller may write its matching DMA buffer only after this returns
    /// Some(index). The queue still validates descriptor availability again
    /// during submit(), so all accesses must remain externally serialized.
    pub(crate) fn next_available_slot<R: E1000Registers>(
        &self,
        registers: &mut R,
        descriptors: &[TxDescriptor],
    ) -> Result<Option<u16>, E1000QueueError<R::Error>> {
        if descriptors.len() != self.cursor_count() {
            return Err(E1000QueueError::InvalidRing);
        }
        if self.pending as usize >= descriptors.len() - 1 {
            return Ok(None);
        }
        let index = self.cursor.index() as usize;
        let descriptor = unsafe { descriptors.as_ptr().add(index) };
        // SAFETY: read the device-written ownership byte from the pinned ring.
        let status = unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*descriptor).status)) };
        if status & TXD_STATUS_DD == 0 {
            return Ok(None);
        }
        registers.dma_read_barrier();
        Ok(Some(index as u16))
    }

    /// Collect consecutive free producer slots with one DMA read barrier.
    ///
    /// The returned indices are in submission order and can be passed directly
    /// to submit_batch(). The caller must serialize queue access between this
    /// availability check and publication.
    pub(crate) fn available_slots<R: E1000Registers>(
        &self,
        registers: &mut R,
        descriptors: &[TxDescriptor],
        output: &mut [u16],
    ) -> Result<usize, E1000QueueError<R::Error>> {
        if descriptors.len() != self.cursor_count() {
            return Err(E1000QueueError::InvalidRing);
        }
        let free = descriptors.len() - 1 - self.pending as usize;
        let limit = core::cmp::min(output.len(), free);
        if limit == 0 {
            return Ok(0);
        }

        let count = descriptors.len();
        let start = self.cursor.index() as usize;
        let mut available = 0;
        while available < limit {
            let index = (start + available) & (count - 1);
            let descriptor = unsafe { descriptors.as_ptr().add(index) };
            // SAFETY: read the device-written ownership byte from the pinned ring.
            let status = unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*descriptor).status)) };
            if status & TXD_STATUS_DD == 0 {
                break;
            }
            output[available] = index as u16;
            available += 1;
        }
        if available != 0 {
            registers.dma_read_barrier();
        }
        Ok(available)
    }

    /// Reclaim completed descriptors and collect free producer slots in one scan
    /// phase and one DMA read barrier. Intended for high-throughput batch TX.
    pub(crate) fn reclaim_and_available_slots<R: E1000Registers>(
        &mut self,
        registers: &mut R,
        descriptors: &[TxDescriptor],
        output: &mut [u16],
    ) -> Result<usize, E1000QueueError<R::Error>> {
        if descriptors.len() != self.cursor_count() {
            return Err(E1000QueueError::InvalidRing);
        }

        let mut reclaimed = 0usize;
        while reclaimed < self.pending as usize {
            let index = self.clean.index() as usize;
            let descriptor = unsafe { descriptors.as_ptr().add(index) };
            // SAFETY: read the device-written ownership byte from the pinned ring.
            let status = unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*descriptor).status)) };
            if status & TXD_STATUS_DD == 0 {
                break;
            }
            self.clean.advance();
            reclaimed += 1;
        }
        self.pending -= reclaimed as u16;

        let free = descriptors.len() - 1 - self.pending as usize;
        let limit = core::cmp::min(output.len(), free);
        let count = descriptors.len();
        let start = self.cursor.index() as usize;
        let mut available = 0usize;
        while available < limit {
            let index = (start + available) & (count - 1);
            let descriptor = unsafe { descriptors.as_ptr().add(index) };
            // SAFETY: read the device-written ownership byte from the pinned ring.
            let status = unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*descriptor).status)) };
            if status & TXD_STATUS_DD == 0 {
                break;
            }
            output[available] = index as u16;
            available += 1;
        }

        if reclaimed != 0 || available != 0 {
            registers.dma_read_barrier();
        }
        Ok(available)
    }

    /// Return the next completed descriptor index in submission order.
    /// The caller can use this index to release the matching DMA buffer.
    pub(crate) fn reap_one<R: E1000Registers>(
        &mut self,
        registers: &mut R,
        descriptors: &[TxDescriptor],
    ) -> Result<Option<u16>, E1000QueueError<R::Error>> {
        if descriptors.len() != self.cursor_count() {
            return Err(E1000QueueError::InvalidRing);
        }
        if self.pending == 0 {
            return Ok(None);
        }
        let index = self.clean.index() as usize;
        let descriptor = unsafe { descriptors.as_ptr().add(index) };
        // SAFETY: read the device-written ownership byte from the pinned ring.
        let status = unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*descriptor).status)) };
        if status & TXD_STATUS_DD == 0 {
            return Ok(None);
        }
        registers.dma_read_barrier();
        self.clean.advance();
        self.pending -= 1;
        Ok(Some(index as u16))
    }

    /// Reclaim all currently completed descriptors with one DMA read barrier.
    /// Completion is consumed strictly in submission order.
    pub(crate) fn reap<R: E1000Registers>(
        &mut self,
        registers: &mut R,
        descriptors: &[TxDescriptor],
    ) -> Result<usize, E1000QueueError<R::Error>> {
        if descriptors.len() != self.cursor_count() {
            return Err(E1000QueueError::InvalidRing);
        }
        if self.pending == 0 {
            return Ok(0);
        }
        let mut completed = 0usize;
        while completed < self.pending as usize {
            let index = self.clean.index() as usize;
            let descriptor = unsafe { descriptors.as_ptr().add(index) };
            // SAFETY: read the device-written ownership byte from the pinned ring.
            let status = unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*descriptor).status)) };
            if status & TXD_STATUS_DD == 0 {
                break;
            }
            self.clean.advance();
            completed += 1;
        }
        if completed != 0 {
            registers.dma_read_barrier();
        }
        self.pending -= completed as u16;
        Ok(completed)
    }

    pub(crate) const fn pending_count(&self) -> u16 {
        self.pending
    }

    const fn cursor_count(&self) -> usize {
        self.cursor.mask() as usize + 1
    }
}

