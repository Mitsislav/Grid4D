//! A rendered picture.

/// A picture in linear RGB with premultiplied alpha (alpha: how much of the pixel
/// objects cover), rows from the top.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[f32; 4]>,
}

impl Image {
    /// A transparent picture.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        let count = usize::try_from(u64::from(width) * u64::from(height)).unwrap_or(0);
        Self {
            width,
            height,
            pixels: vec![[0.0; 4]; count],
        }
    }

    /// The pixel at (`x`, `y`), if inside.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<[f32; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let index = u64::from(y) * u64::from(self.width) + u64::from(x);
        self.pixels.get(usize::try_from(index).ok()?).copied()
    }

    /// 8-bit RGBA for files and screens: colours sRGB-encoded (clamped), alpha
    /// straight (not premultiplied), four bytes per pixel.
    #[must_use]
    pub fn to_rgba8(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.pixels.len() * 4);
        for &[r, g, b, a] in &self.pixels {
            let alpha = a.clamp(0.0, 1.0);
            let straight = |c: f32| if alpha > 0.0 { c / alpha } else { 0.0 };
            bytes.extend([
                to_byte(linear_to_srgb(straight(r))),
                to_byte(linear_to_srgb(straight(g))),
                to_byte(linear_to_srgb(straight(b))),
                to_byte(alpha),
            ]);
        }
        bytes
    }

    /// 8-bit RGB over a black background (no transparency): colours
    /// sRGB-encoded (clamped), three bytes per pixel.
    #[must_use]
    pub fn to_rgb8(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.pixels.len() * 3);
        for &[r, g, b, _] in &self.pixels {
            // Premultiplied colour is already the colour over black.
            bytes.extend([r, g, b].map(|c| to_byte(linear_to_srgb(c))));
        }
        bytes
    }
}

/// The sRGB transfer curve, input clamped to 0…1.
fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// 0…1 to 0…255, rounded.
fn to_byte(value: f32) -> u8 {
    // Exact: a whole number in 0…255 after the clamp and rounding.
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_are_srgb_with_straight_alpha() {
        let mut image = Image::new(2, 1);
        image.pixels[0] = [0.5, 0.5, 0.5, 0.5]; // half-covered mid grey
        image.pixels[1] = [0.216, 1.5, 0.0, 1.0];
        let bytes = image.to_rgba8();
        assert_eq!(&bytes[..4], &[255, 255, 255, 128], "0.5 / 0.5 = white, alpha 128");
        assert_eq!(bytes[4], 128, "linear 0.216 is sRGB mid grey");
        assert_eq!(bytes[5], 255, "clamped");
        assert_eq!(image.pixel(1, 0), Some([0.216, 1.5, 0.0, 1.0]));
        assert_eq!(image.pixel(2, 0), None);

        let opaque = image.to_rgb8();
        assert_eq!(&opaque[..3], &[188, 188, 188], "half grey over black: linear 0.5");
        assert_eq!(&opaque[3..], &[128, 255, 0]);
    }
}
