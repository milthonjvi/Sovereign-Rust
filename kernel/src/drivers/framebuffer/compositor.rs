//! Fixed-capacity software layer compositor.
//!
//! Layers are borrowed image views in slot order (low slot to high slot).
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
pub struct LayerId(usize);

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
            dirty,
        })
    }

    pub const fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    pub const fn layer_capacity(&self) -> usize {
        LAYERS
    }

    pub fn layer_count(&self) -> usize {
        self.layers.iter().filter(|layer| layer.is_some()).count()
    }

    pub fn has_pending_damage(&self) -> bool {
        !self.dirty.is_empty()
    }

    /// Add a visible image layer at the topmost occupied slot.
    pub fn add_layer(
        &mut self,
        image: ImageView<'a>,
        x: i64,
        y: i64,
        opacity: u8,
    ) -> Result<LayerId, CompositorError> {
        let index = self.layers.iter().position(Option::is_none)
            .ok_or(CompositorError::NoLayerCapacity)?;
        self.layers[index] = Some(Layer { image, x, y, opacity, visible: true });
        self.mark_layer_bounds(image, x, y);
        Ok(LayerId(index))
    }

    /// Remove a layer and invalidate the area it previously occupied.
    pub fn remove_layer(&mut self, id: LayerId) -> Result<(), CompositorError> {
        let layer = self.layer(id)?;
        let old = *layer;
        self.layers[id.0] = None;
        self.mark_layer_bounds(old.image, old.x, old.y);
        Ok(())
    }

    /// Move a layer, invalidating both its previous and new positions.
    pub fn move_layer(&mut self, id: LayerId, x: i64, y: i64) -> Result<(), CompositorError> {
        let old = *self.layer(id)?;
        self.mark_layer_bounds(old.image, old.x, old.y);
        let layer = self.layers[id.0].as_mut().ok_or(CompositorError::InvalidLayer)?;
        layer.x = x;
        layer.y = y;
        self.mark_layer_bounds(old.image, x, y);
        Ok(())
    }

    /// Change layer opacity (0 transparent, 255 fully opaque).
    pub fn set_opacity(&mut self, id: LayerId, opacity: u8) -> Result<(), CompositorError> {
        let old = *self.layer(id)?;
        if old.opacity == opacity {
            return Ok(());
        }
        let layer = self.layers[id.0].as_mut().ok_or(CompositorError::InvalidLayer)?;
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
        self.layers[id.0].as_mut().ok_or(CompositorError::InvalidLayer)?.visible = visible;
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
        for layer in self.layers.iter().flatten() {
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
        self.layers.get(id.0)
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
