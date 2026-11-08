//! A material's channels as the path tracer reads them at a point of a
//! surface: each channel's value, or towards its picture (see
//! `docs/reference/material-channels.md`; the viewport's shader reads them the
//! same way).

use std::sync::Arc;

use glam::{Vec2, Vec3};
use grid4d_scene::{Channel, Material, TextureSlot};

use crate::{
    picture::{Picture, Pictures},
    scene::Surface,
};

/// A material with its channels' pictures, ready to read.
#[derive(Clone, Debug)]
pub(crate) struct Channels {
    material: Material,
    /// A picture per channel (in `Channel::ALL`'s order), where its texture
    /// has one.
    pictures: [Option<Arc<Picture>>; 10],
}

/// A channel's picture where it is read: its value there and the slot's
/// settings.
struct Read<'a> {
    value: glam::Vec4,
    slot: &'a TextureSlot,
    grey: bool,
}

impl Channels {
    pub(crate) fn new(material: &Material, pictures: &Pictures) -> Self {
        Self {
            material: material.clone(),
            pictures: Channel::ALL.map(|channel| {
                material
                    .map(channel)
                    .and_then(|slot| pictures.get(slot.texture))
                    .map(Arc::clone)
            }),
        }
    }

    fn picture(&self, channel: Channel) -> Option<&Picture> {
        let i = Channel::ALL.iter().position(|&c| c == channel)?;
        self.pictures.get(i)?.as_deref()
    }

    /// Whether any channel reads a picture (the surface needs coordinates).
    pub(crate) fn has_pictures(&self) -> bool {
        self.pictures.iter().any(Option::is_some)
    }

    /// Whether it has a Normal picture (its surfaces need tangents).
    pub(crate) fn has_normal_map(&self) -> bool {
        self.picture(Channel::Normal).is_some()
    }

    /// Whether it may give off light anywhere: an Emission value, or an
    /// Emission picture with a strength.
    pub(crate) fn can_emit(&self) -> bool {
        let m = &self.material;
        m.emitted().iter().any(|&v| v > 0.0) || (self.picture(Channel::Emission).is_some() && m.emission_strength > 0.0)
    }

    /// Whether a Bump picture bends its shading (only without a Normal one).
    pub(crate) fn has_bump_map(&self) -> bool {
        !self.has_normal_map() && self.picture(Channel::Bump).is_some()
    }

    /// The picture of `channel` at the surface's coordinates `uv` (through the
    /// slot's tiling and offset).
    fn read(&self, channel: Channel, uv: Option<Vec2>) -> Option<Read<'_>> {
        let uv = uv?;
        let picture = self.picture(channel)?;
        let slot = self.material.map(channel)?;
        let at = uv * Vec2::from(slot.tiling) + Vec2::from(slot.offset);
        Some(Read {
            value: picture.sample(at),
            slot,
            grey: picture.is_grey(),
        })
    }

    /// A number channel: its value, or towards the picture's red mapped onto
    /// the slot's range.
    fn scalar(&self, channel: Channel, value: f32, uv: Option<Vec2>) -> f32 {
        self.read(channel, uv).map_or(value, |read| {
            let [from, to] = read.slot.range;
            let pictured = from + (to - from) * read.value[read.slot.source.index()];
            value + (pictured - value) * read.slot.strength
        })
    }

    /// A colour channel: its value, or towards the picture (a grey picture
    /// multiplies the value).
    fn colour(&self, channel: Channel, value: Vec3, uv: Option<Vec2>) -> Vec3 {
        self.read(channel, uv).map_or(value, |read| {
            let rgb = read.value.truncate();
            let pictured = if read.grey { value * rgb } else { rgb };
            value.lerp(pictured, read.slot.strength)
        })
    }

    /// Whether the surface is there at `uv`: opacity at or above the threshold.
    pub(crate) fn is_solid(&self, uv: Option<Vec2>) -> bool {
        let m = &self.material;
        self.scalar(Channel::Opacity, m.opacity, uv) >= m.opacity_threshold
    }

    /// The surface at `uv` (`None`: no coordinates, the values alone).
    pub(crate) fn surface(&self, uv: Option<Vec2>) -> Surface {
        let m = &self.material;
        let emitted = Vec3::from(m.emitted());
        let emission = self.read(Channel::Emission, uv).map_or(emitted, |read| {
            // A grey picture scales the light given off, a coloured one is it.
            let rgb = read.value.truncate();
            let pictured = if read.grey {
                emitted * rgb
            } else {
                rgb * m.emission_strength
            };
            emitted.lerp(pictured, read.slot.strength)
        });
        Surface {
            color: self.colour(Channel::BaseColor, Vec3::from(m.color), uv),
            roughness: self.scalar(Channel::Roughness, m.roughness, uv),
            metallic: self.scalar(Channel::Metallic, m.metallic, uv),
            specular: self.scalar(Channel::Specular, m.specular, uv),
            emission,
            transmission: self.scalar(Channel::Transmission, m.transmission, uv),
            ior: m.ior,
            clearcoat: self.scalar(Channel::Clearcoat, m.clearcoat, uv),
            clearcoat_roughness: m.clearcoat_roughness,
        }
    }

    /// The Normal picture at `uv` in tangent space (+Y up the picture), its
    /// bend scaled by the strengths.
    pub(crate) fn normal(&self, uv: Option<Vec2>) -> Option<Vec3> {
        let read = self.read(Channel::Normal, uv)?;
        let bent = read.value.truncate() * 2.0 - Vec3::ONE;
        let scale = self.material.normal_strength * read.slot.strength;
        Some(Vec3::new(bent.x * scale, bent.y * scale, bent.z.max(0.001)))
    }

    /// The Bump picture's height at `uv`, in cm, and how far apart its texels
    /// are in the surface's coordinates (u, v).
    pub(crate) fn height(&self, uv: Vec2) -> Option<(f32, Vec2)> {
        let read = self.read(Channel::Bump, Some(uv))?;
        let [from, to] = read.slot.range;
        let height = (from + (to - from) * read.value[read.slot.source.index()])
            * self.material.bump_height
            * read.slot.strength;
        let (w, h) = self.picture(Channel::Bump)?.size();
        let side =
            |n: u32, tiling: f32| 1.0 / (f32::from(u16::try_from(n).unwrap_or(u16::MAX)) * tiling.abs().max(1e-3));
        Some((
            height,
            Vec2::new(side(w, read.slot.tiling[0]), side(h, read.slot.tiling[1])),
        ))
    }
}

#[cfg(test)]
mod tests {
    use grid4d_scene::{Texture, Textures};

    use super::*;
    use crate::picture::Texels;

    /// A material with `picture` on `channel`, and that picture.
    fn with(channel: Channel, texels: [u8; 8], edit: impl FnOnce(&mut Material, &mut TextureSlot)) -> Channels {
        let mut textures = Textures::default();
        let id = textures.insert(Texture::linked("t.png".into()));
        let mut material = Material::new("M");
        let mut slot = TextureSlot::new(id);
        edit(&mut material, &mut slot);
        *material.map_mut(channel) = Some(slot);
        let mut pictures = Pictures::new();
        let picture = Picture::new(2, 1, Texels::Linear8(Arc::from(texels))).unwrap();
        pictures.insert(id, Arc::new(picture));
        Channels::new(&material, &pictures)
    }

    #[test]
    fn channels_read_their_pictures_like_the_viewport() {
        let left = Some(Vec2::new(0.25, 0.5));
        let right = Some(Vec2::new(0.75, 0.5));
        // A grey picture multiplies the base colour; without coordinates the
        // value stays.
        let grey = with(Channel::BaseColor, [255, 255, 255, 255, 0, 0, 0, 255], |m, _| {
            m.color = [0.5, 0.25, 1.0];
        });
        assert!(grey.surface(left).color.abs_diff_eq(Vec3::new(0.5, 0.25, 1.0), 1e-5));
        assert_eq!(grey.surface(right).color, Vec3::ZERO);
        assert!(grey.surface(None).color.abs_diff_eq(Vec3::new(0.5, 0.25, 1.0), 1e-5));
        // A coloured one replaces it, half way at strength 0.5.
        let red = with(Channel::BaseColor, [255, 0, 0, 255, 255, 0, 0, 255], |m, slot| {
            m.color = [0.0, 0.0, 1.0];
            slot.strength = 0.5;
        });
        assert!(red.surface(left).color.abs_diff_eq(Vec3::new(0.5, 0.0, 0.5), 1e-5));
        // A number channel maps the picture's red onto the slot's range.
        let rough = with(Channel::Roughness, [255, 0, 0, 255, 0, 0, 0, 255], |_, slot| {
            slot.range = [0.2, 0.6]
        });
        assert!((rough.surface(left).roughness - 0.6).abs() < 1e-5);
        assert!((rough.surface(right).roughness - 0.2).abs() < 1e-5);
        // Opacity cuts out below the threshold.
        let cut = with(Channel::Opacity, [255, 0, 0, 255, 0, 0, 0, 255], |_, _| {});
        assert!(cut.is_solid(left) && !cut.is_solid(right));
        assert!(cut.is_solid(None), "no coordinates: the value");
        // Tiling: two repeats across, so u = 0.375 reads the right texel.
        let tiled = with(Channel::Opacity, [255, 0, 0, 255, 0, 0, 0, 255], |_, slot| {
            slot.tiling = [2.0, 1.0]
        });
        assert!(!tiled.is_solid(Some(Vec2::new(0.375, 0.5))));
    }

    #[test]
    fn number_channels_read_the_picture_s_chosen_channel() {
        // Roughness in green, metallic in blue, as glTF packs them.
        let packed = [0, 255, 0, 255, 0, 0, 255, 0];
        let left = Some(Vec2::new(0.25, 0.5));
        let rough = with(Channel::Roughness, packed, |_, slot| {
            slot.source = grid4d_scene::PictureChannel::Green
        });
        assert!((rough.surface(left).roughness - 1.0).abs() < 1e-5);
        let metal = with(Channel::Metallic, packed, |_, slot| {
            slot.source = grid4d_scene::PictureChannel::Blue
        });
        assert!(metal.surface(left).metallic.abs() < 1e-5);
        // A cut-out from alpha: the right texel is transparent.
        let cut = with(Channel::Opacity, packed, |_, slot| {
            slot.source = grid4d_scene::PictureChannel::Alpha
        });
        assert!(cut.is_solid(left) && !cut.is_solid(Some(Vec2::new(0.75, 0.5))));
    }

    #[test]
    fn normal_and_bump_pictures_bend_the_shading() {
        // Flat blue: the normal itself; strength scales the bend only.
        let flat = with(Channel::Normal, [255, 128, 128, 255, 128, 128, 255, 255], |m, _| {
            m.normal_strength = 2.0
        });
        assert!(flat.has_normal_map() && !flat.has_bump_map());
        let n = flat.normal(Some(Vec2::new(0.75, 0.5))).unwrap();
        assert!(n.abs_diff_eq(Vec3::new(0.0, 0.0, 1.0), 0.02), "{n}");
        let tilted = flat.normal(Some(Vec2::new(0.25, 0.5))).unwrap();
        assert!(tilted.x > 1.9 && tilted.z < 0.01, "{tilted}");
        let bump = with(Channel::Bump, [0, 0, 0, 255, 255, 0, 0, 255], |m, _| {
            m.bump_height = 2.0
        });
        assert!(bump.has_bump_map());
        let (low, step) = bump.height(Vec2::new(0.25, 0.5)).unwrap();
        let (high, _) = bump.height(Vec2::new(0.75, 0.5)).unwrap();
        assert!(low.abs() < 1e-5 && (high - 2.0).abs() < 1e-5);
        assert!(step.abs_diff_eq(Vec2::new(0.5, 1.0), 1e-6), "a texel apart");
        assert!(
            Channels::new(&Material::new("Plain"), &Pictures::new())
                .surface(None)
                .color
                == Vec3::from(Material::DEFAULT_COLOR)
        );
    }
}
