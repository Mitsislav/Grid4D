//! Pictures as PNG files (8 bits per channel).

use std::path::Path;

use crate::{IoError, write_atomic};

/// The channels of 8-bit pixel data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PngChannels {
    /// Red, green, blue: three bytes per pixel.
    Rgb,
    /// Red, green, blue, straight (not premultiplied) alpha: four bytes per pixel.
    Rgba,
}

impl PngChannels {
    const fn bytes_per_pixel(self) -> u64 {
        match self {
            Self::Rgb => 3,
            Self::Rgba => 4,
        }
    }
}

/// The PNG file of a `width` × `height` picture of sRGB `pixels`, rows from the top.
pub fn png_bytes(width: u32, height: u32, channels: PngChannels, pixels: &[u8]) -> Result<Vec<u8>, IoError> {
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
        PngChannels::Rgb => png::ColorType::Rgb,
        PngChannels::Rgba => png::ColorType::Rgba,
    });
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder.write_header().map_err(encoding)?;
    writer.write_image_data(pixels).map_err(encoding)?;
    writer.finish().map_err(encoding)?;
    Ok(bytes)
}

/// Saves a picture (see [`png_bytes`]) to `path`, atomically like documents.
pub fn save_png(path: &Path, width: u32, height: u32, channels: PngChannels, pixels: &[u8]) -> Result<(), IoError> {
    write_atomic(path, &png_bytes(width, height, channels, pixels)?)
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
        let bytes = png_bytes(2, 1, PngChannels::Rgba, &pixels).unwrap();
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().unwrap();
        let mut decoded = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut decoded).unwrap();
        assert_eq!((info.width, info.height, info.color_type), (2, 1, png::ColorType::Rgba));
        assert_eq!(decoded, pixels);

        let rgb = png_bytes(1, 1, PngChannels::Rgb, &[1, 2, 3]).unwrap();
        assert!(rgb.starts_with(b"\x89PNG"));
    }

    #[test]
    fn wrong_sizes_are_errors() {
        assert!(png_bytes(2, 2, PngChannels::Rgb, &[0; 11]).is_err());
        assert!(png_bytes(0, 0, PngChannels::Rgba, &[]).is_err());
    }
}
