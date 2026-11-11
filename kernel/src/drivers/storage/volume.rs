//! Fixed-capacity volume registry for one block device.
//!
//! No heap allocation or global lock is imposed. The subsystem owner provides
//! exclusive access using the kernel synchronization primitives when ready.
use super::block::BlockGeometry;
use super::partition::{Partition, PartitionError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VolumeError { Partition(PartitionError), Duplicate, Overlap, NotFound, Full, GenerationOverflow }

pub struct VolumeTable<const N: usize> {
    entries: [Option<Partition>; N],
    len: usize,
    generation: u64,
}

impl<const N: usize> VolumeTable<N> {
    pub const fn new() -> Self { Self { entries: [None; N], len: 0, generation: 0 } }
    pub const fn len(&self) -> usize { self.len }
    pub const fn is_empty(&self) -> bool { self.len == 0 }
    pub const fn capacity(&self) -> usize { N }
    pub const fn generation(&self) -> u64 { self.generation }
    pub fn get(&self, index: usize) -> Option<Partition> { if index < self.len { self.entries[index] } else { None } }
    pub fn iter(&self) -> impl Iterator<Item = &Partition> { self.entries[..self.len].iter().filter_map(Option::as_ref) }

    pub fn insert(&mut self, partition: Partition, geometry: BlockGeometry) -> Result<usize, VolumeError> {
        let partition = partition.validate(geometry).map_err(VolumeError::Partition)?;
        let end = partition.end_lba_exclusive().ok_or(VolumeError::Partition(PartitionError::InvalidMetadata))?;
        for existing in self.iter() {
            if *existing == partition { return Err(VolumeError::Duplicate); }
            let existing_end = existing.end_lba_exclusive().ok_or(VolumeError::Partition(PartitionError::InvalidMetadata))?;
            if partition.first_lba < existing_end && existing.first_lba < end { return Err(VolumeError::Overlap); }
        }
        if self.len == N { return Err(VolumeError::Full); }
        let generation = self.generation.checked_add(1).ok_or(VolumeError::GenerationOverflow)?;
        let index = self.len;
        self.entries[index] = Some(partition);
        self.len += 1;
        self.generation = generation;
        Ok(index)
    }

    pub fn remove(&mut self, first_lba: u64) -> Result<Partition, VolumeError> {
        let index = self.entries[..self.len].iter().position(|entry| entry.map(|p| p.first_lba == first_lba).unwrap_or(false)).ok_or(VolumeError::NotFound)?;
        let generation = self.generation.checked_add(1).ok_or(VolumeError::GenerationOverflow)?;
        let removed = self.entries[index].take().ok_or(VolumeError::NotFound)?;
        for slot in index..self.len - 1 { self.entries[slot] = self.entries[slot + 1]; }
        self.len -= 1;
        self.entries[self.len] = None;
        self.generation = generation;
        Ok(removed)
    }

    /// Transactionally replace all registered volumes from a caller-owned list.
    pub fn replace_all(&mut self, partitions: &[Option<Partition>], geometry: BlockGeometry) -> Result<usize, VolumeError> {
        if partitions.iter().flatten().count() > N { return Err(VolumeError::Full); }
        let mut staged = [None; N];
        let mut count = 0;
        for partition in partitions.iter().flatten().copied() {
            let partition = partition.validate(geometry).map_err(VolumeError::Partition)?;
            let end = partition.end_lba_exclusive().ok_or(VolumeError::Partition(PartitionError::InvalidMetadata))?;
            for existing in staged[..count].iter().flatten() {
                if *existing == partition { return Err(VolumeError::Duplicate); }
                let existing_end = existing.end_lba_exclusive().ok_or(VolumeError::Partition(PartitionError::InvalidMetadata))?;
                if partition.first_lba < existing_end && existing.first_lba < end { return Err(VolumeError::Overlap); }
            }
            staged[count] = Some(partition);
            count += 1;
        }
        let generation = self.generation.checked_add(1).ok_or(VolumeError::GenerationOverflow)?;
        self.entries = staged;
        self.len = count;
        self.generation = generation;
        Ok(count)
    }
}
impl<const N: usize> Default for VolumeTable<N> { fn default() -> Self { Self::new() } }

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::partition::{PartitionScheme, PartitionType};
    const GEO: BlockGeometry = BlockGeometry { block_count: 256, block_size: 512 };
    fn part(start: u64, count: u64) -> Partition {
        Partition { scheme: PartitionScheme::MbrPrimary, first_lba: start, block_count: count, kind: PartitionType::Mbr { partition_type: 0x83, bootable: false } }
    }
    #[test] fn insert_remove_and_generation() {
        let mut t = VolumeTable::<4>::new();
        assert_eq!(t.insert(part(10, 20), GEO), Ok(0));
        assert_eq!(t.insert(part(40, 10), GEO), Ok(1));
        assert_eq!(t.generation(), 2);
        assert_eq!(t.remove(10), Ok(part(10, 20)));
        assert_eq!(t.get(0), Some(part(40, 10)));
        assert_eq!(t.generation(), 3);
    }
    #[test] fn reject_duplicate_overlap_and_full() {
        let mut t = VolumeTable::<1>::new();
        t.insert(part(10, 20), GEO).unwrap();
        assert_eq!(t.insert(part(10, 20), GEO), Err(VolumeError::Duplicate));
        assert_eq!(t.insert(part(20, 10), GEO), Err(VolumeError::Overlap));
        assert_eq!(t.insert(part(40, 10), GEO), Err(VolumeError::Full));
        assert_eq!(t.len(), 1);
    }
    #[test] fn replace_is_transactional() {
        let mut t = VolumeTable::<4>::new();
        t.insert(part(10, 10), GEO).unwrap();
        let bad = [Some(part(30, 10)), Some(part(35, 10))];
        assert_eq!(t.replace_all(&bad, GEO), Err(VolumeError::Overlap));
        assert_eq!(t.get(0), Some(part(10, 10)));
        let good = [Some(part(50, 10)), Some(part(70, 10))];
        assert_eq!(t.replace_all(&good, GEO), Ok(2));
    }
    #[test] fn zero_capacity_is_valid() { assert!(VolumeTable::<0>::new().is_empty()); }
}
