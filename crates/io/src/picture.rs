//! Rendered pictures as files: PNG (sRGB, 8 bits per channel) and OpenEXR
//! (linear light, 16-bit floats).

use std::{io::Write as _, path::Path};

use crate::{IoError, write_atomic};

/// Which channels a picture has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channels {
    /// Red, green, blue (PNG: three bytes per pixel).
    Rgb,
    /// Red, green, blue and alpha (PNG: straight, four bytes per pixel;
    /// OpenEXR: premultiplied).
    Rgba,
}

impl Channels {
    const fn bytes_per_pixel(self) -> u64 {
        match self {
            Self::Rgb => 3,
            Self::Rgba => 4,
        }
    }
}

/// The PNG file of a `width` × `height` picture of sRGB `pixels`, rows from the top.
pub fn png_bytes(width: u32, height: u32, channels: Channels, pixels: &[u8]) -> Result<Vec<u8>, IoError> {
    let expected = u64::from(width) * u64::from(height) * channels.bytes_per_pixel();
    if u64::try_from(pixels.len()).ok() != Some(expected) || expected == 0 {
        return Err(IoError::Invalid(format!(
            "a {width} × {height} picture needs {expected} bytes, not {}",
            pixels.len()
        )));
    }
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width, height);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_color(match channels {
        Channels::Rgb => png::ColorType::Rgb,
        Channels::Rgba => png::ColorType::Rgba,
    });
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder.write_header().map_err(encoding)?;
    writer.write_image_data(pixels).map_err(encoding)?;
    writer.finish().map_err(encoding)?;
    Ok(bytes)
}

/// Saves a picture (see [`png_bytes`]) to `path`, atomically like documents.
pub fn save_png(path: &Path, width: u32, height: u32, channels: Channels, pixels: &[u8]) -> Result<(), IoError> {
    write_atomic(path, &png_bytes(width, height, channels, pixels)?)
}

/// Lines per compressed block (OpenEXR's ZIP compression).
const EXR_BLOCK_LINES: usize = 16;
/// The largest finite 16-bit float.
const HALF_MAX: f32 = 65_504.0;

/// The OpenEXR file of a `width` × `height` picture of linear, premultiplied
/// RGBA `pixels`, rows from the top: a single-part scanline image of 16-bit
/// floats (R, G, B and, with [`Channels::Rgba`], A), ZIP-compressed in blocks of
/// 16 lines. Values beyond the 16-bit range are clamped; NaN becomes 0.
pub fn exr_bytes(width: u32, height: u32, channels: Channels, pixels: &[[f32; 4]]) -> Result<Vec<u8>, IoError> {
    let too_large = || IoError::Invalid(format!("a {width} × {height} picture is too large for OpenEXR"));
    let count = u64::from(width) * u64::from(height);
    if u64::try_from(pixels.len()).ok() != Some(count) || count == 0 {
        return Err(IoError::Invalid(format!(
            "a {width} × {height} picture needs {count} pixels, not {}",
            pixels.len()
        )));
    }
    let row_length = usize::try_from(width).map_err(|_| too_large())?;
    let max_x = i32::try_from(width - 1).map_err(|_| too_large())?;
    let max_y = i32::try_from(height - 1).map_err(|_| too_large())?;
    // Channels in alphabetical order, as the format stores them, with their
    // index in a pixel.
    let order: &[(&str, usize)] = match channels {
        Channels::Rgb => &[("B", 2), ("G", 1), ("R", 0)],
        Channels::Rgba => &[("A", 3), ("B", 2), ("G", 1), ("R", 0)],
    };

    let mut out = vec![0x76, 0x2f, 0x31, 0x01];
    // Version 2: a single-part scanline file.
    out.extend(2u32.to_le_bytes());
    let mut list = Vec::new();
    for (name, _) in order {
        list.extend(name.as_bytes());
        list.push(0);
        list.extend(1i32.to_le_bytes()); // HALF
        list.extend([0; 4]); // pLinear, reserved
        list.extend(1i32.to_le_bytes()); // x sampling
        list.extend(1i32.to_le_bytes()); // y sampling
    }
    list.push(0);
    let window: Vec<u8> = [0, 0, max_x, max_y].iter().flat_map(|v| v.to_le_bytes()).collect();
    attribute(&mut out, "channels", "chlist", &list);
    attribute(&mut out, "compression", "compression", &[3]); // ZIP, 16 lines
    attribute(&mut out, "dataWindow", "box2i", &window);
    attribute(&mut out, "displayWindow", "box2i", &window);
    attribute(&mut out, "lineOrder", "lineOrder", &[0]); // increasing y
    attribute(&mut out, "pixelAspectRatio", "float", &1f32.to_le_bytes());
    attribute(&mut out, "screenWindowCenter", "v2f", &[0; 8]);
    attribute(&mut out, "screenWindowWidth", "float", &1f32.to_le_bytes());
    out.push(0);

    // The offset of every block, filled in as the blocks are written.
    let blocks = pixels.chunks(row_length * EXR_BLOCK_LINES);
    let table = out.len();
    out.resize(table + blocks.len() * 8, 0);
    let (mut raw, mut shuffled) = (Vec::new(), Vec::new());
    for (index, block) in blocks.enumerate() {
        let offset = u64::try_from(out.len()).map_err(|_| too_large())?;
        out[table + index * 8..table + index * 8 + 8].copy_from_slice(&offset.to_le_bytes());
        raw.clear();
        for row in block.chunks(row_length) {
            for &(_, channel) in order {
                for pixel in row {
                    raw.extend(half::f16::from_f32(finite_half(pixel[channel])).to_le_bytes());
                }
            }
        }
        let data = compress(&raw, &mut shuffled)?;
        let first_line = i32::try_from(index * EXR_BLOCK_LINES).map_err(|_| too_large())?;
        let size = i32::try_from(data.len()).map_err(|_| too_large())?;
        out.extend(first_line.to_le_bytes());
        out.extend(size.to_le_bytes());
        out.extend_from_slice(&data);
    }
    Ok(out)
}

/// Saves a picture (see [`exr_bytes`]) to `path`, atomically like documents.
pub fn save_exr(path: &Path, width: u32, height: u32, channels: Channels, pixels: &[[f32; 4]]) -> Result<(), IoError> {
    write_atomic(path, &exr_bytes(width, height, channels, pixels)?)
}

/// One header attribute: name, type, size, value.
fn attribute(out: &mut Vec<u8>, name: &str, kind: &str, value: &[u8]) {
    out.extend(name.as_bytes());
    out.push(0);
    out.extend(kind.as_bytes());
    out.push(0);
    // Header values are a few bytes.
    out.extend(i32::try_from(value.len()).unwrap_or(0).to_le_bytes());
    out.extend(value);
}

fn finite_half(value: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(-HALF_MAX, HALF_MAX)
    }
}

/// A block as OpenEXR's ZIP compression stores it: bytes split into the even
/// and the odd ones, each byte replaced by its difference to the one before
/// (plus 128), then zlib. A block that would not get smaller is stored as is
/// (readers tell by its size).
fn compress<'a>(raw: &'a [u8], shuffled: &mut Vec<u8>) -> Result<std::borrow::Cow<'a, [u8]>, IoError> {
    shuffled.clear();
    shuffled.extend(raw.iter().step_by(2));
    shuffled.extend(raw.iter().skip(1).step_by(2));
    if let Some((&mut first, rest)) = shuffled.split_first_mut() {
        let mut previous = first;
        for byte in rest {
            let value = *byte;
            *byte = value.wrapping_sub(previous).wrapping_add(128);
            previous = value;
        }
    }
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(shuffled)?;
    let compressed = encoder.finish()?;
    Ok(if compressed.len() < raw.len() {
        std::borrow::Cow::Owned(compressed)
    } else {
        std::borrow::Cow::Borrowed(raw)
    })
}

fn encoding(err: png::EncodingError) -> IoError {
    match err {
        png::EncodingError::IoError(err) => IoError::Io(err),
        other => IoError::Invalid(format!("could not encode the picture: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_are_written_and_read_back() {
        let pixels = [255, 0, 0, 255, 0, 255, 0, 128];
        let bytes = png_bytes(2, 1, Channels::Rgba, &pixels).unwrap();
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().unwrap();
        let mut decoded = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut decoded).unwrap();
        assert_eq!((info.width, info.height, info.color_type), (2, 1, png::ColorType::Rgba));
        assert_eq!(decoded, pixels);

        let rgb = png_bytes(1, 1, Channels::Rgb, &[1, 2, 3]).unwrap();
        assert!(rgb.starts_with(b"\x89PNG"));
    }

    #[test]
    fn wrong_sizes_are_errors() {
        assert!(png_bytes(2, 2, Channels::Rgb, &[0; 11]).is_err());
        assert!(png_bytes(0, 0, Channels::Rgba, &[]).is_err());
    }

    /// A picture with values a 16-bit float holds exactly.
    fn sample(width: u32, height: u32) -> Vec<[f32; 4]> {
        (0..width * height)
            .map(|i| {
                let v = f32::from(u16::try_from(i % 64).unwrap()) * 0.25;
                [v, 1.0 - v * 0.01, 3.5, if i % 3 == 0 { 0.5 } else { 1.0 }]
            })
            .collect()
    }

    /// Reads an OpenEXR file with the `exr` crate: (name, values) per channel.
    fn decode(bytes: Vec<u8>) -> (exr::math::Vec2<usize>, Vec<(String, Vec<f32>)>) {
        use exr::prelude::{ReadChannels as _, ReadLayers as _};
        let image = exr::prelude::read()
            .no_deep_data()
            .largest_resolution_level()
            .all_channels()
            .all_layers()
            .all_attributes()
            .from_buffered(std::io::Cursor::new(bytes))
            .unwrap();
        let layer = &image.layer_data[0];
        let channels = layer
            .channel_data
            .list
            .iter()
            .map(|channel| {
                let values = channel.sample_data.values_as_f32().collect();
                (channel.name.to_string(), values)
            })
            .collect();
        (layer.size, channels)
    }

    #[test]
    fn exr_files_hold_the_linear_values() {
        // 37 lines: two full blocks and a short one.
        let (width, height) = (19, 37);
        let pixels = sample(width, height);
        let (size, channels) = decode(exr_bytes(width, height, Channels::Rgba, &pixels).unwrap());
        assert_eq!((size.x(), size.y()), (19, 37));
        let names: Vec<&str> = channels.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["A", "B", "G", "R"]);
        for (name, values) in &channels {
            let index = match name.as_str() {
                "R" => 0,
                "G" => 1,
                "B" => 2,
                _ => 3,
            };
            for (value, pixel) in values.iter().zip(&pixels) {
                let expected = half::f16::from_f32(pixel[index]).to_f32();
                assert_eq!(*value, expected, "channel {name}");
            }
        }
        assert!(channels[3].1.iter().any(|&r| r > 1.0), "values above 1 are kept");

        let (_, rgb) = decode(exr_bytes(width, height, Channels::Rgb, &pixels).unwrap());
        assert_eq!(rgb.len(), 3, "no alpha");
    }

    #[test]
    fn exr_clamps_and_rejects_wrong_sizes() {
        let pixels = [[1e9, f32::NAN, -1e9, 1.0]];
        let (_, channels) = decode(exr_bytes(1, 1, Channels::Rgb, &pixels).unwrap());
        let value = |name: &str| channels.iter().find(|(n, _)| n == name).unwrap().1[0];
        assert_eq!((value("R"), value("G"), value("B")), (65_504.0, 0.0, -65_504.0));
        assert!(exr_bytes(2, 2, Channels::Rgba, &pixels).is_err());
    }

    #[test]
    fn flat_pictures_compress() {
        let pixels = vec![[0.5, 0.5, 0.5, 1.0]; 256 * 64];
        let bytes = exr_bytes(256, 64, Channels::Rgba, &pixels).unwrap();
        assert!(bytes.len() < 256 * 64 * 8 / 20, "{} bytes", bytes.len());
    }
}
