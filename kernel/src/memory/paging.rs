//! Read-only inspection of the active x86_64 recursive page-table window.
//!
//! Sovereign Boot reserves top-level slot 510 as a supervisor-only recursive
//! mapping. This module translates virtual addresses without assuming that
//! physical RAM has a direct-map window.

const PRESENT: u64 = 1 << 0;
const WRITABLE: u64 = 1 << 1;
const USER: u64 = 1 << 2;
const HUGE_PAGE: u64 = 1 << 7;
const GLOBAL: u64 = 1 << 8;
const NO_EXECUTE: u64 = 1 << 63;
const ADDRESS_MASK: u64 = 0x000F_FFFF_FFFF_F000;
const PAGE_SIZE: u64 = 4096;
const PAGE_SHIFT: u64 = 12;
const INDEX_MASK: u64 = 0x1ff;
pub const RECURSIVE_SLOT: usize = 510;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PagingError {
    UnsupportedPagingLevels,
    InvalidRecursiveSlot,
    NonCanonicalAddress,
    NotMapped,
    CorruptEntry,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Translation {
    pub virtual_address: u64,
    pub physical_address: u64,
    pub page_size: u64,
    pub writable: bool,
    pub user_accessible: bool,
    pub executable: bool,
    pub global: bool,
}

/// A view of the current address space through its recursive page-table slot.
///
/// # Safety
/// The active CR3 must contain a self-reference at recursive_slot, and the
/// current virtual address space must keep that recursive window mapped.
pub struct RecursivePageTable {
    levels: u8,
    recursive_slot: usize,
}

impl RecursivePageTable {
    /// Bind to the active recursive mapping.
    ///
    /// # Safety
    /// The caller must guarantee that the active root table maps itself at the
    /// supplied top-level slot with supervisor read/write permissions.
    pub unsafe fn new(levels: u8, recursive_slot: usize) -> Result<Self, PagingError> {
        if levels != 4 && levels != 5 {
            return Err(PagingError::UnsupportedPagingLevels);
        }
        if recursive_slot > INDEX_MASK as usize {
            return Err(PagingError::InvalidRecursiveSlot);
        }
        Ok(Self { levels, recursive_slot })
    }

    pub const fn levels(&self) -> u8 { self.levels }

    /// Translate a canonical virtual address using the active page tables.
    ///
    /// Supports 4 KiB, 2 MiB, and 1 GiB leaf mappings and computes effective
    /// permissions by intersecting all ancestor permissions.
    ///
    /// # Safety
    /// The recursive mapping must remain valid and page tables must not be
    /// concurrently modified without synchronization.
    pub unsafe fn translate(&self, virtual_address: u64) -> Result<Translation, PagingError> {
        if !is_canonical_for_levels(virtual_address, self.levels) {
            return Err(PagingError::NonCanonicalAddress);
        }

        let mut indices = [0usize; 5];
        for depth in 0..self.levels as usize {
            let shift = PAGE_SHIFT + 9 * (self.levels as u64 - depth as u64 - 1);
            indices[depth] = ((virtual_address >> shift) & INDEX_MASK) as usize;
        }

        let mut writable = true;
        let mut user_accessible = true;
        let mut executable = true;

        for depth in 0..self.levels as usize {
            let entry_address = recursive_entry_address(
                self.levels,
                self.recursive_slot,
                &indices,
                depth,
            ).ok_or(PagingError::CorruptEntry)?;

            // SAFETY: Constructor contract guarantees the recursive mapping;
            // caller contract excludes concurrent table destruction.
            let entry = unsafe { (entry_address as *const u64).read_volatile() };
            if entry & PRESENT == 0 { return Err(PagingError::NotMapped); }

            writable &= entry & WRITABLE != 0;
            user_accessible &= entry & USER != 0;
            executable &= entry & NO_EXECUTE == 0;

            let remaining_levels = self.levels as usize - depth;
            // At the final PTE level bit 7 is PAT, not the huge-page flag.
            if remaining_levels == 1 {
                let physical_base = entry & ADDRESS_MASK;
                return Ok(Translation {
                    virtual_address,
                    physical_address: physical_base | (virtual_address & (PAGE_SIZE - 1)),
                    page_size: PAGE_SIZE,
                    writable,
                    user_accessible,
                    executable,
                    global: entry & GLOBAL != 0,
                });
            }

            if entry & HUGE_PAGE != 0 {
                if remaining_levels != 3 && remaining_levels != 2 {
                    return Err(PagingError::CorruptEntry);
                }
                let page_size = 1u64 << (PAGE_SHIFT + 9 * (remaining_levels as u64 - 1));
                let physical_base = entry & ADDRESS_MASK & !(page_size - 1);
                return Ok(Translation {
                    virtual_address,
                    physical_address: physical_base | (virtual_address & (page_size - 1)),
                    page_size,
                    writable,
                    user_accessible,
                    executable,
                    global: entry & GLOBAL != 0,
                });
            }

            if entry & ADDRESS_MASK == 0 { return Err(PagingError::CorruptEntry); }
        }

        Err(PagingError::CorruptEntry)
    }
}

fn recursive_entry_address(
    levels: u8,
    recursive_slot: usize,
    indices: &[usize; 5],
    depth: usize,
) -> Option<u64> {
    if depth >= levels as usize || (levels != 4 && levels != 5) || recursive_slot > 511 {
        return None;
    }

    // The recursive alias uses [R repeated (levels-depth), i0, ...]
    // so the walk repeatedly follows the root self-reference, then descends
    // through the target path to the table page containing the desired entry.
    let recursive_prefix = levels as usize - depth;
    let mut raw = 0u64;
    for index in 0..levels as usize {
        let value = if index < recursive_prefix {
            recursive_slot as u64
        } else {
            indices[index - recursive_prefix] as u64
        };
        raw = (raw << 9) | value;
    }
    raw = (raw << 12) | ((indices[depth] as u64) * 8);

    let address_bits = PAGE_SHIFT + 9 * levels as u64;
    let sign_bit = address_bits - 1;
    if raw & (1u64 << sign_bit) != 0 {
        raw |= u64::MAX << address_bits;
    }
    Some(raw)
}

const fn is_canonical_for_levels(address: u64, levels: u8) -> bool {
    let sign_bit = if levels == 5 { 56 } else { 47 };
    let upper_shift = sign_bit + 1;
    let upper = address >> upper_shift;
    let sign = (address >> sign_bit) & 1;
    let upper_mask = u64::MAX >> upper_shift;
    (sign == 0 && upper == 0) || (sign == 1 && upper == upper_mask)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recursive_entry_addresses_are_canonical_for_both_modes() {
        let indices = [0x1ff, 0x123, 0x045, 0x067, 0x089];
        for levels in [4, 5] {
            for depth in 0..levels as usize {
                let address = recursive_entry_address(levels, RECURSIVE_SLOT, &indices, depth).unwrap();
                assert!(is_canonical_for_levels(address, levels));

                let address_bits = PAGE_SHIFT + 9 * levels as u64;
                let raw = address & ((1u64 << address_bits) - 1);
                let recursive_prefix = levels as usize - depth;
                for position in 0..levels as usize {
                    let shift = PAGE_SHIFT + 9 * (levels as u64 - position as u64 - 1);
                    let actual = ((raw >> shift) & INDEX_MASK) as usize;
                    let expected = if position < recursive_prefix {
                        RECURSIVE_SLOT
                    } else {
                        indices[position - recursive_prefix]
                    };
                    assert_eq!(actual, expected);
                }
                assert_eq!(raw & 0xfff, indices[depth] as u64 * 8);
            }
        }
    }

    #[test]
    fn recursive_entry_address_rejects_invalid_depth() {
        let indices = [0; 5];
        assert_eq!(recursive_entry_address(4, RECURSIVE_SLOT, &indices, 4), None);
        assert_eq!(recursive_entry_address(3, RECURSIVE_SLOT, &indices, 0), None);
    }
}
