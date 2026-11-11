//! Fixed-capacity dirty-region tracking for framebuffer composition.
//!
//! The tracker performs no allocation. Rectangles are clipped to the target
//! surface and overlapping or edge-adjacent regions are coalesced. If the
//! fixed storage fills, the tracker switches to a full-surface invalidation;
//! callers must then redraw the complete target.

/// A clipped rectangle expressed in half-open pixel coordinates.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl Rect {
    pub const EMPTY: Self = Self {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
    };

    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub const fn right(self) -> usize {
        self.x.saturating_add(self.width)
    }

    pub const fn bottom(self) -> usize {
        self.y.saturating_add(self.height)
    }
    /// Return the half-open intersection with another rectangle.
    /// Empty intersections are represented by Rect::EMPTY.
    pub fn intersection(self, other: Self) -> Self {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        if left >= right || top >= bottom { return Self::EMPTY; }
        Self { x: left, y: top, width: right - left, height: bottom - top }
    }

    /// Whether two non-empty half-open rectangles share at least one pixel.
    pub fn intersects(self, other: Self) -> bool {
        !self.intersection(other).is_empty()
    }

    /// Return the smallest rectangle containing both inputs.
    /// Empty inputs are treated as identity elements.
    pub fn bounding_union(self, other: Self) -> Self {
        if self.is_empty() { return other; }
        if other.is_empty() { return self; }
        self.union(other)
    }

    /// Clip this rectangle to a target extent beginning at the origin.
    pub fn clip_to(self, width: usize, height: usize) -> Self {
        self.clipped(width, height)
    }

    /// Pixel area, saturating if the mathematical area exceeds usize.
    pub const fn area(self) -> usize {
        self.width.saturating_mul(self.height)
    }

    fn touches_or_overlaps(self, other: Self) -> bool {
        self.x <= other.right()
            && other.x <= self.right()
            && self.y <= other.bottom()
            && other.y <= self.bottom()
    }

    fn union(self, other: Self) -> Self {
        let left = self.x.min(other.x);
        let top = self.y.min(other.y);
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Self {
            x: left,
            y: top,
            width: right.saturating_sub(left),
            height: bottom.saturating_sub(top),
        }
    }

    fn clipped(self, width: usize, height: usize) -> Self {
        let left = self.x.min(width);
        let top = self.y.min(height);
        let right = self.right().min(width);
        let bottom = self.bottom().min(height);
        Self {
            x: left,
            y: top,
            width: right.saturating_sub(left),
            height: bottom.saturating_sub(top),
        }
    }
}

/// Allocation-free invalidation set for a fixed-size render target.
///
/// When `is_full()` is true, individual rectangles are intentionally hidden:
/// the consumer must redraw the complete surface. Call `reset()` only after
/// the corresponding redraw or presentation has completed.
pub struct DirtyRegionSet<const CAPACITY: usize> {
    width: usize,
    height: usize,
    regions: [Rect; CAPACITY],
    len: usize,
    full: bool,
}

impl<const CAPACITY: usize> DirtyRegionSet<CAPACITY> {
    /// Create a tracker for a non-empty target. Zero-sized targets are rejected.
    pub const fn new(width: usize, height: usize) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        Some(Self {
            width,
            height,
            regions: [Rect::EMPTY; CAPACITY],
            len: 0,
            full: false,
        })
    }

    pub const fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    pub const fn is_full(&self) -> bool {
        self.full
    }

    pub const fn is_empty(&self) -> bool {
        !self.full && self.len == 0
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    /// Borrow the pending rectangles. Empty when a full redraw is required.
    pub fn regions(&self) -> &[Rect] {
        &self.regions[..self.len]
    }

    /// Mark a rectangle as changed. Coordinates are clipped to the target.
    ///
    /// Adjacent and overlapping rectangles are merged transitively. If the
    /// merged set exceeds CAPACITY, a full redraw is requested instead.
    pub fn mark(&mut self, rect: Rect) {
        if self.full {
            return;
        }

        let mut merged = rect.clipped(self.width, self.height);
        if merged.is_empty() {
            return;
        }

        // Re-scan after every merge: a larger union may now touch an earlier
        // region that did not touch the original incoming rectangle.
        let mut index = 0;
        while index < self.len {
            if merged.touches_or_overlaps(self.regions[index]) {
                merged = merged.union(self.regions[index]);
                self.remove(index);
                index = 0;
            } else {
                index += 1;
            }
        }

        if self.len == CAPACITY {
            self.mark_full();
            return;
        }

        self.regions[self.len] = merged;
        self.len += 1;
    }

    /// Invalidate the complete target, discarding individual regions.
    pub fn mark_full(&mut self) {
        self.len = 0;
        self.full = true;
    }

    /// Forget all pending invalidations after a successful redraw/present.
    pub fn reset(&mut self) {
        self.len = 0;
        self.full = false;
    }

    fn remove(&mut self, index: usize) {
        debug_assert!(index < self.len);
        for current in index..self.len - 1 {
            self.regions[current] = self.regions[current + 1];
        }
        self.len -= 1;
        self.regions[self.len] = Rect::EMPTY;
    }
}

#[cfg(test)]
mod tests {
    use super::{DirtyRegionSet, Rect};

    #[test]
    fn rectangle_intersection_uses_half_open_pixel_bounds() {
        let a = Rect { x: 2, y: 3, width: 5, height: 4 };
        let b = Rect { x: 5, y: 1, width: 4, height: 5 };
        assert_eq!(a.intersection(b), Rect { x: 5, y: 3, width: 2, height: 3 });
        assert!(a.intersects(b));
        assert!(!a.intersects(Rect { x: 7, y: 3, width: 2, height: 2 }));
    }

    #[test]
    fn rectangle_union_and_clip_handle_empty_and_overflowing_extents() {
        let empty = Rect::EMPTY;
        let rect = Rect { x: 8, y: 7, width: usize::MAX, height: 4 };
        assert_eq!(empty.bounding_union(rect), rect);
        assert_eq!(rect.clip_to(10, 9), Rect { x: 8, y: 7, width: 2, height: 2 });
        assert_eq!(rect.area(), usize::MAX);
    }
    #[test]
    fn clips_regions_to_surface() {
        let mut dirty = DirtyRegionSet::<4>::new(100, 80).unwrap();
        dirty.mark(Rect { x: 90, y: 70, width: 30, height: 30 });
        assert_eq!(dirty.regions(), &[Rect { x: 90, y: 70, width: 10, height: 10 }]);
    }

    #[test]
    fn merges_overlapping_and_adjacent_regions() {
        let mut dirty = DirtyRegionSet::<4>::new(100, 100).unwrap();
        dirty.mark(Rect { x: 0, y: 0, width: 10, height: 10 });
        dirty.mark(Rect { x: 10, y: 0, width: 5, height: 10 });
        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty.regions()[0], Rect { x: 0, y: 0, width: 15, height: 10 });
    }

    #[test]
    fn merge_is_transitive() {
        let mut dirty = DirtyRegionSet::<4>::new(100, 100).unwrap();
        dirty.mark(Rect { x: 20, y: 0, width: 5, height: 5 });
        dirty.mark(Rect { x: 0, y: 0, width: 5, height: 5 });
        dirty.mark(Rect { x: 5, y: 0, width: 15, height: 5 });
        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty.regions()[0], Rect { x: 0, y: 0, width: 25, height: 5 });
    }

    #[test]
    fn capacity_overflow_requests_full_redraw() {
        let mut dirty = DirtyRegionSet::<1>::new(100, 100).unwrap();
        dirty.mark(Rect { x: 0, y: 0, width: 5, height: 5 });
        dirty.mark(Rect { x: 20, y: 20, width: 5, height: 5 });
        assert!(dirty.is_full());
        assert!(dirty.regions().is_empty());
    }

    #[test]
    fn ignores_empty_and_outside_regions() {
        let mut dirty = DirtyRegionSet::<2>::new(10, 10).unwrap();
        dirty.mark(Rect { x: 1, y: 1, width: 0, height: 4 });
        dirty.mark(Rect { x: 20, y: 20, width: 5, height: 5 });
        assert!(dirty.is_empty());
    }

    #[test]
    fn reset_clears_full_invalidation() {
        let mut dirty = DirtyRegionSet::<1>::new(10, 10).unwrap();
        dirty.mark_full();
        dirty.reset();
        assert!(dirty.is_empty());
        assert!(!dirty.is_full());
    }
}
