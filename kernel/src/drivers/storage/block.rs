//! Transport-independent block-device contract and request validation.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockGeometry { pub block_count: u64, pub block_size: u32 }

impl BlockGeometry {
    pub const fn validate(self) -> Result<Self, BlockError> {
        if self.block_count == 0 || self.block_size < 512 || self.block_size > 65_536 || !self.block_size.is_power_of_two() { return Err(BlockError::InvalidGeometry); }
        Ok(self)
    }
    pub const fn validate_range(self, range: BlockRange) -> Result<(), BlockError> {
        if range.block_count == 0 { return Err(BlockError::EmptyRequest); }
        match range.start_block.checked_add(range.block_count) {
            Some(end) if end <= self.block_count => Ok(()),
            Some(_) => Err(BlockError::OutOfBounds),
            None => Err(BlockError::AddressOverflow),
        }
    }
    pub const fn byte_len(self, count: u64) -> Result<usize, BlockError> {
        let bytes = match count.checked_mul(self.block_size as u64) { Some(v) => v, None => return Err(BlockError::AddressOverflow) };
        if bytes > usize::MAX as u64 { return Err(BlockError::AddressOverflow); }
        Ok(bytes as usize)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockRange { pub start_block: u64, pub block_count: u64 }
impl BlockRange { pub const fn new(start_block: u64, block_count: u64) -> Self { Self { start_block, block_count } } }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockError { InvalidGeometry, EmptyRequest, OutOfBounds, AddressOverflow, BufferSizeMismatch, InvalidTransferLimit, NotReady, UnsupportedOperation, ResourceUnavailable, Timeout, DeviceFault }

pub const fn validate_buffer(geometry: BlockGeometry, range: BlockRange, buffer_len: usize) -> Result<(), BlockError> {
    geometry.validate()?;
    geometry.validate_range(range)?;
    if geometry.byte_len(range.block_count)? != buffer_len { return Err(BlockError::BufferSizeMismatch); }
    Ok(())
}

/// DMA transports must use buffers owned and mapped by the memory subsystem.
pub trait BlockDevice {
    fn geometry(&self) -> BlockGeometry;
    fn read_blocks(&mut self, range: BlockRange, buffer: &mut [u8]) -> Result<(), BlockError>;
    fn write_blocks(&mut self, range: BlockRange, buffer: &[u8]) -> Result<(), BlockError>;
    fn flush(&mut self) -> Result<(), BlockError>;
    /// Maximum logical blocks accepted by one transport command.
    fn max_transfer_blocks(&self) -> u64 { DEFAULT_MAX_TRANSFER_BLOCKS }
}

/// Conservative default command size for transports without a tighter limit.
pub const DEFAULT_MAX_TRANSFER_BLOCKS: u64 = u32::MAX as u64;

/// Read a validated range in transport-sized commands using a caller-owned buffer.
/// Completed earlier chunks remain in the destination if a later command fails.
pub fn read_blocks_chunked<D: BlockDevice>(device: &mut D, range: BlockRange, buffer: &mut [u8]) -> Result<(), BlockError> {
    let geometry = device.geometry().validate()?;
    validate_buffer(geometry, range, buffer.len())?;
    let limit = device.max_transfer_blocks();
    if limit == 0 { return Err(BlockError::InvalidTransferLimit); }
    let mut completed = 0u64;
    while completed < range.block_count {
        let count = core::cmp::min(range.block_count - completed, limit);
        let offset = geometry.byte_len(completed)?;
        let bytes = geometry.byte_len(count)?;
        let end = offset.checked_add(bytes).ok_or(BlockError::AddressOverflow)?;
        let lba = range.start_block.checked_add(completed).ok_or(BlockError::AddressOverflow)?;
        device.read_blocks(BlockRange::new(lba, count), &mut buffer[offset..end])?;
        completed += count;
    }
    Ok(())
}

/// Write a validated range in transport-sized commands using a caller-owned buffer.
/// Completed earlier chunks may already be persistent if a later command fails.
pub fn write_blocks_chunked<D: BlockDevice>(device: &mut D, range: BlockRange, buffer: &[u8]) -> Result<(), BlockError> {
    let geometry = device.geometry().validate()?;
    validate_buffer(geometry, range, buffer.len())?;
    let limit = device.max_transfer_blocks();
    if limit == 0 { return Err(BlockError::InvalidTransferLimit); }
    let mut completed = 0u64;
    while completed < range.block_count {
        let count = core::cmp::min(range.block_count - completed, limit);
        let offset = geometry.byte_len(completed)?;
        let bytes = geometry.byte_len(count)?;
        let end = offset.checked_add(bytes).ok_or(BlockError::AddressOverflow)?;
        let lba = range.start_block.checked_add(completed).ok_or(BlockError::AddressOverflow)?;
        device.write_blocks(BlockRange::new(lba, count), &buffer[offset..end])?;
        completed += count;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const GEO: BlockGeometry = BlockGeometry { block_count: 1024, block_size: 512 };
    #[test] fn valid_geometry_and_range() { assert_eq!(GEO.validate(), Ok(GEO)); assert_eq!(GEO.validate_range(BlockRange::new(8, 4)), Ok(())); }
    #[test] fn rejects_out_of_bounds() { assert_eq!(GEO.validate_range(BlockRange::new(1023, 2)), Err(BlockError::OutOfBounds)); }
    #[test] fn rejects_overflow() { assert_eq!(GEO.validate_range(BlockRange::new(u64::MAX, 2)), Err(BlockError::AddressOverflow)); }
    #[test] fn rejects_empty_range() { assert_eq!(GEO.validate_range(BlockRange::new(0, 0)), Err(BlockError::EmptyRequest)); }
    #[test] fn validates_exact_buffer() { assert_eq!(validate_buffer(GEO, BlockRange::new(2, 2), 1024), Ok(())); assert_eq!(validate_buffer(GEO, BlockRange::new(2, 2), 512), Err(BlockError::BufferSizeMismatch)); }
    #[test] fn rejects_invalid_geometry() { assert_eq!(BlockGeometry { block_count: 10, block_size: 1000 }.validate(), Err(BlockError::InvalidGeometry)); }
}
