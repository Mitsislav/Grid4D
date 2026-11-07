//! Decoding texture pictures (see `docs/reference/material-textures.md`):
//! PNG, JPEG, TIFF, WebP, Radiance HDR and OpenEXR through the `image` crate
//! (only those decoders, pure Rust), into a chain of mip-maps made once.
//!
//! Sizes a file claims are never trusted: the decoder works under limits
//! (16 384 pixels a side, 1 GiB of memory), so a hostile file is an error,
//! never an abort.

use std::io::Cursor;

use grid4d_scene::ColorSpace;
use half::f16;
use image::{DynamicImage, ImageReader, Limits};

use crate::IoError;

/// The extensions of the pictures Grid4D reads as textures.
pub const TEXTURE_EXTENSIONS: [&str; 8] = ["png", "jpg", "jpeg", "tif", "tiff", "webp", "hdr", "exr"];

/// The largest side of a texture, in pixels.
pub const MAX_TEXTURE_SIDE: u32 = 16_384;
/// The most memory one decode may take.
const MAX_DECODE_BYTES: u64 = 1 << 30;

/// `true` if `extension` (without the dot) names a picture Grid4D reads.
#[must_use]
pub fn is_texture_extension(extension: &str) -> bool {
    TEXTURE_EXTENSIONS.iter().any(|e| extension.eq_ignore_ascii_case(e))
}

/// One level's pixels, RGBA, rows top to bottom.
#[derive(Clone, Debug, PartialEq)]
pub enum Pixels {
    /// 8 bits a channel as the file stores them (sRGB-encoded or data,
    /// as the texture's colour space says).
    Rgba8(Vec<u8>),
    /// Half floats: pictures with more than 8 bits a channel (16-bit, HDR,
    /// OpenEXR), as stored.
    RgbaF16(Vec<f16>),
}

/// One level of a mip chain.
#[derive(Clone, Debug, PartialEq)]
pub struct Level {
    pub width: u32,
    pub height: u32,
    pub pixels: Pixels,
}

impl Level {
    /// The texels as bytes, as the GPU takes them: RGBA8 as stored, half
    /// floats little-endian.
    #[must_use]
    pub fn bytes(&self) -> std::borrow::Cow<'_, [u8]> {
        match &self.pixels {
            Pixels::Rgba8(bytes) => std::borrow::Cow::Borrowed(bytes),
            Pixels::RgbaF16(values) => values.iter().flat_map(|v| v.to_le_bytes()).collect(),
        }
    }

    /// The level as 8-bit sRGB-encoded RGBA, for showing it on screen (values
    /// above 1 are clipped).
    #[must_use]
    pub fn to_srgb8(&self, color_space: ColorSpace) -> Vec<u8> {
        match (&self.pixels, color_space) {
            (Pixels::Rgba8(bytes), ColorSpace::Srgb) => bytes.clone(),
            // Data shown as it is (a normal map looks like a normal map).
            (Pixels::Rgba8(bytes), ColorSpace::Linear) => bytes.clone(),
            (Pixels::RgbaF16(values), _) => values
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| {
                    let [r, g, b, a] = p.map(f32::from);
                    let encode = |v: f32| match color_space {
                        ColorSpace::Linear => linear_to_srgb8(v),
                        ColorSpace::Srgb => unit_to_u8(v),
                    };
                    [encode(r), encode(g), encode(b), unit_to_u8(a)]
                })
                .collect(),
        }
    }
}

/// A decoded texture: its full-size picture first, then each level half the
/// size of the one before, down to 1 × 1.
#[derive(Clone, Debug, PartialEq)]
pub struct TextureImage {
    pub levels: Vec<Level>,
}

impl TextureImage {
    /// Width and height of the full-size picture.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        self.levels.first().map_or((0, 0), |l| (l.width, l.height))
    }

    /// The largest level no larger than `side` pixels either way (the
    /// smallest one if none is).
    #[must_use]
    pub fn level_within(&self, side: u32) -> Option<&Level> {
        self.levels
            .iter()
            .find(|l| l.width <= side && l.height <= side)
            .or_else(|| self.levels.last())
    }

    /// Bytes the pixels of every level take.
    #[must_use]
    pub fn memory(&self) -> usize {
        self.levels
            .iter()
            .map(|l| match &l.pixels {
                Pixels::Rgba8(p) => p.len(),
                Pixels::RgbaF16(p) => p.len() * 2,
            })
            .sum()
    }
}

/// Decodes the picture in `bytes` (its format told by its first bytes) and
/// makes its mip-maps; `color_space` says how 8-bit values are averaged.
pub fn decode_texture(bytes: &[u8], color_space: ColorSpace) -> Result<TextureImage, IoError> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(IoError::Io)?;
    if reader.format().is_none() {
        return Err(IoError::Invalid("not a picture Grid4D reads".to_owned()));
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_TEXTURE_SIDE);
    limits.max_image_height = Some(MAX_TEXTURE_SIDE);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|err| IoError::Invalid(format!("the picture cannot be read: {err}")))?;
    let (width, height) = (image.width(), image.height());
    if width == 0 || height == 0 {
        return Err(IoError::Invalid("the picture is empty".to_owned()));
    }
    let wide = matches!(
        image,
        DynamicImage::ImageLuma16(_)
            | DynamicImage::ImageLumaA16(_)
            | DynamicImage::ImageRgb16(_)
            | DynamicImage::ImageRgba16(_)
            | DynamicImage::ImageRgb32F(_)
            | DynamicImage::ImageRgba32F(_)
    );
    let first = Level {
        width,
        height,
        pixels: if wide {
            Pixels::RgbaF16(image.into_rgba32f().into_raw().into_iter().map(f16::from_f32).collect())
        } else {
            Pixels::Rgba8(image.into_rgba8().into_raw())
        },
    };
    let mut levels = vec![first];
    while let Some(last) = levels.last().filter(|l| l.width > 1 || l.height > 1) {
        let next = half_size(last, color_space);
        levels.push(next);
    }
    Ok(TextureImage { levels })
}

/// The level half the size of `level` (at least 1), each pixel the average of
/// the 2 × 2 (or fewer, at an odd edge) below it; sRGB colour averaged in
/// linear light, alpha and data as they are.
fn half_size(level: &Level, color_space: ColorSpace) -> Level {
    let (w, h) = (level.width, level.height);
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let index = |x: u32, y: u32| (us(y) * us(w) + us(x)) * 4;
    let sources = |x: u32, y: u32| {
        let (x0, y0) = ((2 * x).min(w - 1), (2 * y).min(h - 1));
        let (x1, y1) = ((2 * x + 1).min(w - 1), (2 * y + 1).min(h - 1));
        [index(x0, y0), index(x1, y0), index(x0, y1), index(x1, y1)]
    };
    let pixels = match &level.pixels {
        Pixels::Rgba8(src) => {
            let lut = srgb_table();
            let mut out = Vec::with_capacity(us(nw * nh) * 4);
            for y in 0..nh {
                for x in 0..nw {
                    let at = sources(x, y);
                    for c in 0..4 {
                        let colour = c < 3 && color_space == ColorSpace::Srgb;
                        let sum: f32 = at
                            .iter()
                            .map(|&i| {
                                let v = src[i + c];
                                if colour {
                                    lut[usize::from(v)]
                                } else {
                                    f32::from(v) / 255.0
                                }
                            })
                            .sum();
                        let mean = sum * 0.25;
                        out.push(if colour {
                            linear_to_srgb8(mean)
                        } else {
                            unit_to_u8(mean)
                        });
                    }
                }
            }
            Pixels::Rgba8(out)
        }
        Pixels::RgbaF16(src) => {
            let mut out = Vec::with_capacity(us(nw * nh) * 4);
            for y in 0..nh {
                for x in 0..nw {
                    let at = sources(x, y);
                    for c in 0..4 {
                        let sum: f32 = at.iter().map(|&i| f32::from(src[i + c])).sum();
                        out.push(f16::from_f32(sum * 0.25));
                    }
                }
            }
            Pixels::RgbaF16(out)
        }
    };
    Level {
        width: nw,
        height: nh,
        pixels,
    }
}

/// A pixel count or index as `usize` (lossless on every supported platform).
fn us(n: u32) -> usize {
    usize::try_from(n).unwrap_or(usize::MAX)
}

/// 8-bit sRGB code → linear 0 – 1.
fn srgb_table() -> [f32; 256] {
    std::array::from_fn(|i| {
        let v = f32::from(u8::try_from(i).unwrap_or(u8::MAX)) / 255.0;
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    })
}

/// Linear 0 – 1 → 8-bit sRGB code (clipped).
fn linear_to_srgb8(v: f32) -> u8 {
    let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    let encoded = if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    unit_to_u8(encoded)
}

/// 0 – 1 → 0 – 255, rounded (clipped).
fn unit_to_u8(v: f32) -> u8 {
    let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    // Exact: a whole number in 0 – 255 after the clamp and rounding.
    (v * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test code")]

    use image::{ImageFormat, Rgb32FImage, RgbaImage};

    use super::*;

    /// A 5 × 3 picture with a gradient, encoded as `format`.
    fn encoded(format: ImageFormat) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        if matches!(format, ImageFormat::Hdr | ImageFormat::OpenExr) {
            let picture = Rgb32FImage::from_fn(5, 3, |x, y| {
                image::Rgb([
                    f32::from(u8::try_from(x).unwrap()) * 0.5,
                    f32::from(u8::try_from(y).unwrap()),
                    4.0,
                ])
            });
            DynamicImage::ImageRgb32F(picture).write_to(&mut out, format).unwrap();
        } else {
            let picture = RgbaImage::from_fn(5, 3, |x, y| {
                image::Rgba([u8::try_from(x * 50).unwrap(), u8::try_from(y * 100).unwrap(), 128, 255])
            });
            let picture = DynamicImage::ImageRgba8(picture);
            let picture = if format == ImageFormat::Jpeg {
                DynamicImage::ImageRgb8(picture.into_rgb8())
            } else {
                picture
            };
            picture.write_to(&mut out, format).unwrap();
        }
        out.into_inner()
    }

    #[test]
    fn every_format_decodes_into_a_mip_chain() {
        for format in [
            ImageFormat::Png,
            ImageFormat::Jpeg,
            ImageFormat::Tiff,
            ImageFormat::WebP,
            ImageFormat::Hdr,
            ImageFormat::OpenExr,
        ] {
            let image = decode_texture(&encoded(format), ColorSpace::Srgb).unwrap();
            assert_eq!(image.size(), (5, 3), "{format:?}");
            let sizes: Vec<(u32, u32)> = image.levels.iter().map(|l| (l.width, l.height)).collect();
            assert_eq!(sizes, [(5, 3), (2, 1), (1, 1)], "{format:?}");
            let float = matches!(image.levels[0].pixels, Pixels::RgbaF16(_));
            assert_eq!(
                float,
                matches!(format, ImageFormat::Hdr | ImageFormat::OpenExr),
                "{format:?}"
            );
            assert_eq!(image.level_within(2).unwrap().width, 2);
            assert_eq!(image.level_within(0).unwrap().width, 1, "the smallest when none fits");
            assert_eq!(image.levels[0].to_srgb8(ColorSpace::Srgb).len(), 5 * 3 * 4);
        }
    }

    #[test]
    fn colour_is_averaged_in_linear_light_and_data_as_it_is() {
        // Black and white side by side: the 1 × 1 level of sRGB colour is
        // middle grey in light (188), of data the plain mean (128).
        let picture = RgbaImage::from_fn(2, 1, |x, _| image::Rgba([if x == 0 { 0 } else { 255 }; 4]));
        let mut png = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(picture)
            .write_to(&mut png, ImageFormat::Png)
            .unwrap();
        let level = |space| {
            let image = decode_texture(png.get_ref(), space).unwrap();
            let Pixels::Rgba8(p) = &image.levels[1].pixels else {
                panic!("8 bits")
            };
            p.clone()
        };
        assert_eq!(level(ColorSpace::Srgb), [188, 188, 188, 128], "alpha is never encoded");
        assert_eq!(level(ColorSpace::Linear), [128, 128, 128, 128]);
    }

    #[test]
    fn damaged_and_oversized_pictures_are_errors() {
        assert!(decode_texture(b"not a picture at all", ColorSpace::Srgb).is_err());
        let mut png = encoded(ImageFormat::Png);
        png.truncate(png.len() / 2);
        assert!(decode_texture(&png, ColorSpace::Srgb).is_err());
        // A PNG header claiming 100 000 × 100 000 pixels: refused, nothing allocated.
        let mut huge = encoded(ImageFormat::Png);
        huge[16..20].copy_from_slice(&100_000u32.to_be_bytes());
        huge[20..24].copy_from_slice(&100_000u32.to_be_bytes());
        assert!(decode_texture(&huge, ColorSpace::Srgb).is_err());
        assert!(is_texture_extension("JPG") && !is_texture_extension("gif"));
    }
}
