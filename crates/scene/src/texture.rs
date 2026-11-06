//! Textures (see `docs/reference/material-textures.md`): the pictures a
//! document uses, kept once in an ordered table. A texture is a file on disk
//! (linked) or the file's bytes kept in the document (embedded); decoding and
//! loading happen elsewhere (`grid4d-io`, the interface), never here.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use slotmap::SlotMap;

slotmap::new_key_type! {
    /// Stable, generational handle to a texture in [`Textures`]: a removed
    /// texture's id never names another one.
    pub struct TextureId;
}

/// How a texture's pixel values are read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ColorSpace {
    /// Colour pictures as image files usually store them (photos, paintings).
    #[default]
    Srgb,
    /// Values as they are: data maps (roughness, normals, heights) and HDR /
    /// EXR pictures.
    Linear,
}

impl ColorSpace {
    /// The usual colour space for a file with this extension: linear for
    /// HDR and OpenEXR (stored in linear light), sRGB for the rest.
    #[must_use]
    pub fn for_extension(extension: &str) -> Self {
        if ["hdr", "exr"].iter().any(|e| extension.eq_ignore_ascii_case(e)) {
            Self::Linear
        } else {
            Self::Srgb
        }
    }
}

/// Where a texture's picture comes from.
#[derive(Clone, Debug)]
pub enum TextureSource {
    /// A file. In memory the path is as the user chose it (absolute when
    /// chosen in a dialog); files store it relative to the document's folder
    /// when it lies inside it.
    Linked(PathBuf),
    /// The file's bytes, kept in the document; `file_name` is the original
    /// file's name (its extension tells the format; Unembed suggests it).
    Embedded { file_name: String, bytes: Arc<[u8]> },
}

impl PartialEq for TextureSource {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Linked(a), Self::Linked(b)) => a == b,
            (Self::Embedded { file_name: a, bytes: x }, Self::Embedded { file_name: b, bytes: y }) => {
                a == b && (Arc::ptr_eq(x, y) || x == y)
            }
            (Self::Linked(_), Self::Embedded { .. }) | (Self::Embedded { .. }, Self::Linked(_)) => false,
        }
    }
}

impl TextureSource {
    /// The file name (with its extension), for labels and formats.
    #[must_use]
    pub fn file_name(&self) -> String {
        match self {
            Self::Linked(path) => path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            Self::Embedded { file_name, .. } => file_name.clone(),
        }
    }

    /// The file's extension, lower case (empty without one).
    #[must_use]
    pub fn extension(&self) -> String {
        let name = self.file_name();
        Path::new(&name)
            .extension()
            .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default()
    }
}

/// A picture materials can use.
#[derive(Clone, Debug, PartialEq)]
pub struct Texture {
    pub name: String,
    pub source: TextureSource,
    pub color_space: ColorSpace,
}

impl Texture {
    /// A texture linked to the file at `path`, named after it, in the usual
    /// colour space for its extension.
    #[must_use]
    pub fn linked(path: PathBuf) -> Self {
        let source = TextureSource::Linked(path);
        let name = Path::new(&source.file_name())
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let color_space = ColorSpace::for_extension(&source.extension());
        Self {
            name,
            source,
            color_space,
        }
    }
}

/// The document's textures in Texture Manager order.
#[derive(Clone, Debug, Default)]
pub struct Textures {
    table: SlotMap<TextureId, Texture>,
    order: Vec<TextureId>,
}

impl PartialEq for Textures {
    fn eq(&self, other: &Self) -> bool {
        self.order == other.order && self.order.iter().all(|&id| self.table.get(id) == other.table.get(id))
    }
}

impl Textures {
    #[must_use]
    pub fn len(&self) -> usize {
        self.order.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    #[must_use]
    pub fn get(&self, id: TextureId) -> Option<&Texture> {
        self.table.get(id)
    }

    pub fn get_mut(&mut self, id: TextureId) -> Option<&mut Texture> {
        self.table.get_mut(id)
    }

    #[must_use]
    pub fn contains(&self, id: TextureId) -> bool {
        self.table.contains_key(id)
    }

    /// Ids in Texture Manager order.
    #[must_use]
    pub fn ids(&self) -> &[TextureId] {
        &self.order
    }

    /// Textures in Texture Manager order.
    pub fn iter(&self) -> impl Iterator<Item = (TextureId, &Texture)> + '_ {
        self.order.iter().filter_map(|&id| self.table.get(id).map(|t| (id, t)))
    }

    /// Appends `texture`.
    pub fn insert(&mut self, texture: Texture) -> TextureId {
        let id = self.table.insert(texture);
        self.order.push(id);
        id
    }

    /// Removes the texture; `None` if there was none.
    pub fn remove(&mut self, id: TextureId) -> Option<Texture> {
        let texture = self.table.remove(id)?;
        self.order.retain(|&t| t != id);
        Some(texture)
    }

    /// The texture linked to exactly `path`, if any.
    #[must_use]
    pub fn linked_to(&self, path: &Path) -> Option<TextureId> {
        self.iter()
            .find(|(_, t)| matches!(&t.source, TextureSource::Linked(p) if p == path))
            .map(|(id, _)| id)
    }

    /// `base` if no texture has that name, else `base.1`, `base.2`, … (as for
    /// objects and materials).
    #[must_use]
    pub fn unique_name(&self, base: &str) -> String {
        let taken = |name: &str| self.iter().any(|(_, t)| t.name == name);
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
    fn linked_textures_are_named_after_their_files() {
        let wood = Texture::linked(PathBuf::from("maps/Wood Floor.JPG"));
        assert_eq!(wood.name, "Wood Floor");
        assert_eq!(wood.source.extension(), "jpg");
        assert_eq!(wood.color_space, ColorSpace::Srgb);
        let sky = Texture::linked(PathBuf::from("sky.HDR"));
        assert_eq!(sky.color_space, ColorSpace::Linear, "HDR is linear light");
        assert_eq!(ColorSpace::for_extension("exr"), ColorSpace::Linear);
    }

    #[test]
    fn ordered_table_with_unique_names_and_stable_ids() {
        let mut textures = Textures::default();
        let a = textures.insert(Texture::linked(PathBuf::from("a.png")));
        let b = textures.insert(Texture {
            name: textures.unique_name("a"),
            source: TextureSource::Embedded {
                file_name: "b.png".to_owned(),
                bytes: Arc::from([1u8, 2, 3].as_slice()),
            },
            color_space: ColorSpace::Linear,
        });
        assert_eq!(textures.get(b).unwrap().name, "a.1");
        assert_eq!(textures.ids(), &[a, b]);
        assert_eq!(textures.linked_to(Path::new("a.png")), Some(a));
        assert_eq!(
            textures.linked_to(Path::new("b.png")),
            None,
            "embedded ones are not linked"
        );

        let copy = textures.clone();
        assert_eq!(copy, textures, "copies share the bytes and compare equal");
        textures.remove(a);
        let c = textures.insert(Texture::linked(PathBuf::from("a.png")));
        assert_ne!(c, a, "a removed id never names another texture");
        assert_ne!(textures, copy);
        let mut changed = copy.clone();
        if let Some(TextureSource::Embedded { bytes, .. }) = changed.get_mut(b).map(|t| &mut t.source) {
            *bytes = Arc::from([1u8, 2, 4].as_slice());
        }
        assert_ne!(changed, copy, "different bytes differ");
    }
}
