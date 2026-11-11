//! Fixed-capacity software layer compositor.
//!
//! Layers are borrowed image views ordered by an independent back-to-front
//! table. Stable generation-checked handles identify slots, not draw positions.
//! Reusing a vacant slot appends the new layer to the front of the draw order.
//! Rendering is damage-driven: each dirty region is restored to the compositor
//! background and every intersecting visible layer is composited over it.
//! No heap allocation or implicit framebuffer ownership is required.

use super::color::Color;
use super::dirty::{DirtyRegionSet, Rect};
use super::image::ImageView;
use super::info::FramebufferError;
use super::surface::FramebufferSurface;

/// Stable handle for a layer slot during its lifetime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LayerId { slot: usize, generation: u64 }

#[derive(Clone, Copy)]
struct Layer<'a> {
    image: ImageView<'a>,
    x: i64,
    y: i64,
    opacity: u8,
    visible: bool,
}

/// Errors returned by compositor management and rendering operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompositorError {
    NoLayerCapacity,
    InvalidLayer,
    TargetDimensionsMismatch,
    Framebuffer(FramebufferError),
}

impl From<FramebufferError> for CompositorError {
    fn from(error: FramebufferError) -> Self {
        Self::Framebuffer(error)
    }
}

/// Allocation-free compositor with compile-time layer and damage capacities.
pub struct Compositor<'a, const LAYERS: usize, const DIRTY: usize> {
    width: usize,
    height: usize,
    background: Color,
    layers: [Option<Layer<'a>>; LAYERS],
    /// Slot indices ordered from back to front; LayerId remains a stable slot.
    order: [usize; LAYERS],
    order_len: usize,
    generations: [u64; LAYERS],
    retired: [bool; LAYERS],
    dirty: DirtyRegionSet<DIRTY>,
}

impl<'a, const LAYERS: usize, const DIRTY: usize> Compositor<'a, LAYERS, DIRTY> {
    /// Construct a compositor for a fixed output resolution.
    pub fn new(width: usize, height: usize, background: Color) -> Option<Self> {
        let dirty = DirtyRegionSet::new(width, height)?;
        Some(Self {
            width,
            height,
            background,
            layers: [None; LAYERS],
            order: [0; LAYERS],
            order_len: 0,
            generations: [0; LAYERS],
            retired: [false; LAYERS],
            dirty,
        })
    }

    pub const fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    pub const fn layer_capacity(&self) -> usize {
        LAYERS
    }

    pub const fn layer_count(&self) -> usize {
        self.order_len
    }

    pub fn has_pending_damage(&self) -> bool {
        !self.dirty.is_empty()
    }

    /// Add a visible image layer in the first available slot.
    pub fn add_layer(
        &mut self,
        image: ImageView<'a>,
        x: i64,
        y: i64,
        opacity: u8,
    ) -> Result<LayerId, CompositorError> {
        let index = (0..LAYERS)
            .find(|&slot| self.layers[slot].is_none() && !self.retired[slot])
            .ok_or(CompositorError::NoLayerCapacity)?;
        self.layers[index] = Some(Layer { image, x, y, opacity, visible: true });
        self.order[self.order_len] = index;
        self.order_len += 1;
        self.mark_layer_bounds(image, x, y);
        Ok(LayerId { slot: index, generation: self.generations[index] })
    }

    /// Remove a layer and invalidate the area it previously occupied.
    pub fn remove_layer(&mut self, id: LayerId) -> Result<(), CompositorError> {
        let layer = self.layer(id)?;
        let old = *layer;
        self.layers[id.slot] = None;
        if let Some(position) = self.order[..self.order_len].iter().position(|&slot| slot == id.slot) {
            for index in position..self.order_len - 1 {
                self.order[index] = self.order[index + 1];
            }
            self.order_len -= 1;
        }
        if let Some(next_generation) = self.generations[id.slot].checked_add(1) {
            self.generations[id.slot] = next_generation;
        } else {
            self.retired[id.slot] = true;
        }
        self.mark_layer_bounds(old.image, old.x, old.y);
        Ok(())
    }

    /// Move a layer, invalidating both its previous and new positions.
    pub fn move_layer(&mut self, id: LayerId, x: i64, y: i64) -> Result<(), CompositorError> {
        let old = *self.layer(id)?;
        self.mark_layer_bounds(old.image, old.x, old.y);
        let layer = self.layers[id.slot].as_mut().ok_or(CompositorError::InvalidLayer)?;
        layer.x = x;
        layer.y = y;
        self.mark_layer_bounds(old.image, x, y);
        Ok(())
    }

    /// Swap draw order without changing either stable layer handle.
    pub fn swap_layer_order(&mut self, first: LayerId, second: LayerId) -> Result<(), CompositorError> {
        if first == second {
            self.layer(first)?;
            return Ok(());
        }
        let a = *self.layer(first)?;
        let b = *self.layer(second)?;
        self.mark_layer_bounds(a.image, a.x, a.y);
        self.mark_layer_bounds(b.image, b.x, b.y);
        let first_position = self.order[..self.order_len].iter()
            .position(|&slot| slot == first.slot).ok_or(CompositorError::InvalidLayer)?;
        let second_position = self.order[..self.order_len].iter()
            .position(|&slot| slot == second.slot).ok_or(CompositorError::InvalidLayer)?;
        self.order.swap(first_position, second_position);
        Ok(())
    }

    /// Move a layer to the highest occupied slot, preserving its handle.
    pub fn raise_layer(&mut self, id: LayerId) -> Result<(), CompositorError> {
        let layer = *self.layer(id)?;
        let position = self.order[..self.order_len].iter()
            .position(|&slot| slot == id.slot).ok_or(CompositorError::InvalidLayer)?;
        let highest = self.order_len - 1;
        if position != highest {
            self.mark_layer_bounds(layer.image, layer.x, layer.y);
            let slot = self.order[position];
            for index in position..highest {
                self.order[index] = self.order[index + 1];
            }
            self.order[highest] = slot;
        }
        Ok(())
    }

    /// Move a layer to the lowest occupied slot, preserving its handle.
    pub fn lower_layer(&mut self, id: LayerId) -> Result<(), CompositorError> {
        let layer = *self.layer(id)?;
        let position = self.order[..self.order_len].iter()
            .position(|&slot| slot == id.slot).ok_or(CompositorError::InvalidLayer)?;
        if position != 0 {
            self.mark_layer_bounds(layer.image, layer.x, layer.y);
            let slot = self.order[position];
            for index in (1..=position).rev() {
                self.order[index] = self.order[index - 1];
            }
            self.order[0] = slot;
        }
        Ok(())
    }

    /// Replace a layer's borrowed image and invalidate both old and new bounds.
    pub fn set_layer_image(
        &mut self,
        id: LayerId,
        image: ImageView<'a>,
    ) -> Result<(), CompositorError> {
        let old = *self.layer(id)?;
        self.mark_layer_bounds(old.image, old.x, old.y);
        self.layers[id.slot].as_mut()
            .ok_or(CompositorError::InvalidLayer)?
            .image = image;
        self.mark_layer_bounds(image, old.x, old.y);
        Ok(())
    }

    /// Change layer opacity (0 transparent, 255 fully opaque).
    pub fn set_opacity(&mut self, id: LayerId, opacity: u8) -> Result<(), CompositorError> {
        let old = *self.layer(id)?;
        if old.opacity == opacity {
            return Ok(());
        }
        let layer = self.layers[id.slot].as_mut().ok_or(CompositorError::InvalidLayer)?;
        layer.opacity = opacity;
        self.mark_layer_bounds(old.image, old.x, old.y);
        Ok(())
    }

    /// Toggle layer visibility and invalidate its bounds.
    pub fn set_visible(&mut self, id: LayerId, visible: bool) -> Result<(), CompositorError> {
        let old = *self.layer(id)?;
        if old.visible == visible {
            return Ok(());
        }
        self.layers[id.slot].as_mut().ok_or(CompositorError::InvalidLayer)?.visible = visible;
        self.mark_layer_bounds(old.image, old.x, old.y);
        Ok(())
    }

    /// Change the compositor background and request a complete redraw.
    pub fn set_background(&mut self, background: Color) {
        if self.background != background {
            self.background = background;
            self.dirty.mark_full();
        }
    }

    /// Force a complete redraw on the next render call.
    pub fn invalidate_all(&mut self) {
        self.dirty.mark_full();
    }

    /// Recompose pending damage into the supplied framebuffer.
    ///
    /// Damage is cleared only after all affected regions render successfully.
    pub fn render(
        &mut self,
        target: &mut FramebufferSurface<'_>,
    ) -> Result<(), CompositorError> {
        let info = *target.info();
        if info.width != self.width || info.height != self.height {
            return Err(CompositorError::TargetDimensionsMismatch);
        }
        if self.dirty.is_empty() {
            return Ok(());
        }

        if self.dirty.is_full() {
            self.render_region(target, Rect { x: 0, y: 0, width: self.width, height: self.height })?;
        } else {
            let count = self.dirty.len();
            for index in 0..count {
                let region = self.dirty.regions()[index];
                self.render_region(target, region)?;
            }
        }
        self.dirty.reset();
        Ok(())
    }

    fn render_region(
        &self,
        target: &mut FramebufferSurface<'_>,
        region: Rect,
    ) -> Result<(), CompositorError> {
        target.fill_rect(region.x, region.y, region.width, region.height, self.background);
        for order_index in 0..self.order_len {
            let layer = self.layers[self.order[order_index]]
                .as_ref()
                .ok_or(CompositorError::InvalidLayer)?;
            if !layer.visible || layer.opacity == 0 {
                continue;
            }
            layer.image.draw_alpha_blended_clipped(
                target,
                layer.x,
                layer.y,
                region.x,
                region.y,
                region.width,
                region.height,
                layer.opacity,
            )?;
        }
        Ok(())
    }

    fn layer(&self, id: LayerId) -> Result<&Layer<'a>, CompositorError> {
        if self.generations.get(id.slot).copied() != Some(id.generation) {
            return Err(CompositorError::InvalidLayer);
        }
        self.layers.get(id.slot)
            .and_then(Option::as_ref)
            .ok_or(CompositorError::InvalidLayer)
    }

    fn mark_layer_bounds(&mut self, image: ImageView<'a>, x: i64, y: i64) {
        let (width, height) = image.dimensions();
        let left = (x as i128).max(0);
        let top = (y as i128).max(0);
        let right = (x as i128 + width as i128).min(self.width as i128);
        let bottom = (y as i128 + height as i128).min(self.height as i128);
        if left >= right || top >= bottom {
            return;
        }
        self.dirty.mark(Rect {
            x: left as usize,
            y: top as usize,
            width: (right - left) as usize,
            height: (bottom - top) as usize,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{Compositor, CompositorError};
    use crate::drivers::framebuffer::color::Color;
    use crate::drivers::framebuffer::image::ImageView;

    #[test]
    fn z_order_changes_do_not_invalidate_layer_handles() {
        let pixels_a = [Color::opaque(255, 0, 0)];
        let pixels_b = [Color::opaque(0, 0, 255)];
        let image_a = ImageView::new(&pixels_a, 1, 1, 1).unwrap();
        let image_b = ImageView::new(&pixels_b, 1, 1, 1).unwrap();
        let mut compositor = Compositor::<4, 8>::new(32, 32, Color::BLACK).unwrap();

        let a = compositor.add_layer(image_a, 0, 0, 255).unwrap();
        let b = compositor.add_layer(image_b, 1, 1, 255).unwrap();
        compositor.raise_layer(a).unwrap();
        compositor.lower_layer(b).unwrap();
        compositor.swap_layer_order(a, b).unwrap();

        assert_eq!(compositor.layer_count(), 2);
        compositor.remove_layer(a).unwrap();
        compositor.remove_layer(b).unwrap();
        assert_eq!(compositor.layer_count(), 0);
    }

    #[test]
    fn rejects_stale_layer_handle_after_slot_reuse() {
        let pixels = [Color::opaque(1, 2, 3)];
        let image = ImageView::new(&pixels, 1, 1, 1).unwrap();
        let mut compositor = Compositor::<1, 2>::new(8, 8, Color::BLACK).unwrap();
        let stale = compositor.add_layer(image, 0, 0, 255).unwrap();
        compositor.remove_layer(stale).unwrap();
        let current = compositor.add_layer(image, 1, 1, 255).unwrap();
        assert_ne!(stale, current);
        assert_eq!(compositor.set_visible(stale, false), Err(CompositorError::InvalidLayer));
        assert!(compositor.set_visible(current, false).is_ok());
    }
}
