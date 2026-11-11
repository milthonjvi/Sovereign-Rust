//! Pixel color representation and channel ordering.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PixelFormat { Rgb, Bgr }

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Color { pub red: u8, pub green: u8, pub blue: u8, pub alpha: u8 }

impl Color {
    pub const BLACK: Self = Self::new(0, 0, 0);
    pub const WHITE: Self = Self::new(255, 255, 255);
    pub const RED: Self = Self::new(255, 0, 0);
    pub const GREEN: Self = Self::new(0, 255, 0);
    pub const BLUE: Self = Self::new(0, 0, 255);
    /// Construct a color with the framebuffer's reserved byte cleared.
    /// For image data intended for alpha compositing, use `opaque` or
    /// `with_alpha` so the source opacity is explicit.
    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue, alpha: 0 }
    }
    /// Construct a fully opaque color suitable for image layers.
    pub const fn opaque(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue, alpha: 255 }
    }
    /// Construct an RGBA color where alpha 0 is transparent and 255 opaque.
    pub const fn with_alpha(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self { red, green, blue, alpha }
    }
    pub(crate) const fn memory_bytes(self, format: PixelFormat) -> [u8; 4] {
        match format {
            PixelFormat::Rgb => [self.red, self.green, self.blue, self.alpha],
            PixelFormat::Bgr => [self.blue, self.green, self.red, self.alpha],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Color;

    #[test]
    fn opaque_constructor_sets_full_alpha() {
        assert_eq!(Color::opaque(1, 2, 3).alpha, 255);
    }

    #[test]
    fn rgb_constructor_keeps_reserved_byte_clear() {
        assert_eq!(Color::new(1, 2, 3).alpha, 0);
    }
}
