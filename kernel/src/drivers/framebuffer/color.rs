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
    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue, alpha: 0 }
    }
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
