//! Fixed-capacity, write-through block cache.
//!
//! Exclusive mutable access serializes cache operations. Kernel owners may wrap
//! it with the synchronization and I/O queue primitives already in the kernel.
use super::block::{validate_buffer, BlockDevice, BlockError, BlockGeometry, BlockRange};

#[derive(Clone, Copy)]
struct CacheLine { lba: u64, valid: bool, last_used: u64 }
impl CacheLine { const EMPTY: Self = Self { lba: 0, valid: false, last_used: 0 }; }

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CacheStats { pub hits: u64, pub misses: u64, pub evictions: u64, pub bypassed_blocks: u64 }

/// Write-through LRU cache. Storage must contain at least capacity * block_size bytes.
pub struct BlockCache<'a, D, const N: usize> {
    inner: D, lines: [CacheLine; N], storage: &'a mut [u8],
    block_size: usize, clock: u64, stats: CacheStats,
}
impl<'a, D: BlockDevice, const N: usize> BlockCache<'a, D, N> {
    pub fn new(inner: D, storage: &'a mut [u8]) -> Result<Self, BlockError> {
        let geometry = inner.geometry().validate()?;
        let required = N.checked_mul(geometry.block_size as usize).ok_or(BlockError::AddressOverflow)?;
        if storage.len() < required { return Err(BlockError::BufferSizeMismatch); }
        Ok(Self { inner, lines: [CacheLine::EMPTY; N], storage,
            block_size: geometry.block_size as usize, clock: 0, stats: CacheStats::default() })
    }
    pub const fn stats(&self) -> CacheStats { self.stats }
    pub const fn capacity(&self) -> usize { N }
    pub fn inner(&self) -> &D { &self.inner }
    /// Mutations made through this escape hatch require explicit cache invalidation.
    pub fn inner_mut(&mut self) -> &mut D { &mut self.inner }
    pub fn invalidate_all(&mut self) { self.lines.fill(CacheLine::EMPTY); }
    pub fn invalidate_range(&mut self, range: BlockRange) -> Result<(), BlockError> {
        self.inner.geometry().validate_range(range)?;
        let end = range.start_block.checked_add(range.block_count).ok_or(BlockError::AddressOverflow)?;
        for line in &mut self.lines {
            if line.valid && line.lba >= range.start_block && line.lba < end { line.valid = false; }
        }
        Ok(())
    }
    fn touch(&mut self, index: usize) {
        self.clock = self.clock.wrapping_add(1);
        if self.clock == 0 {
            for line in &mut self.lines { line.last_used = 0; }
            self.clock = 1;
        }
        self.lines[index].last_used = self.clock;
    }
    fn find(&self, lba: u64) -> Option<usize> { self.lines.iter().position(|line| line.valid && line.lba == lba) }
    fn slot(&mut self, lba: u64) -> Option<usize> {
        if N == 0 { return None; }
        if let Some(index) = self.lines.iter().position(|line| !line.valid) {
            self.lines[index] = CacheLine { lba, valid: false, last_used: 0 };
            return Some(index);
        }
        let index = self.lines.iter().enumerate().min_by_key(|(_, line)| line.last_used).map(|(i, _)| i)?;
        self.lines[index] = CacheLine { lba, valid: false, last_used: 0 };
        self.stats.evictions = self.stats.evictions.saturating_add(1);
        Some(index)
    }
    fn line_mut(&mut self, index: usize) -> &mut [u8] {
        let start = index * self.block_size;
        &mut self.storage[start..start + self.block_size]
    }
}
impl<D: BlockDevice, const N: usize> BlockDevice for BlockCache<'_, D, N> {
    fn geometry(&self) -> BlockGeometry { self.inner.geometry() }
    fn read_blocks(&mut self, range: BlockRange, output: &mut [u8]) -> Result<(), BlockError> {
        let geometry = self.inner.geometry().validate()?;
        validate_buffer(geometry, range, output.len())?;
        if N == 0 || range.block_count > N as u64 {
            self.stats.bypassed_blocks = self.stats.bypassed_blocks.saturating_add(range.block_count);
            return self.inner.read_blocks(range, output);
        }
        let block_size = self.block_size;
        for index in 0..range.block_count as usize {
            let lba = range.start_block + index as u64;
            let dst = &mut output[index * block_size..(index + 1) * block_size];
            if let Some(line) = self.find(lba) {
                self.stats.hits = self.stats.hits.saturating_add(1);
                self.touch(line);
                dst.copy_from_slice(self.line_mut(line));
            } else {
                self.stats.misses = self.stats.misses.saturating_add(1);
                if let Some(line) = self.slot(lba) {
                    let start = line * block_size;
                    self.inner.read_blocks(BlockRange::new(lba, 1), &mut self.storage[start..start + block_size])?;
                    self.lines[line].valid = true;
                    self.touch(line);
                    dst.copy_from_slice(&self.storage[start..start + block_size]);
                } else {
                    self.inner.read_blocks(BlockRange::new(lba, 1), dst)?;
                }
            }
        }
        Ok(())
    }
    fn write_blocks(&mut self, range: BlockRange, input: &[u8]) -> Result<(), BlockError> {
        let geometry = self.inner.geometry().validate()?;
        validate_buffer(geometry, range, input.len())?;
        if let Err(error) = self.inner.write_blocks(range, input) {
            let _ = self.invalidate_range(range);
            return Err(error);
        }
        let block_size = self.block_size;
        let end = range.start_block + range.block_count;
        for line in 0..N {
            let lba = self.lines[line].lba;
            if self.lines[line].valid && lba >= range.start_block && lba < end {
                let offset = (lba - range.start_block) as usize * block_size;
                self.line_mut(line).copy_from_slice(&input[offset..offset + block_size]);
                self.touch(line);
            }
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<(), BlockError> { self.inner.flush() }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Disk { bytes: [u8; 2048], reads: u32 }
    impl BlockDevice for Disk {
        fn geometry(&self) -> BlockGeometry { BlockGeometry { block_count: 4, block_size: 512 } }
        fn read_blocks(&mut self, r: BlockRange, out: &mut [u8]) -> Result<(), BlockError> {
            validate_buffer(self.geometry(), r, out.len())?;
            self.reads += 1;
            let start = r.start_block as usize * 512;
            out.copy_from_slice(&self.bytes[start..start + out.len()]);
            Ok(())
        }
        fn write_blocks(&mut self, r: BlockRange, input: &[u8]) -> Result<(), BlockError> {
            validate_buffer(self.geometry(), r, input.len())?;
            let start = r.start_block as usize * 512;
            self.bytes[start..start + input.len()].copy_from_slice(input);
            Ok(())
        }
        fn flush(&mut self) -> Result<(), BlockError> { Ok(()) }
    }
    #[test]
    fn read_hit_avoids_second_device_read() {
        let disk = Disk { bytes: [0x5A; 2048], reads: 0 };
        let mut storage = [0u8; 1024];
        let mut cache = BlockCache::<_, 2>::new(disk, &mut storage).unwrap();
        let mut out = [0u8; 512];
        cache.read_blocks(BlockRange::new(1,1), &mut out).unwrap();
        cache.read_blocks(BlockRange::new(1,1), &mut out).unwrap();
        assert_eq!(cache.inner().reads, 1);
        assert_eq!(cache.stats().hits, 1);
        assert_eq!(cache.stats().misses, 1);
    }
    #[test]
    fn write_through_updates_cached_copy() {
        let disk = Disk { bytes: [0; 2048], reads: 0 };
        let mut storage = [0u8; 512];
        let mut cache = BlockCache::<_, 1>::new(disk, &mut storage).unwrap();
        let mut out = [0u8; 512];
        cache.read_blocks(BlockRange::new(0,1), &mut out).unwrap();
        let input = [0xA5u8; 512];
        cache.write_blocks(BlockRange::new(0,1), &input).unwrap();
        out.fill(0);
        cache.read_blocks(BlockRange::new(0,1), &mut out).unwrap();
        assert_eq!(out, input);
        assert_eq!(cache.inner().bytes[..512], input);
    }
    #[test]
    fn large_reads_bypass_cache() {
        let disk = Disk { bytes: [0x11; 2048], reads: 0 };
        let mut storage = [0u8; 512];
        let mut cache = BlockCache::<_, 1>::new(disk, &mut storage).unwrap();
        let mut out = [0u8; 1024];
        cache.read_blocks(BlockRange::new(0,2), &mut out).unwrap();
        assert_eq!(cache.inner().reads, 1);
        assert_eq!(cache.stats().bypassed_blocks, 2);
    }
}
