//! Materials (see `docs/reference/manager-material.md` and
//! `material-channels.md`): surface properties kept once per document in an
//! ordered table and linked to objects by Material tags.
//!
//! A material is one fixed, principled set of channels (D26): each a value
//! and, optionally, a texture ([`TextureSlot`]) that replaces the value across
//! the surface. The viewport, the path tracer and interchange all read this
//! one model.

use slotmap::SlotMap;

use crate::TextureId;

slotmap::new_key_type! {
    /// Stable, generational handle to a material in [`Materials`]: a removed
    /// material's id never names another one.
    pub struct MaterialId;
}

/// The channels of a material, for the textures they may take.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Channel {
    BaseColor,
    Metallic,
    Roughness,
    Specular,
    /// A tangent-space normal map (texture only).
    Normal,
    /// A height map (texture only; used without a Normal texture).
    Bump,
    Emission,
    Opacity,
    Transmission,
    Clearcoat,
}

impl Channel {
    pub const ALL: [Self; 10] = [
        Self::BaseColor,
        Self::Metallic,
        Self::Roughness,
        Self::Specular,
        Self::Normal,
        Self::Bump,
        Self::Emission,
        Self::Opacity,
        Self::Transmission,
        Self::Clearcoat,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::BaseColor => "Base Color",
            Self::Metallic => "Metallic",
            Self::Roughness => "Roughness",
            Self::Specular => "Specular",
            Self::Normal => "Normal",
            Self::Bump => "Bump",
            Self::Emission => "Emission",
            Self::Opacity => "Opacity",
            Self::Transmission => "Transmission",
            Self::Clearcoat => "Clearcoat",
        }
    }

    /// `true` for channels whose textures hold colour (read as sRGB by
    /// default); the others hold data (read as linear).
    #[must_use]
    pub const fn is_colour(self) -> bool {
        match self {
            Self::BaseColor | Self::Emission => true,
            Self::Metallic
            | Self::Roughness
            | Self::Specular
            | Self::Normal
            | Self::Bump
            | Self::Opacity
            | Self::Transmission
            | Self::Clearcoat => false,
        }
    }

    const fn index(self) -> usize {
        self as usize
    }
}

/// A texture on a channel and how it lies on the surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextureSlot {
    /// The document's texture (a removed one simply does nothing).
    pub texture: TextureId,
    /// Repeats across the UV square (U, V); more than 0.
    pub tiling: [f32; 2],
    /// Shift in picture widths (U, V).
    pub offset: [f32; 2],
    /// The picture's 0 – 1 mapped onto this range (scalar channels).
    pub range: [f32; 2],
    /// From the channel's value (0) to the texture (1).
    pub strength: f32,
}

impl TextureSlot {
    /// `texture` once across the surface, at full strength.
    #[must_use]
    pub const fn new(texture: TextureId) -> Self {
        Self {
            texture,
            tiling: [1.0; 2],
            offset: [0.0; 2],
            range: [0.0, 1.0],
            strength: 1.0,
        }
    }

    fn numbers(&self) -> [f32; 7] {
        let [tu, tv] = self.tiling;
        let [ou, ov] = self.offset;
        let [r0, r1] = self.range;
        [tu, tv, ou, ov, r0, r1, self.strength]
    }
}

/// A surface. Colours are linear RGB.
#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub name: String,
    /// Base Color: a dielectric's colour, a metal's reflection colour (what
    /// the viewport shows).
    pub color: [f32; 3],
    /// 0 mirror-like … 1 matte.
    pub roughness: f32,
    /// 0 dielectric … 1 metal.
    pub metallic: f32,
    /// A dielectric's reflection strength: 0.5 is the usual 4 %.
    pub specular: f32,
    /// How strongly a Normal texture bends the shading (0 – 2).
    pub normal_strength: f32,
    /// How high a Bump texture's white lies (cm).
    pub bump_height: f32,
    /// Light the surface gives off (may exceed 1), times `emission_strength`.
    pub emission: [f32; 3],
    pub emission_strength: f32,
    /// Cut-out: where it is below `opacity_threshold` the surface is not there.
    pub opacity: f32,
    pub opacity_threshold: f32,
    /// Light passing through (glass), bent by `ior`.
    pub transmission: f32,
    pub ior: f32,
    /// A thin varnish over the base, and how rough it is.
    pub clearcoat: f32,
    pub clearcoat_roughness: f32,
    /// Both sides of each polygon shaded alike.
    pub two_sided: bool,
    /// The texture on each channel, in [`Channel::ALL`] order.
    maps: [Option<TextureSlot>; 10],
}

impl Material {
    /// 80 % grey (sRGB), as linear RGB.
    pub const DEFAULT_COLOR: [f32; 3] = [0.604, 0.604, 0.604];
    /// The refractive index range offered (glass is about 1.5).
    pub const IOR_RANGE: (f32, f32) = (1.0, 3.0);

    /// A material with the defaults.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            color: Self::DEFAULT_COLOR,
            roughness: 0.5,
            metallic: 0.0,
            specular: 0.5,
            normal_strength: 1.0,
            bump_height: 0.1,
            emission: [0.0; 3],
            emission_strength: 1.0,
            opacity: 1.0,
            opacity_threshold: 0.5,
            transmission: 0.0,
            ior: 1.5,
            clearcoat: 0.0,
            clearcoat_roughness: 0.03,
            two_sided: false,
            maps: [None; 10],
        }
    }

    /// The texture on `channel`, if any.
    #[must_use]
    pub fn map(&self, channel: Channel) -> Option<&TextureSlot> {
        self.maps[channel.index()].as_ref()
    }

    /// The texture slot of `channel`, to set, change or clear.
    pub fn map_mut(&mut self, channel: Channel) -> &mut Option<TextureSlot> {
        &mut self.maps[channel.index()]
    }

    /// The channels with textures, with them.
    pub fn maps(&self) -> impl Iterator<Item = (Channel, &TextureSlot)> + '_ {
        Channel::ALL
            .into_iter()
            .filter_map(|channel| self.map(channel).map(|slot| (channel, slot)))
    }

    /// The light given off: the colour times the strength.
    #[must_use]
    pub const fn emitted(&self) -> [f32; 3] {
        let [r, g, b] = self.emission;
        let k = self.emission_strength;
        [r * k, g * k, b * k]
    }

    /// Every number, for finiteness checks.
    #[must_use]
    pub fn numbers(&self) -> Vec<f32> {
        let mut out = Vec::with_capacity(24);
        out.extend(self.color);
        out.extend(self.emission);
        out.extend([
            self.roughness,
            self.metallic,
            self.specular,
            self.normal_strength,
            self.bump_height,
            self.emission_strength,
            self.opacity,
            self.opacity_threshold,
            self.transmission,
            self.ior,
            self.clearcoat,
            self.clearcoat_roughness,
        ]);
        for (_, slot) in self.maps() {
            out.extend(slot.numbers());
        }
        out
    }
}

/// The document's materials in Material Manager order.
#[derive(Clone, Debug, Default)]
pub struct Materials {
    table: SlotMap<MaterialId, Material>,
    order: Vec<MaterialId>,
}

impl PartialEq for Materials {
    fn eq(&self, other: &Self) -> bool {
        self.order == other.order && self.order.iter().all(|&id| self.table.get(id) == other.table.get(id))
    }
}

impl Materials {
    #[must_use]
    pub fn len(&self) -> usize {
        self.order.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    #[must_use]
    pub fn get(&self, id: MaterialId) -> Option<&Material> {
        self.table.get(id)
    }

    pub fn get_mut(&mut self, id: MaterialId) -> Option<&mut Material> {
        self.table.get_mut(id)
    }

    #[must_use]
    pub fn contains(&self, id: MaterialId) -> bool {
        self.table.contains_key(id)
    }

    /// Ids in Material Manager order.
    #[must_use]
    pub fn ids(&self) -> &[MaterialId] {
        &self.order
    }

    /// Materials in Material Manager order.
    pub fn iter(&self) -> impl Iterator<Item = (MaterialId, &Material)> + '_ {
        self.order.iter().filter_map(|&id| self.table.get(id).map(|m| (id, m)))
    }

    /// Appends `material`.
    pub fn insert(&mut self, material: Material) -> MaterialId {
        let id = self.table.insert(material);
        self.order.push(id);
        id
    }

    /// Removes the material; `None` if there was none.
    pub fn remove(&mut self, id: MaterialId) -> Option<Material> {
        let material = self.table.remove(id)?;
        self.order.retain(|&m| m != id);
        Some(material)
    }

    /// Position in Material Manager order.
    #[must_use]
    pub fn index_of(&self, id: MaterialId) -> Option<usize> {
        self.order.iter().position(|&m| m == id)
    }

    /// `base` if no material has that name, else `base.1`, `base.2`, … (as for
    /// objects).
    #[must_use]
    pub fn unique_name(&self, base: &str) -> String {
        let taken = |name: &str| self.iter().any(|(_, m)| m.name == name);
        if !taken(base) {
            return base.to_owned();
        }
        (1..)
            .map(|n| format!("{base}.{n}"))
            .find(|name| !taken(name))
            .unwrap_or_else(|| base.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_take_textures_and_every_number_is_counted() {
        let mut textures = crate::Textures::default();
        let wood = textures.insert(crate::Texture::linked("wood.png".into()));
        let mut material = Material::new("Floor");
        assert_eq!(material.maps().count(), 0);
        let before = material.numbers().len();
        *material.map_mut(Channel::Roughness) = Some(TextureSlot {
            tiling: [4.0, 4.0],
            ..TextureSlot::new(wood)
        });
        assert_eq!(material.map(Channel::Roughness).map(|s| s.tiling), Some([4.0, 4.0]));
        assert_eq!(
            material.maps().map(|(c, _)| c).collect::<Vec<_>>(),
            [Channel::Roughness]
        );
        assert_eq!(material.numbers().len(), before + 7);
        assert!(Channel::BaseColor.is_colour() && !Channel::Normal.is_colour());
        material.emission = [1.0, 0.5, 0.0];
        material.emission_strength = 2.0;
        assert_eq!(material.emitted(), [2.0, 1.0, 0.0]);
        assert_eq!(Channel::ALL.map(Channel::index), [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
    }

    #[test]
    fn ordered_table_with_unique_names_and_stable_ids() {
        let mut materials = Materials::default();
        assert!(materials.is_empty());
        let a = materials.insert(Material::new(materials.unique_name("Mat")));
        let b = materials.insert(Material::new(materials.unique_name("Mat")));
        assert_eq!(materials.get(b).unwrap().name, "Mat.1");
        assert_eq!(materials.ids(), &[a, b]);
        assert_eq!(materials.index_of(b), Some(1));

        let snapshot = materials.clone();
        assert!(materials.remove(a).is_some());
        assert!(!materials.contains(a));
        let c = materials.insert(Material::new("Mat"));
        assert_ne!(c, a, "a removed id never names another material");
        assert_ne!(materials, snapshot);
        assert_eq!(snapshot.ids(), &[a, b], "a copy keeps the ids");
        assert_eq!(
            materials.iter().map(|(_, m)| m.name.as_str()).collect::<Vec<_>>(),
            ["Mat.1", "Mat"]
        );
    }
}
