//! Texture pictures as the path tracer reads them (see
//! `docs/reference/material-channels.md`): the full-size level of a decoded
//! texture, shared with whoever decoded it, sampled bilinearly and repeating.
//! Many samples per pixel already smooth what a mip chain would.

use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
};

use glam::{Vec2, Vec4};
use grid4d_scene::TextureId;
use half::f16;

/// A picture's texels: RGBA, rows top to bottom.
#[derive(Clone, Debug, PartialEq)]
pub enum Texels {
    /// 8 bits a channel, colour sRGB-encoded (alpha linear).
    Srgb8(Arc<[u8]>),
    /// 8 bits a channel, data read as stored.
    Linear8(Arc<[u8]>),
    /// Half floats, linear.
    Half(Arc<[f16]>),
}

/// One texture's picture.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    width: u32,
    height: u32,
    texels: Texels,
    /// Every texel grey: on a colour channel it multiplies the value.
    grey: bool,
}

/// 8-bit sRGB code → linear 0 – 1.
static SRGB: LazyLock<[f32; 256]> = LazyLock::new(|| {
    std::array::from_fn(|i| {
        let v = f32::from(u8::try_from(i).unwrap_or(u8::MAX)) / 255.0;
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    })
});

impl Picture {
    /// The picture `width` × `height` holding `texels`; `None` if they are
    /// not that many RGBA texels, or a side is 0 or over 65 535.
    #[must_use]
    pub fn new(width: u32, height: u32, texels: Texels) -> Option<Self> {
        if width == 0 || height == 0 || width > u32::from(u16::MAX) || height > u32::from(u16::MAX) {
            return None;
        }
        let count = usize::try_from(u64::from(width) * u64::from(height) * 4).ok()?;
        let grey = match &texels {
            Texels::Srgb8(bytes) | Texels::Linear8(bytes) => {
                if bytes.len() != count {
                    return None;
                }
                bytes.as_chunks::<4>().0.iter().all(|[r, g, b, _]| r == g && g == b)
            }
            Texels::Half(values) => {
                if values.len() != count {
                    return None;
                }
                values.as_chunks::<4>().0.iter().all(|[r, g, b, _]| r == g && g == b)
            }
        };
        Some(Self {
            width,
            height,
            texels,
            grey,
        })
    }

    /// Width and height in texels.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Whether every texel is grey.
    #[must_use]
    pub fn is_grey(&self) -> bool {
        self.grey
    }

    /// The texel at column `x`, row `y` (clamped into the picture), linear
    /// RGBA.
    #[must_use]
    pub fn texel_at(&self, x: u32, y: u32) -> Vec4 {
        let column = usize::try_from(x.min(self.width - 1)).unwrap_or(0);
        let row = usize::try_from(y.min(self.height - 1)).unwrap_or(0);
        self.texel(column, row)
    }

    /// The texel at column `x`, row `y` (both in range), linear RGBA.
    fn texel(&self, x: usize, y: usize) -> Vec4 {
        let width = usize::from(u16::try_from(self.width).unwrap_or(u16::MAX));
        let i = (y * width + x) * 4;
        match &self.texels {
            Texels::Srgb8(bytes) => {
                let lut = &*SRGB;
                let [r, g, b, a] = [bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]];
                Vec4::new(
                    lut[usize::from(r)],
                    lut[usize::from(g)],
                    lut[usize::from(b)],
                    f32::from(a) / 255.0,
                )
            }
            Texels::Linear8(bytes) => {
                Vec4::new(
                    f32::from(bytes[i]),
                    f32::from(bytes[i + 1]),
                    f32::from(bytes[i + 2]),
                    f32::from(bytes[i + 3]),
                ) / 255.0
            }
            Texels::Half(values) => Vec4::new(
                f32::from(values[i]),
                f32::from(values[i + 1]),
                f32::from(values[i + 2]),
                f32::from(values[i + 3]),
            ),
        }
    }

    /// The picture at `uv` (u right, v down; 0 – 1 across, repeating),
    /// between the four nearest texels.
    #[must_use]
    pub fn sample(&self, uv: Vec2) -> Vec4 {
        let (w, h) = (side(self.width), side(self.height));
        let x = uv.x * w - 0.5;
        let y = uv.y * h - 0.5;
        if !(x.is_finite() && y.is_finite()) {
            return self.texel(0, 0);
        }
        let (fx, fy) = (x - x.floor(), y - y.floor());
        let wrap = |i: f32, n: f32| {
            // `rem_euclid` keeps it in 0 … n − 1; whole numbers below 65 536.
            let r = i.rem_euclid(n);
            (r as usize).min(n as usize - 1)
        };
        let (x0, y0) = (wrap(x.floor(), w), wrap(y.floor(), h));
        let (x1, y1) = (wrap(x.floor() + 1.0, w), wrap(y.floor() + 1.0, h));
        let top = self.texel(x0, y0).lerp(self.texel(x1, y0), fx);
        let bottom = self.texel(x0, y1).lerp(self.texel(x1, y1), fx);
        top.lerp(bottom, fy)
    }
}

/// A side as a float (at most 65 535, exact).
fn side(n: u32) -> f32 {
    f32::from(u16::try_from(n).unwrap_or(u16::MAX))
}

/// The pictures a render reads, by the document's texture. A texture without
/// one (missing, not loaded yet) renders as its channels' values.
#[derive(Clone, Debug, Default)]
pub struct Pictures {
    map: HashMap<TextureId, Arc<Picture>>,
}

impl Pictures {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, id: TextureId, picture: Arc<Picture>) {
        self.map.insert(id, picture);
    }

    #[must_use]
    pub fn get(&self, id: TextureId) -> Option<&Arc<Picture>> {
        self.map.get(&id)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_check_their_size_and_know_when_they_are_grey() {
        let grey: Arc<[u8]> = Arc::from([10u8, 10, 10, 255, 200, 200, 200, 0]);
        let picture = Picture::new(2, 1, Texels::Linear8(Arc::clone(&grey))).unwrap();
        assert!(picture.is_grey());
        assert!(Picture::new(3, 1, Texels::Linear8(grey)).is_none(), "too few texels");
        let colour = Picture::new(1, 1, Texels::Srgb8(Arc::from([255u8, 0, 0, 255]))).unwrap();
        assert!(!colour.is_grey());
        assert!(Picture::new(0, 1, Texels::Srgb8(Arc::from([]))).is_none());
    }

    #[test]
    fn sampling_decodes_srgb_repeats_and_blends_neighbours() {
        // Black and white side by side.
        let bytes: Arc<[u8]> = Arc::from([0u8, 0, 0, 255, 255, 255, 255, 255]);
        let srgb = Picture::new(2, 1, Texels::Srgb8(Arc::clone(&bytes))).unwrap();
        assert!(
            srgb.sample(Vec2::new(0.25, 0.5))
                .abs_diff_eq(Vec4::new(0.0, 0.0, 0.0, 1.0), 1e-6)
        );
        assert!(srgb.sample(Vec2::new(0.75, 0.5)).abs_diff_eq(Vec4::ONE, 1e-6));
        // Half way between the texels' centres, and across the repeat seam.
        assert!((srgb.sample(Vec2::new(0.5, 0.5)).x - 0.5).abs() < 1e-6);
        assert!((srgb.sample(Vec2::new(1.0, 0.5)).x - 0.5).abs() < 1e-6);
        assert!(
            srgb.sample(Vec2::new(-0.25, 3.5)).abs_diff_eq(Vec4::ONE, 1e-6),
            "repeats"
        );
        // sRGB 128 is about 21 % linear light; as data it is 50 %.
        let mid: Arc<[u8]> = Arc::from([128u8, 128, 128, 255]);
        let srgb = Picture::new(1, 1, Texels::Srgb8(Arc::clone(&mid))).unwrap();
        assert!((srgb.sample(Vec2::ZERO).x - 0.2158).abs() < 1e-3);
        let data = Picture::new(1, 1, Texels::Linear8(mid)).unwrap();
        assert!((data.sample(Vec2::ZERO).x - 0.502).abs() < 1e-3);
        let float = Picture::new(1, 1, Texels::Half(Arc::from([f16::from_f32(4.0); 4]))).unwrap();
        assert_eq!(float.sample(Vec2::new(f32::NAN, 0.0)), Vec4::splat(4.0));
    }
}
