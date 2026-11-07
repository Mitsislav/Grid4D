//! Format version 5 (current, Grid4D 1.2 in development): a zip archive.
//! `document.json` holds the document (under the tag `"grid4d"`), and each mesh
//! is a binary file `meshes/<index>.bin`.
//!
//! A mesh file is little-endian, with the sizes given in `document.json`:
//! `points × 3` f32 (x, y, z), then `faces` u32 corner counts, then `corners` u32
//! point indices (faces counter-clockwise, in order), then — if `uvs` —
//! `corners × 2` f32 UVs.
//!
//! Version 5 is version 4 (Grid4D 1.1, frozen) plus what Phase 16 adds (render
//! quality: area lights, the sky and environment, lens and shutter, passes,
//! sampling); version 4 files are read through these types after
//! [`crate::v4::upgrade`], version 3 files after [`crate::v3::upgrade`] too.
//! Until Grid4D 1.2 ships version 5 grows only by optional fields left out at
//! their defaults and by new types, each listed in `v4.rs` (what a version 4
//! file must not carry) and `docs/FILE_FORMAT.md`; then it is frozen like
//! version 4.

use std::{
    collections::HashMap,
    io::{Cursor, Write as _},
};

use glam::{Vec2, Vec3};
use grid4d_geom::{
    BitSet, ComponentSet, PolyMesh,
    cloner::{CloneOrder, Cloner, Distribution, GridArray, Layout, Linear, Radial, StepMode, Surface},
    deform::{Bend, BendOrigin, Bulge, DeformerMode, Ffd, Shape, Taper, Twist},
    effector::{Effector, Falloff as EffectorFalloff, FalloffShape},
    spline::{
        ArcType, Interpolation as SplineInterpolation, Segment, Spline, SplinePlane, SplinePrimitive, SplineShape,
        SplineType, Text, TextAlign,
    },
    subdivision::{Boundary, MAX_LEVEL},
    surface::{CapType, ExtrudeDirection, Extrusion, Lathe, Loft, Sweep},
};
use grid4d_scene::{
    Axis, Body, CachedBody, CollisionShape, ComponentState, Components, Curve, Document, Falloff, FrameNaming,
    FrameRange, ImageFormat, Inherit, Interpolation, Key, Layer, LayerId, LayerSwitches, LightDetails, LightShadow,
    LightType, MassMode, Material, MaterialId, MeshHandle, MeshId, Motion, NodeData, NodeId, ObjectKind,
    ObjectOverrides, ObjectParam, Override, Param, PluginObject, PluginValue, RenderSettings, SimulationCache,
    SimulationSettings, SplineHandle, Tag, Take, TakeId, Takes, Tangents, TimeSettings, Track, Transform,
};
use grid4d_scene::{
    Channel, ColorSpace, MaterialTag, PictureChannel, Placement, Projection, Texture, TextureId, TextureSlot,
    TextureSource,
};
use serde::{Deserialize, Serialize};
use zip::{CompressionMethod, DateTime, ZipWriter, write::SimpleFileOptions};

use crate::{
    FORMAT_TAG, IoError,
    common::{Archive, assemble_with_ids, check_finite},
};

pub(crate) const VERSION: u32 = 5;
/// The archive entry holding everything but the meshes' arrays.
pub(crate) const DOCUMENT_ENTRY: &str = "document.json";

fn mesh_entry(index: usize) -> String {
    format!("meshes/{index}.bin")
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct File {
    format: String,
    version: u32,
    document: DocumentDto,
    /// Sizes of `meshes/0.bin`, `meshes/1.bin`, …
    meshes: Vec<MeshDto>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DocumentDto {
    name: String,
    time: TimeDto,
    objects: Vec<ObjectDto>,
    /// Added in the course of version 3 (step 6.6); absent when there are none.
    /// Material tags refer to them by index.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    materials: Vec<MaterialDto>,
    /// Added in the course of version 3 (step 12.4); absent when there are
    /// none. Objects refer to them by index.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    layers: Vec<LayerDto>,
    /// Added in the course of version 3 (step 12.5); absent when there are
    /// none. The objects hold the active take's values; `main_values` Main's
    /// beside them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    takes: Option<TakesDto>,
    /// Added in the course of version 3 (step 8.2); absent while they are the
    /// defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    render: Option<RenderDto>,
    /// Added in the course of version 3 (step 11.1); absent while they are the
    /// defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    simulation: Option<SimulationDto>,
    /// Added in step 11.5: the simulated frames, when saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    simulation_cache: Option<SimCacheDto>,
    /// Added in version 4 (step 15.3): the document's textures, linked or
    /// embedded (`textures/<n>.<ext>`); absent when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    textures: Vec<TextureDto>,
}

/// A texture: a linked `file` (relative to the document's folder when it lies
/// inside it, with `/` between folders) or an `embedded` file's original
/// name, whose bytes are the entry `textures/<index>.<ext>`.
#[derive(Debug, Serialize, Deserialize)]
struct TextureDto {
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    embedded: Option<String>,
    #[serde(default, skip_serializing_if = "ColorSpaceDto::is_srgb")]
    color_space: ColorSpaceDto,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ColorSpaceDto {
    #[default]
    Srgb,
    Linear,
}

impl ColorSpaceDto {
    #[allow(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )]
    fn is_srgb(&self) -> bool {
        *self == Self::Srgb
    }
}

/// The most an embedded texture's entry may hold.
const MAX_TEXTURE_BYTES: u64 = 1 << 30;

/// The archive entry of embedded texture `index` with original file `name`:
/// `textures/<index>.<ext>`, the extension lower case and plain (else `bin`).
fn texture_entry(index: usize, name: &str) -> String {
    let ext = std::path::Path::new(name)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .filter(|e| !e.is_empty() && e.len() <= 8 && e.bytes().all(|b| b.is_ascii_alphanumeric()))
        .unwrap_or_else(|| "bin".to_owned());
    format!("textures/{index}.{ext}")
}

/// `path` as a file stores it: relative to `folder` (with `/`) when it lies
/// inside it, else as it is.
fn path_text(path: &std::path::Path, folder: Option<&std::path::Path>) -> String {
    if let Some(inside) = folder.and_then(|folder| path.strip_prefix(folder).ok()) {
        let parts: Vec<String> = inside
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        return parts.join("/");
    }
    if path.is_relative() {
        // Kept relative: written with `/`, whatever the system.
        let parts: Vec<String> = path
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        return parts.join("/");
    }
    path.to_string_lossy().into_owned()
}

/// A stored path: a relative one is taken from `folder` (if known).
fn read_path(text: &str, folder: Option<&std::path::Path>) -> std::path::PathBuf {
    let relative: std::path::PathBuf = text.split('/').collect();
    match folder {
        Some(folder) if std::path::Path::new(text).is_relative() && !text.contains('\\') => folder.join(relative),
        _ if std::path::Path::new(text).is_relative() && !text.contains('\\') => relative,
        _ => std::path::PathBuf::from(text),
    }
}

impl TextureDto {
    fn new(texture: &Texture, folder: Option<&std::path::Path>) -> Self {
        let (file, embedded) = match &texture.source {
            TextureSource::Linked(path) => (Some(path_text(path, folder)), None),
            TextureSource::Embedded { file_name, .. } => (None, Some(file_name.clone())),
        };
        Self {
            name: texture.name.clone(),
            file,
            embedded,
            color_space: match texture.color_space {
                ColorSpace::Srgb => ColorSpaceDto::Srgb,
                ColorSpace::Linear => ColorSpaceDto::Linear,
            },
        }
    }

    fn into_texture(
        self,
        index: usize,
        archive: &mut Archive<'_>,
        folder: Option<&std::path::Path>,
    ) -> Result<Texture, IoError> {
        let source = match (self.file, self.embedded) {
            (Some(file), None) => TextureSource::Linked(read_path(&file, folder)),
            (None, Some(file_name)) => {
                let bytes = archive.read(&texture_entry(index, &file_name), MAX_TEXTURE_BYTES)?;
                TextureSource::Embedded {
                    file_name,
                    bytes: bytes.into(),
                }
            }
            _ => {
                return Err(IoError::Invalid(format!(
                    "texture \"{}\" needs either a file or an embedded picture",
                    self.name
                )));
            }
        };
        Ok(Texture {
            name: self.name,
            source,
            color_space: match self.color_space {
                ColorSpaceDto::Srgb => ColorSpace::Srgb,
                ColorSpaceDto::Linear => ColorSpace::Linear,
            },
        })
    }
}

/// The takes, the active one and Main's values beside it.
#[derive(Debug, Serialize, Deserialize)]
struct TakesDto {
    takes: Vec<TakeDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active: Option<usize>,
    /// Render Settings ▸ Take.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    render: Option<usize>,
    /// Main's values of what the active take overrides.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    main_values: Vec<OverridesDto>,
    /// Main's render settings while the active take has its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    main_render: Option<OwnRenderDto>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TakeDto {
    name: String,
    /// The camera object's index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    camera: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    render: Option<OwnRenderDto>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    objects: Vec<OverridesDto>,
}

/// Render settings a take holds (`settings` left out while the defaults).
#[derive(Debug, Serialize, Deserialize)]
struct OwnRenderDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    settings: Option<RenderDto>,
}

impl OwnRenderDto {
    fn new(settings: &RenderSettings) -> Self {
        Self {
            settings: RenderDto::new(settings),
        }
    }

    fn into_settings(self) -> RenderSettings {
        self.settings.map(RenderDto::into_settings).unwrap_or_default()
    }
}

/// One object's overridden groups.
#[derive(Debug, Serialize, Deserialize)]
struct OverridesDto {
    object: usize,
    /// Position, rotation (heading, pitch, bank in radians), scale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    transform: Option<[[f32; 3]; 3]>,
    /// Visible in the editor, in the renderer, enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    visibility: Option<[bool; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parameters: Option<KindDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tags: Option<Vec<TagDto>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    display_color: Option<[f32; 3]>,
}

impl OverridesDto {
    /// `None` for an object that is not saved (deleted).
    fn new(id: NodeId, values: &[Override], refs: &Refs, meshes: &mut MeshTable) -> Option<Self> {
        let mut dto = Self {
            object: *refs.objects.get(&id)?,
            transform: None,
            visibility: None,
            parameters: None,
            tags: None,
            display_color: None,
        };
        for value in values {
            match value {
                Override::Transform(t) => {
                    dto.transform = Some([t.position.to_array(), t.rotation.to_array(), t.scale.to_array()]);
                }
                Override::Visibility {
                    editor,
                    render,
                    enabled,
                } => dto.visibility = Some([*editor, *render, *enabled]),
                Override::Parameters(kind) => dto.parameters = Some(KindDto::new(kind, "take", meshes)),
                Override::Tags(tags) => dto.tags = Some(tags.iter().map(|tag| TagDto::new(tag, refs)).collect()),
                Override::DisplayColor(colour) => dto.display_color = Some(*colour),
            }
        }
        Some(dto)
    }

    fn check_finite(&self) -> Result<(), IoError> {
        let transform: Vec<f32> = self.transform.iter().flatten().flatten().copied().collect();
        let colour: Vec<f32> = self.display_color.iter().flatten().copied().collect();
        let parameters = self.parameters.as_ref().map(KindDto::parameters).unwrap_or_default();
        let tags: Vec<f32> = self.tags.iter().flatten().flat_map(TagDto::parameters).collect();
        check_finite(
            "a take's override",
            &[
                ("transform", &transform),
                ("display colour", &colour),
                ("parameters", &parameters),
                ("tag", &tags),
            ],
        )
    }

    /// The object's id and values; `None` for an index that names no object.
    fn into_values(
        self,
        ids: &[NodeId],
        materials: &[MaterialId],
        meshes: &[MeshHandle],
    ) -> Result<Option<(NodeId, Vec<Override>)>, IoError> {
        let Some(&id) = ids.get(self.object) else {
            return Ok(None);
        };
        let mut values = Vec::new();
        if let Some([position, rotation, scale]) = self.transform {
            values.push(Override::Transform(Transform {
                position: Vec3::from_array(position),
                rotation: Vec3::from_array(rotation),
                scale: Vec3::from_array(scale),
            }));
        }
        if let Some([editor, render, enabled]) = self.visibility {
            values.push(Override::Visibility {
                editor,
                render,
                enabled,
            });
        }
        if let Some(kind) = self.parameters {
            values.push(Override::Parameters(kind.into_kind(meshes, "take")?));
        }
        if let Some(tags) = self.tags {
            values.push(Override::Tags(
                tags.into_iter().map(|tag| tag.into_tag(ids, materials)).collect(),
            ));
        }
        if let Some(colour) = self.display_color {
            values.push(Override::DisplayColor(colour));
        }
        Ok(Some((id, values)))
    }
}

/// A layer; its colour is linear RGB.
#[derive(Debug, Serialize, Deserialize)]
#[expect(clippy::struct_excessive_bools, reason = "the Layer Manager's switches")]
struct LayerDto {
    name: String,
    color: [f32; 3],
    solo: bool,
    view: bool,
    render: bool,
    manager: bool,
    locked: bool,
    animation: bool,
    generators: bool,
    deformers: bool,
}

impl LayerDto {
    fn new(layer: &Layer) -> Self {
        let s = layer.switches;
        Self {
            name: layer.name.clone(),
            color: layer.color,
            solo: s.solo,
            view: s.view,
            render: s.render,
            manager: s.manager,
            locked: s.locked,
            animation: s.animation,
            generators: s.generators,
            deformers: s.deformers,
        }
    }

    fn into_layer(self) -> Result<Layer, IoError> {
        if !self.color.iter().all(|v| v.is_finite()) {
            return Err(IoError::Invalid(format!(
                "layer \"{}\" has a colour that is not a finite number",
                self.name
            )));
        }
        Ok(Layer {
            name: self.name,
            color: self.color,
            switches: LayerSwitches {
                solo: self.solo,
                view: self.view,
                render: self.render,
                manager: self.manager,
                locked: self.locked,
                animation: self.animation,
                generators: self.generators,
                deformers: self.deformers,
            },
        })
    }
}

/// A material; colours are linear RGB.
#[derive(Debug, Serialize, Deserialize)]
struct MaterialDto {
    name: String,
    color: [f32; 3],
    roughness: f32,
    metallic: f32,
    emission: [f32; 3],
    /// Added in version 4 (step 15.4): the other channels, each left out
    /// while it has its default.
    #[serde(default = "half", skip_serializing_if = "is_half")]
    specular: f32,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    normal_strength: f32,
    #[serde(default = "default_bump", skip_serializing_if = "is_default_bump")]
    bump_height: f32,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    emission_strength: f32,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    opacity: f32,
    #[serde(default = "half", skip_serializing_if = "is_half")]
    opacity_threshold: f32,
    #[serde(default, skip_serializing_if = "is_zero")]
    transmission: f32,
    #[serde(default = "default_ior", skip_serializing_if = "is_default_ior")]
    ior: f32,
    #[serde(default, skip_serializing_if = "is_zero")]
    clearcoat: f32,
    #[serde(default = "default_coat", skip_serializing_if = "is_default_coat")]
    clearcoat_roughness: f32,
    #[serde(default, skip_serializing_if = "is_false")]
    two_sided: bool,
    /// The channels' textures, by channel name; absent when there are none.
    #[serde(default, skip_serializing_if = "MapsDto::is_empty")]
    maps: MapsDto,
}

const fn half() -> f32 {
    0.5
}

#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_half(value: &f32) -> bool {
    *value == 0.5
}

const fn default_bump() -> f32 {
    0.1
}

#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_default_bump(value: &f32) -> bool {
    *value == 0.1
}

const fn default_ior() -> f32 {
    1.5
}

#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_default_ior(value: &f32) -> bool {
    *value == 1.5
}

const fn default_coat() -> f32 {
    0.03
}

#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_default_coat(value: &f32) -> bool {
    *value == 0.03
}

/// The textures of a material's channels.
#[derive(Debug, Default, Serialize, Deserialize)]
struct MapsDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    base_color: Option<SlotDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    metallic: Option<SlotDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    roughness: Option<SlotDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    specular: Option<SlotDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    normal: Option<SlotDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bump: Option<SlotDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    emission: Option<SlotDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    opacity: Option<SlotDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    transmission: Option<SlotDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    clearcoat: Option<SlotDto>,
}

impl MapsDto {
    fn is_empty(&self) -> bool {
        Channel::ALL.into_iter().all(|channel| self.get(channel).is_none())
    }

    fn get(&self, channel: Channel) -> Option<&SlotDto> {
        self.slot(channel).as_ref()
    }

    const fn slot(&self, channel: Channel) -> &Option<SlotDto> {
        match channel {
            Channel::BaseColor => &self.base_color,
            Channel::Metallic => &self.metallic,
            Channel::Roughness => &self.roughness,
            Channel::Specular => &self.specular,
            Channel::Normal => &self.normal,
            Channel::Bump => &self.bump,
            Channel::Emission => &self.emission,
            Channel::Opacity => &self.opacity,
            Channel::Transmission => &self.transmission,
            Channel::Clearcoat => &self.clearcoat,
        }
    }

    const fn slot_mut(&mut self, channel: Channel) -> &mut Option<SlotDto> {
        match channel {
            Channel::BaseColor => &mut self.base_color,
            Channel::Metallic => &mut self.metallic,
            Channel::Roughness => &mut self.roughness,
            Channel::Specular => &mut self.specular,
            Channel::Normal => &mut self.normal,
            Channel::Bump => &mut self.bump,
            Channel::Emission => &mut self.emission,
            Channel::Opacity => &mut self.opacity,
            Channel::Transmission => &mut self.transmission,
            Channel::Clearcoat => &mut self.clearcoat,
        }
    }
}

/// A channel's texture: `texture` is an index into `document.textures`.
#[derive(Debug, Serialize, Deserialize)]
struct SlotDto {
    texture: usize,
    #[serde(default = "unit_pair", skip_serializing_if = "is_unit_pair")]
    tiling: [f32; 2],
    #[serde(default, skip_serializing_if = "is_zero_pair")]
    offset: [f32; 2],
    #[serde(default = "unit_range", skip_serializing_if = "is_unit_range")]
    range: [f32; 2],
    #[serde(default = "one", skip_serializing_if = "is_one")]
    strength: f32,
    /// The picture's channel a number channel reads (added in step 15.8;
    /// red when absent).
    #[serde(default, skip_serializing_if = "PictureChannelDto::is_red")]
    channel: PictureChannelDto,
}

/// [`PictureChannel`] in the file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PictureChannelDto {
    #[default]
    Red,
    Green,
    Blue,
    Alpha,
}

impl PictureChannelDto {
    #[allow(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )]
    fn is_red(&self) -> bool {
        *self == Self::Red
    }
}

impl From<PictureChannel> for PictureChannelDto {
    fn from(channel: PictureChannel) -> Self {
        match channel {
            PictureChannel::Red => Self::Red,
            PictureChannel::Green => Self::Green,
            PictureChannel::Blue => Self::Blue,
            PictureChannel::Alpha => Self::Alpha,
        }
    }
}

impl From<PictureChannelDto> for PictureChannel {
    fn from(channel: PictureChannelDto) -> Self {
        match channel {
            PictureChannelDto::Red => Self::Red,
            PictureChannelDto::Green => Self::Green,
            PictureChannelDto::Blue => Self::Blue,
            PictureChannelDto::Alpha => Self::Alpha,
        }
    }
}

const fn unit_pair() -> [f32; 2] {
    [1.0, 1.0]
}

#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_unit_pair(value: &[f32; 2]) -> bool {
    *value == [1.0, 1.0]
}

#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_zero_pair(value: &[f32; 2]) -> bool {
    *value == [0.0, 0.0]
}

const fn unit_range() -> [f32; 2] {
    [0.0, 1.0]
}

#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_unit_range(value: &[f32; 2]) -> bool {
    *value == [0.0, 1.0]
}

impl MaterialDto {
    fn new(material: &Material, refs: &Refs) -> Self {
        let mut maps = MapsDto::default();
        for (channel, slot) in material.maps() {
            // A link to a removed texture is not written (it did nothing).
            if let Some(&texture) = refs.textures.get(&slot.texture) {
                *maps.slot_mut(channel) = Some(SlotDto {
                    texture,
                    tiling: slot.tiling,
                    offset: slot.offset,
                    range: slot.range,
                    strength: slot.strength,
                    channel: slot.source.into(),
                });
            }
        }
        Self {
            name: material.name.clone(),
            color: material.color,
            roughness: material.roughness,
            metallic: material.metallic,
            emission: material.emission,
            specular: material.specular,
            normal_strength: material.normal_strength,
            bump_height: material.bump_height,
            emission_strength: material.emission_strength,
            opacity: material.opacity,
            opacity_threshold: material.opacity_threshold,
            transmission: material.transmission,
            ior: material.ior,
            clearcoat: material.clearcoat,
            clearcoat_roughness: material.clearcoat_roughness,
            two_sided: material.two_sided,
            maps,
        }
    }

    /// The material; values out of range are clamped, texture indices that
    /// name no texture are dropped.
    fn into_material(self, textures: &[TextureId]) -> Result<Material, IoError> {
        let unit = |v: f32| v.clamp(0.0, 1.0);
        let mut material = Material::new(self.name);
        material.color = self.color.map(|c| c.max(0.0));
        material.roughness = unit(self.roughness);
        material.metallic = unit(self.metallic);
        material.emission = self.emission.map(|c| c.max(0.0));
        material.specular = unit(self.specular);
        material.normal_strength = self.normal_strength.clamp(0.0, 2.0);
        material.bump_height = self.bump_height.max(0.0);
        material.emission_strength = self.emission_strength.max(0.0);
        material.opacity = unit(self.opacity);
        material.opacity_threshold = unit(self.opacity_threshold);
        material.transmission = unit(self.transmission);
        material.ior = self.ior.clamp(Material::IOR_RANGE.0, Material::IOR_RANGE.1);
        material.clearcoat = unit(self.clearcoat);
        material.clearcoat_roughness = unit(self.clearcoat_roughness);
        material.two_sided = self.two_sided;
        for channel in Channel::ALL {
            let Some(slot) = self.maps.get(channel) else {
                continue;
            };
            let Some(&texture) = textures.get(slot.texture) else {
                continue;
            };
            *material.map_mut(channel) = Some(TextureSlot {
                texture,
                tiling: slot.tiling.map(|t| if t.abs() < 1e-6 { 1.0 } else { t }),
                offset: slot.offset,
                range: slot.range,
                strength: unit(slot.strength),
                source: slot.channel.into(),
            });
        }
        check_material_finite(&material)?;
        Ok(material)
    }
}

fn check_material_finite(material: &Material) -> Result<(), IoError> {
    if material.numbers().iter().all(|v| v.is_finite()) {
        Ok(())
    } else {
        Err(IoError::Invalid(format!(
            "material \"{}\" has a value that is not a finite number",
            material.name
        )))
    }
}

/// Indices that tags write instead of ids.
struct Refs {
    objects: HashMap<NodeId, usize>,
    materials: HashMap<MaterialId, usize>,
    layers: HashMap<LayerId, usize>,
    textures: HashMap<TextureId, usize>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TimeDto {
    fps: u32,
    start: i32,
    end: i32,
    looping: bool,
}

/// The render settings; out-of-range values are clamped on loading.
#[derive(Debug, Serialize, Deserialize)]
struct RenderDto {
    width: u32,
    height: u32,
    lock_ratio: bool,
    frames: FrameRangeDto,
    frame_step: u32,
    samples: u32,
    bounces: u32,
    auto_light: bool,
    save: bool,
    output: String,
    alpha: bool,
    /// Added with step 8.5; PNG when absent.
    #[serde(default, skip_serializing_if = "ImageFormatDto::is_png")]
    format: ImageFormatDto,
    /// Added with step 8.5; `name0000` when absent.
    #[serde(default, skip_serializing_if = "FrameNamingDto::is_default")]
    naming: FrameNamingDto,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ImageFormatDto {
    #[default]
    Png,
    Exr,
}

impl ImageFormatDto {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )]
    fn is_png(&self) -> bool {
        *self == Self::Png
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FrameNamingDto {
    #[default]
    Name0000,
    NameDot0000,
    NameUnderscore0000,
}

impl FrameNamingDto {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )]
    fn is_default(&self) -> bool {
        *self == Self::Name0000
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FrameRangeDto {
    Current,
    All,
    Manual { from: i32, to: i32 },
}

impl RenderDto {
    /// `None` for the defaults, which files leave out.
    fn new(settings: &RenderSettings) -> Option<Self> {
        if *settings == RenderSettings::default() {
            return None;
        }
        let RenderSettings {
            width,
            height,
            lock_ratio,
            frames,
            frame_step,
            samples,
            bounces,
            auto_light,
            save,
            ref output,
            alpha,
            format,
            naming,
        } = *settings;
        Some(Self {
            width,
            height,
            lock_ratio,
            frames: match frames {
                FrameRange::Current => FrameRangeDto::Current,
                FrameRange::All => FrameRangeDto::All,
                FrameRange::Manual { from, to } => FrameRangeDto::Manual { from, to },
            },
            frame_step,
            samples,
            bounces,
            auto_light,
            save,
            output: output.clone(),
            alpha,
            format: match format {
                ImageFormat::Png => ImageFormatDto::Png,
                ImageFormat::Exr => ImageFormatDto::Exr,
            },
            naming: match naming {
                FrameNaming::Name0000 => FrameNamingDto::Name0000,
                FrameNaming::NameDot0000 => FrameNamingDto::NameDot0000,
                FrameNaming::NameUnderscore0000 => FrameNamingDto::NameUnderscore0000,
            },
        })
    }

    fn into_settings(self) -> RenderSettings {
        RenderSettings {
            width: self.width,
            height: self.height,
            lock_ratio: self.lock_ratio,
            frames: match self.frames {
                FrameRangeDto::Current => FrameRange::Current,
                FrameRangeDto::All => FrameRange::All,
                FrameRangeDto::Manual { from, to } => FrameRange::Manual { from, to },
            },
            frame_step: self.frame_step,
            samples: self.samples,
            bounces: self.bounces,
            auto_light: self.auto_light,
            save: self.save,
            output: self.output,
            alpha: self.alpha,
            format: match self.format {
                ImageFormatDto::Png => ImageFormat::Png,
                ImageFormatDto::Exr => ImageFormat::Exr,
            },
            naming: match self.naming {
                FrameNamingDto::Name0000 => FrameNaming::Name0000,
                FrameNamingDto::NameDot0000 => FrameNaming::NameDot0000,
                FrameNamingDto::NameUnderscore0000 => FrameNaming::NameUnderscore0000,
            },
        }
        .validated()
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ObjectDto {
    name: String,
    /// Index of the parent in `objects`, always smaller than this object's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent: Option<usize>,
    kind: KindDto,
    position: [f32; 3],
    /// Heading, pitch, bank in radians.
    rotation_hpb: [f32; 3],
    scale: [f32; 3],
    /// Linear RGB.
    display_color: [f32; 3],
    visible_in_editor: bool,
    visible_in_render: bool,
    enabled: bool,
    /// Added in the course of version 3; absent when the object has none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<TagDto>,
    /// Added in the course of version 3; absent when the object is not animated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tracks: Vec<TrackDto>,
    /// Added in the course of version 3 (step 12.4): its own layer's index in
    /// `layers`; absent without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    layer: Option<usize>,
}

/// An animated parameter and its keys.
#[derive(Debug, Serialize, Deserialize)]
struct TrackDto {
    param: ParamDto,
    keys: Vec<KeyDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ParamDto {
    PositionX,
    PositionY,
    PositionZ,
    /// Radians, like `rotation_hpb`.
    RotationH,
    RotationP,
    RotationB,
    ScaleX,
    ScaleY,
    ScaleZ,
    // An object's own parameters (added in step 10.1), named by meaning.
    SizeX,
    SizeY,
    SizeZ,
    Radius,
    Height,
    Width,
    TopRadius,
    BottomRadius,
    InnerRadius,
    OuterRadius,
    RingRadius,
    PipeRadius,
    EllipseRadius,
    RoundingRadius,
    StartRadius,
    EndRadius,
    StartAngle,
    EndAngle,
    Twist,
    RadialBias,
    HeightBias,
    HorizontalSpacing,
    VerticalSpacing,
    FieldOfView,
    Intensity,
    InnerAngle,
    OuterAngle,
    FalloffRadius,
    Offset,
    MovementX,
    MovementY,
    MovementZ,
    Angle,
    Rise,
    Scaling,
    EndScale,
    EndRotation,
    StartGrowth,
    EndGrowth,
    Strength,
    Amount,
    Curvature,
    /// A plugin object's number, by its name (added in step 13.2):
    /// `{"plugin": "Phase"}`.
    Plugin(String),
}

impl ParamDto {
    /// `param` of an object of `kind` (a plugin's number by its name).
    fn new(param: Param, kind: &ObjectKind) -> Self {
        match param {
            Param::Object(p @ ObjectParam::Plugin(_)) => Self::Plugin(kind.param_label(p).to_owned()),
            _ => param.into(),
        }
    }

    /// The parameter of an object of `kind` this names; `None` for a plugin's
    /// number the object has no value of.
    fn into_param(self, kind: &ObjectKind) -> Option<Param> {
        let Self::Plugin(name) = self else {
            return Some(self.into());
        };
        let ObjectKind::Plugin(object) = kind else {
            return None;
        };
        let index = object.values.iter().position(|(n, _)| *n == name)?;
        u8::try_from(index)
            .ok()
            .filter(|&i| usize::from(i) < ObjectParam::PLUGIN_PARAMS)
            .map(|i| Param::Object(ObjectParam::Plugin(i)))
    }
}

/// A built-in parameter (a plugin's needs its object: [`ParamDto::new`]).
impl From<Param> for ParamDto {
    fn from(param: Param) -> Self {
        match param {
            Param::Position(Axis::X) => Self::PositionX,
            Param::Position(Axis::Y) => Self::PositionY,
            Param::Position(Axis::Z) => Self::PositionZ,
            Param::Rotation(Axis::X) => Self::RotationH,
            Param::Rotation(Axis::Y) => Self::RotationP,
            Param::Rotation(Axis::Z) => Self::RotationB,
            Param::Scale(Axis::X) => Self::ScaleX,
            Param::Scale(Axis::Y) => Self::ScaleY,
            Param::Scale(Axis::Z) => Self::ScaleZ,
            Param::Object(ObjectParam::Size(Axis::X)) => Self::SizeX,
            Param::Object(ObjectParam::Size(Axis::Y)) => Self::SizeY,
            Param::Object(ObjectParam::Size(Axis::Z)) => Self::SizeZ,
            Param::Object(ObjectParam::Radius) => Self::Radius,
            Param::Object(ObjectParam::Height) => Self::Height,
            Param::Object(ObjectParam::Width) => Self::Width,
            Param::Object(ObjectParam::TopRadius) => Self::TopRadius,
            Param::Object(ObjectParam::BottomRadius) => Self::BottomRadius,
            Param::Object(ObjectParam::InnerRadius) => Self::InnerRadius,
            Param::Object(ObjectParam::OuterRadius) => Self::OuterRadius,
            Param::Object(ObjectParam::RingRadius) => Self::RingRadius,
            Param::Object(ObjectParam::PipeRadius) => Self::PipeRadius,
            Param::Object(ObjectParam::EllipseRadius) => Self::EllipseRadius,
            Param::Object(ObjectParam::RoundingRadius) => Self::RoundingRadius,
            Param::Object(ObjectParam::StartRadius) => Self::StartRadius,
            Param::Object(ObjectParam::EndRadius) => Self::EndRadius,
            Param::Object(ObjectParam::StartAngle) => Self::StartAngle,
            Param::Object(ObjectParam::EndAngle) => Self::EndAngle,
            Param::Object(ObjectParam::Twist) => Self::Twist,
            Param::Object(ObjectParam::RadialBias) => Self::RadialBias,
            Param::Object(ObjectParam::HeightBias) => Self::HeightBias,
            Param::Object(ObjectParam::HorizontalSpacing) => Self::HorizontalSpacing,
            Param::Object(ObjectParam::VerticalSpacing) => Self::VerticalSpacing,
            Param::Object(ObjectParam::FieldOfView) => Self::FieldOfView,
            Param::Object(ObjectParam::Intensity) => Self::Intensity,
            Param::Object(ObjectParam::InnerAngle) => Self::InnerAngle,
            Param::Object(ObjectParam::OuterAngle) => Self::OuterAngle,
            Param::Object(ObjectParam::FalloffRadius) => Self::FalloffRadius,
            Param::Object(ObjectParam::Offset) => Self::Offset,
            Param::Object(ObjectParam::Movement(Axis::X)) => Self::MovementX,
            Param::Object(ObjectParam::Movement(Axis::Y)) => Self::MovementY,
            Param::Object(ObjectParam::Movement(Axis::Z)) => Self::MovementZ,
            Param::Object(ObjectParam::Angle) => Self::Angle,
            Param::Object(ObjectParam::Rise) => Self::Rise,
            Param::Object(ObjectParam::Scaling) => Self::Scaling,
            Param::Object(ObjectParam::EndScale) => Self::EndScale,
            Param::Object(ObjectParam::EndRotation) => Self::EndRotation,
            Param::Object(ObjectParam::StartGrowth) => Self::StartGrowth,
            Param::Object(ObjectParam::EndGrowth) => Self::EndGrowth,
            Param::Object(ObjectParam::Strength) => Self::Strength,
            Param::Object(ObjectParam::Amount) => Self::Amount,
            Param::Object(ObjectParam::Curvature) => Self::Curvature,
            // Named by `ParamDto::new`, which has the object.
            Param::Object(ObjectParam::Plugin(_)) => Self::Plugin(String::new()),
        }
    }
}

/// A built-in parameter (a plugin's needs its object: [`ParamDto::into_param`]).
impl From<ParamDto> for Param {
    fn from(param: ParamDto) -> Self {
        match param {
            ParamDto::PositionX => Self::Position(Axis::X),
            ParamDto::PositionY => Self::Position(Axis::Y),
            ParamDto::PositionZ => Self::Position(Axis::Z),
            ParamDto::RotationH => Self::Rotation(Axis::X),
            ParamDto::RotationP => Self::Rotation(Axis::Y),
            ParamDto::RotationB => Self::Rotation(Axis::Z),
            ParamDto::ScaleX => Self::Scale(Axis::X),
            ParamDto::ScaleY => Self::Scale(Axis::Y),
            ParamDto::ScaleZ => Self::Scale(Axis::Z),
            ParamDto::SizeX => Self::Object(ObjectParam::Size(Axis::X)),
            ParamDto::SizeY => Self::Object(ObjectParam::Size(Axis::Y)),
            ParamDto::SizeZ => Self::Object(ObjectParam::Size(Axis::Z)),
            ParamDto::Radius => Self::Object(ObjectParam::Radius),
            ParamDto::Height => Self::Object(ObjectParam::Height),
            ParamDto::Width => Self::Object(ObjectParam::Width),
            ParamDto::TopRadius => Self::Object(ObjectParam::TopRadius),
            ParamDto::BottomRadius => Self::Object(ObjectParam::BottomRadius),
            ParamDto::InnerRadius => Self::Object(ObjectParam::InnerRadius),
            ParamDto::OuterRadius => Self::Object(ObjectParam::OuterRadius),
            ParamDto::RingRadius => Self::Object(ObjectParam::RingRadius),
            ParamDto::PipeRadius => Self::Object(ObjectParam::PipeRadius),
            ParamDto::EllipseRadius => Self::Object(ObjectParam::EllipseRadius),
            ParamDto::RoundingRadius => Self::Object(ObjectParam::RoundingRadius),
            ParamDto::StartRadius => Self::Object(ObjectParam::StartRadius),
            ParamDto::EndRadius => Self::Object(ObjectParam::EndRadius),
            ParamDto::StartAngle => Self::Object(ObjectParam::StartAngle),
            ParamDto::EndAngle => Self::Object(ObjectParam::EndAngle),
            ParamDto::Twist => Self::Object(ObjectParam::Twist),
            ParamDto::RadialBias => Self::Object(ObjectParam::RadialBias),
            ParamDto::HeightBias => Self::Object(ObjectParam::HeightBias),
            ParamDto::HorizontalSpacing => Self::Object(ObjectParam::HorizontalSpacing),
            ParamDto::VerticalSpacing => Self::Object(ObjectParam::VerticalSpacing),
            ParamDto::FieldOfView => Self::Object(ObjectParam::FieldOfView),
            ParamDto::Intensity => Self::Object(ObjectParam::Intensity),
            ParamDto::InnerAngle => Self::Object(ObjectParam::InnerAngle),
            ParamDto::OuterAngle => Self::Object(ObjectParam::OuterAngle),
            ParamDto::FalloffRadius => Self::Object(ObjectParam::FalloffRadius),
            ParamDto::Offset => Self::Object(ObjectParam::Offset),
            ParamDto::MovementX => Self::Object(ObjectParam::Movement(Axis::X)),
            ParamDto::MovementY => Self::Object(ObjectParam::Movement(Axis::Y)),
            ParamDto::MovementZ => Self::Object(ObjectParam::Movement(Axis::Z)),
            ParamDto::Angle => Self::Object(ObjectParam::Angle),
            ParamDto::Rise => Self::Object(ObjectParam::Rise),
            ParamDto::Scaling => Self::Object(ObjectParam::Scaling),
            ParamDto::EndScale => Self::Object(ObjectParam::EndScale),
            ParamDto::EndRotation => Self::Object(ObjectParam::EndRotation),
            ParamDto::StartGrowth => Self::Object(ObjectParam::StartGrowth),
            ParamDto::EndGrowth => Self::Object(ObjectParam::EndGrowth),
            ParamDto::Strength => Self::Object(ObjectParam::Strength),
            ParamDto::Amount => Self::Object(ObjectParam::Amount),
            ParamDto::Curvature => Self::Object(ObjectParam::Curvature),
            // Resolved by `ParamDto::into_param`; never reached from there.
            ParamDto::Plugin(_) => Self::Object(ObjectParam::Plugin(u8::MAX)),
        }
    }
}

/// A key; Spline interpolation and Auto tangents (the defaults) are left out.
#[derive(Debug, Serialize, Deserialize)]
struct KeyDto {
    frame: f64,
    value: f32,
    #[serde(default, skip_serializing_if = "InterpolationDto::is_spline")]
    interpolation: InterpolationDto,
    /// Manual tangents as (frames, value change); absent for Auto.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tangents: Option<TangentsDto>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum InterpolationDto {
    #[default]
    Spline,
    Linear,
    Step,
}

impl InterpolationDto {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )]
    fn is_spline(&self) -> bool {
        *self == Self::Spline
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct TangentsDto {
    left: (f64, f32),
    right: (f64, f32),
}

impl TrackDto {
    fn new(track: &Track, kind: &ObjectKind) -> Self {
        Self {
            param: ParamDto::new(track.param, kind),
            keys: track
                .curve
                .keys()
                .iter()
                .map(|key| KeyDto {
                    frame: key.frame,
                    value: key.value,
                    interpolation: match key.interpolation {
                        Interpolation::Spline => InterpolationDto::Spline,
                        Interpolation::Linear => InterpolationDto::Linear,
                        Interpolation::Step => InterpolationDto::Step,
                    },
                    tangents: match key.tangents {
                        Tangents::Auto => None,
                        Tangents::Manual { left, right } => Some(TangentsDto { left, right }),
                    },
                })
                .collect(),
        }
    }

    /// The track of an object of `kind`; `None` if it names a plugin value
    /// the object does not have.
    fn into_track(self, kind: &ObjectKind) -> Option<Track> {
        Some(Track {
            param: self.param.into_param(kind)?,
            curve: Curve::from_keys(self.keys.into_iter().map(|key| Key {
                frame: key.frame,
                value: key.value,
                interpolation: match key.interpolation {
                    InterpolationDto::Spline => Interpolation::Spline,
                    InterpolationDto::Linear => Interpolation::Linear,
                    InterpolationDto::Step => Interpolation::Step,
                },
                tangents: key.tangents.map_or(Tangents::Auto, |t| Tangents::Manual {
                    left: (t.left.0.max(0.0), t.left.1),
                    right: (t.right.0.max(0.0), t.right.1),
                }),
            })),
        })
    }

    /// `true` if every frame, value and tangent is a finite number.
    fn is_finite(&self) -> bool {
        self.keys.iter().all(|k| {
            k.frame.is_finite()
                && k.value.is_finite()
                && k.tangents.as_ref().is_none_or(|t| {
                    t.left.0.is_finite() && t.left.1.is_finite() && t.right.0.is_finite() && t.right.1.is_finite()
                })
        })
    }
}

/// A tag; objects it refers to are given by their index in `objects`.
/// A Dynamics tag's body (added in step 11.1); defaults omitted.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct BodyDto {
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    enabled: bool,
    motion: MotionDto,
    #[serde(default, skip_serializing_if = "is_false")]
    compound: bool,
    #[serde(default, skip_serializing_if = "ShapeDto::is_automatic")]
    shape: ShapeDto,
    bounce: f32,
    friction: f32,
    #[serde(default, skip_serializing_if = "MassModeDto::is_global")]
    mass_mode: MassModeDto,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    density: f32,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    mass: f32,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    rotational_mass: f32,
    #[serde(default, skip_serializing_if = "is_zero")]
    linear_damping: f32,
    #[serde(default, skip_serializing_if = "is_zero")]
    angular_damping: f32,
    /// cm/s.
    #[serde(default, skip_serializing_if = "is_zero_vector")]
    initial_velocity: [f32; 3],
    /// Radians/s.
    #[serde(default, skip_serializing_if = "is_zero_vector")]
    initial_spin: [f32; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MotionDto {
    Dynamic,
    Collider,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ShapeDto {
    #[default]
    Automatic,
    Box,
    Sphere,
    ConvexHull,
    StaticMesh,
}

impl ShapeDto {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )]
    fn is_automatic(&self) -> bool {
        *self == Self::Automatic
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MassModeDto {
    #[default]
    GlobalDensity,
    CustomDensity,
    CustomMass,
}

impl MassModeDto {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )]
    fn is_global(&self) -> bool {
        *self == Self::GlobalDensity
    }
}

impl BodyDto {
    fn new(body: &Body) -> Self {
        Self {
            enabled: body.enabled,
            motion: match body.motion {
                Motion::Dynamic => MotionDto::Dynamic,
                Motion::Collider => MotionDto::Collider,
            },
            compound: body.inherit == Inherit::Compound,
            shape: match body.shape {
                CollisionShape::Automatic => ShapeDto::Automatic,
                CollisionShape::Box => ShapeDto::Box,
                CollisionShape::Sphere => ShapeDto::Sphere,
                CollisionShape::ConvexHull => ShapeDto::ConvexHull,
                CollisionShape::StaticMesh => ShapeDto::StaticMesh,
            },
            bounce: body.bounce,
            friction: body.friction,
            mass_mode: match body.mass_mode {
                MassMode::GlobalDensity => MassModeDto::GlobalDensity,
                MassMode::CustomDensity => MassModeDto::CustomDensity,
                MassMode::CustomMass => MassModeDto::CustomMass,
            },
            density: body.density,
            mass: body.mass,
            rotational_mass: body.rotational_mass,
            linear_damping: body.linear_damping,
            angular_damping: body.angular_damping,
            initial_velocity: body.initial_velocity.to_array(),
            initial_spin: body.initial_spin.to_array(),
        }
    }

    fn into_body(self) -> Body {
        Body {
            enabled: self.enabled,
            motion: match self.motion {
                MotionDto::Dynamic => Motion::Dynamic,
                MotionDto::Collider => Motion::Collider,
            },
            inherit: if self.compound {
                Inherit::Compound
            } else {
                Inherit::None
            },
            shape: match self.shape {
                ShapeDto::Automatic => CollisionShape::Automatic,
                ShapeDto::Box => CollisionShape::Box,
                ShapeDto::Sphere => CollisionShape::Sphere,
                ShapeDto::ConvexHull => CollisionShape::ConvexHull,
                ShapeDto::StaticMesh => CollisionShape::StaticMesh,
            },
            bounce: self.bounce,
            friction: self.friction,
            mass_mode: match self.mass_mode {
                MassModeDto::GlobalDensity => MassMode::GlobalDensity,
                MassModeDto::CustomDensity => MassMode::CustomDensity,
                MassModeDto::CustomMass => MassMode::CustomMass,
            },
            density: self.density,
            mass: self.mass,
            rotational_mass: self.rotational_mass,
            linear_damping: self.linear_damping,
            angular_damping: self.angular_damping,
            initial_velocity: Vec3::from_array(self.initial_velocity),
            initial_spin: Vec3::from_array(self.initial_spin),
        }
        .validated()
    }

    fn parameters(&self) -> Vec<f32> {
        let mut numbers = vec![
            self.bounce,
            self.friction,
            self.density,
            self.mass,
            self.rotational_mass,
            self.linear_damping,
            self.angular_damping,
        ];
        numbers.extend(self.initial_velocity);
        numbers.extend(self.initial_spin);
        numbers
    }
}

/// The document's simulation settings (added in step 11.1); absent while they
/// are the defaults. Gravity in cm/s².
#[derive(Debug, Serialize, Deserialize)]
struct SimulationDto {
    enabled: bool,
    gravity: [f32; 3],
    steps_per_frame: u32,
    solver_iterations: u32,
    time_scale: f32,
    /// Added in step 11.5.
    #[serde(default = "default_memory")]
    memory_limit_mb: u32,
}

const fn default_memory() -> u32 {
    256
}

/// A saved simulation cache (step 11.5): its bodies here, the poses in
/// [`SIMULATION_ENTRY`] (frames × bodies × position and rotation, seven
/// little-endian `f32`s each).
#[derive(Debug, Serialize, Deserialize)]
struct SimCacheDto {
    start: i32,
    frames: u32,
    bodies: Vec<CachedBodyDto>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedBodyDto {
    /// Index in `objects`.
    object: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    clone: Option<u32>,
    scale: [f32; 3],
}

/// The archive entry holding a saved cache's poses.
const SIMULATION_ENTRY: &str = "simulation.bin";
/// The largest saved cache read (bytes): what a file claims is never trusted.
const MAX_CACHE_BYTES: u64 = 1 << 31;

impl SimCacheDto {
    /// The cache's description and its poses' bytes; `None` when a body's
    /// object is not in the file (the cache is then left out).
    fn new(cache: &SimulationCache, refs: &Refs) -> Option<(Self, Vec<u8>)> {
        let bodies = cache
            .bodies
            .iter()
            .map(|b| {
                Some(CachedBodyDto {
                    object: *refs.objects.get(&b.node)?,
                    clone: b.clone,
                    scale: b.scale.to_array(),
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let mut bytes = Vec::with_capacity(cache.poses.len() * 28);
        for (position, rotation) in &cache.poses {
            for value in position.to_array().into_iter().chain(rotation.to_array()) {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        let frames = u32::try_from(cache.frames()).ok()?;
        Some((
            Self {
                start: cache.start,
                frames,
                bodies,
            },
            bytes,
        ))
    }

    fn into_cache(self, archive: &mut Archive<'_>, ids: &[NodeId]) -> Result<SimulationCache, IoError> {
        let invalid = |what: &str| IoError::Invalid(format!("the saved simulation {what}"));
        let expected = u64::from(self.frames)
            .checked_mul(u64::try_from(self.bodies.len()).unwrap_or(u64::MAX))
            .and_then(|n| n.checked_mul(28))
            .filter(|&n| n <= MAX_CACHE_BYTES)
            .ok_or_else(|| invalid("is too large"))?;
        let bytes = archive.read(SIMULATION_ENTRY, expected)?;
        if u64::try_from(bytes.len()).ok() != Some(expected) {
            return Err(invalid("has the wrong size"));
        }
        let bodies = self
            .bodies
            .iter()
            .map(|b| {
                let scale = Vec3::from_array(b.scale);
                (scale.is_finite())
                    .then_some(())
                    .and_then(|()| ids.get(b.object))
                    .map(|&node| CachedBody {
                        node,
                        clone: b.clone,
                        scale,
                    })
                    .ok_or_else(|| invalid("names an object that is not in the file"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let values: Vec<f32> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        if !values.iter().all(|v| v.is_finite()) {
            return Err(invalid("holds a number that is not finite"));
        }
        let poses = values
            .as_chunks::<7>()
            .0
            .iter()
            .map(|v| (Vec3::new(v[0], v[1], v[2]), unit_rotation(v[3], v[4], v[5], v[6])))
            .collect();
        Ok(SimulationCache {
            start: self.start,
            bodies,
            poses,
        })
    }
}

/// A saved rotation as a unit quaternion: kept bit for bit when it is one
/// already (so a file saves back unchanged), normalised when it is off, and no
/// rotation when it has no length (normalising that would give NaNs).
fn unit_rotation(x: f32, y: f32, z: f32, w: f32) -> glam::Quat {
    let q = glam::Quat::from_xyzw(x, y, z, w);
    let length = q.length_squared();
    if !(length > 1e-12 && length.is_finite()) {
        glam::Quat::IDENTITY
    } else if (length - 1.0).abs() > 1e-5 {
        q.normalize()
    } else {
        q
    }
}

impl SimulationDto {
    /// `None` for the defaults, which files leave out.
    fn new(settings: &SimulationSettings) -> Option<Self> {
        (*settings != SimulationSettings::default()).then(|| Self {
            enabled: settings.enabled,
            gravity: settings.gravity.to_array(),
            steps_per_frame: settings.steps_per_frame,
            solver_iterations: settings.solver_iterations,
            time_scale: settings.time_scale,
            memory_limit_mb: settings.memory_limit_mb,
        })
    }

    fn into_settings(self) -> Result<SimulationSettings, IoError> {
        if !self.gravity.iter().chain([&self.time_scale]).all(|v| v.is_finite()) {
            return Err(IoError::Invalid(
                "the simulation settings hold a number that is not finite".to_owned(),
            ));
        }
        Ok(SimulationSettings {
            enabled: self.enabled,
            gravity: Vec3::from_array(self.gravity),
            steps_per_frame: self.steps_per_frame,
            solver_iterations: self.solver_iterations,
            time_scale: self.time_scale,
            memory_limit_mb: self.memory_limit_mb,
        }
        .validated())
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TagDto {
    Phong {
        angle_limit: bool,
        /// Radians.
        angle: f32,
    },
    Protection {
        position: bool,
        scale: bool,
        rotation: bool,
    },
    Target {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        up: Option<usize>,
        pitch: bool,
    },
    /// Added with materials (step 6.6): the material's index in `materials`;
    /// the projection's fields were added in version 4 (step 15.4), each left
    /// out while it has its default.
    Material {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        material: Option<usize>,
        #[serde(default, skip_serializing_if = "ProjectionDto::is_uv")]
        projection: ProjectionDto,
        #[serde(default, skip_serializing_if = "is_zero_pair")]
        offset: [f32; 2],
        #[serde(default = "unit_pair", skip_serializing_if = "is_unit_pair")]
        tiling: [f32; 2],
        /// Radians.
        #[serde(default, skip_serializing_if = "is_zero")]
        rotation: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        placement: Option<PlacementDto>,
        /// Camera projection: the camera object's index.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        camera: Option<usize>,
    },
    /// A rigid or collider body (step 11.1).
    Dynamics(BodyDto),
}

/// How a Material tag projects (version 4).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProjectionDto {
    #[default]
    Uv,
    Box,
    Spherical,
    Cylindrical,
    Flat,
    Camera,
}

impl ProjectionDto {
    #[allow(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )]
    fn is_uv(&self) -> bool {
        *self == Self::Uv
    }
}

impl From<Projection> for ProjectionDto {
    fn from(projection: Projection) -> Self {
        match projection {
            Projection::Uv => Self::Uv,
            Projection::Box => Self::Box,
            Projection::Spherical => Self::Spherical,
            Projection::Cylindrical => Self::Cylindrical,
            Projection::Flat => Self::Flat,
            Projection::Camera => Self::Camera,
        }
    }
}

impl From<ProjectionDto> for Projection {
    fn from(projection: ProjectionDto) -> Self {
        match projection {
            ProjectionDto::Uv => Self::Uv,
            ProjectionDto::Box => Self::Box,
            ProjectionDto::Spherical => Self::Spherical,
            ProjectionDto::Cylindrical => Self::Cylindrical,
            ProjectionDto::Flat => Self::Flat,
            ProjectionDto::Camera => Self::Camera,
        }
    }
}

/// Where a projection's shape sits in the object's space: centre, turn
/// (heading, pitch, bank in radians), full size.
#[derive(Debug, Serialize, Deserialize)]
struct PlacementDto {
    position: [f32; 3],
    rotation: [f32; 3],
    size: [f32; 3],
}

impl TagDto {
    fn new(tag: &Tag, refs: &Refs) -> Self {
        let index = |id: Option<NodeId>| id.and_then(|id| refs.objects.get(&id).copied());
        match *tag {
            Tag::Phong { angle_limit, angle } => Self::Phong { angle_limit, angle },
            Tag::Protection {
                position,
                scale,
                rotation,
            } => Self::Protection {
                position,
                scale,
                rotation,
            },
            Tag::Target { target, up, pitch } => Self::Target {
                target: index(target),
                up: index(up),
                pitch,
            },
            Tag::Material(ref tag) => Self::Material {
                material: tag.material.and_then(|m| refs.materials.get(&m).copied()),
                projection: tag.projection.into(),
                offset: tag.offset,
                tiling: tag.tiling,
                rotation: tag.rotation,
                placement: tag.placement.map(|p| PlacementDto {
                    position: p.position.to_array(),
                    rotation: p.rotation.to_array(),
                    size: p.size.to_array(),
                }),
                camera: index(tag.camera),
            },
            Tag::Dynamics(ref body) => Self::Dynamics(BodyDto::new(body)),
        }
    }

    /// The tag, with object references resolved through `ids` and material
    /// references through `materials` (an index that names nothing drops the
    /// reference).
    fn into_tag(self, ids: &[NodeId], materials: &[MaterialId]) -> Tag {
        let id = |index: Option<usize>| index.and_then(|i| ids.get(i).copied());
        match self {
            Self::Phong { angle_limit, angle } => Tag::Phong {
                angle_limit,
                angle: angle.clamp(0.0, std::f32::consts::PI),
            },
            Self::Protection {
                position,
                scale,
                rotation,
            } => Tag::Protection {
                position,
                scale,
                rotation,
            },
            Self::Target { target, up, pitch } => Tag::Target {
                target: id(target),
                up: id(up),
                pitch,
            },
            Self::Material {
                material,
                projection,
                offset,
                tiling,
                rotation,
                placement,
                camera,
            } => Tag::Material(
                MaterialTag {
                    material: material.and_then(|i| materials.get(i).copied()),
                    projection: projection.into(),
                    offset,
                    tiling,
                    rotation,
                    placement: placement.map(|p| Placement {
                        position: Vec3::from_array(p.position),
                        rotation: Vec3::from_array(p.rotation),
                        size: Vec3::from_array(p.size),
                    }),
                    camera: id(camera),
                }
                .validated(),
            ),
            Self::Dynamics(body) => Tag::Dynamics(body.into_body()),
        }
    }

    fn parameters(&self) -> Vec<f32> {
        match self {
            Self::Phong { angle, .. } => vec![*angle],
            Self::Protection { .. } | Self::Target { .. } => Vec::new(),
            Self::Material {
                offset,
                tiling,
                rotation,
                placement,
                ..
            } => {
                let mut numbers = vec![offset[0], offset[1], tiling[0], tiling[1], *rotation];
                if let Some(p) = placement {
                    numbers.extend(p.position.iter().chain(&p.rotation).chain(&p.size));
                }
                numbers
            }
            Self::Dynamics(body) => body.parameters(),
        }
    }
}

/// One value of a plugin object: `{"name": "Teeth", "int": 12}`.
#[derive(Debug, Serialize, Deserialize)]
struct PluginValueDto {
    name: String,
    #[serde(flatten)]
    value: PluginValueKindDto,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PluginValueKindDto {
    Float(f32),
    Int(i32),
    Bool(bool),
    Choice(u32),
    Vector([f32; 3]),
    /// Linear RGB.
    Color([f32; 3]),
}

impl From<PluginValue> for PluginValueKindDto {
    fn from(value: PluginValue) -> Self {
        match value {
            PluginValue::Float(v) => Self::Float(v),
            PluginValue::Int(v) => Self::Int(v),
            PluginValue::Bool(v) => Self::Bool(v),
            PluginValue::Choice(v) => Self::Choice(v),
            PluginValue::Vector(v) => Self::Vector(v.to_array()),
            PluginValue::Color(c) => Self::Color(c),
        }
    }
}

impl From<PluginValueKindDto> for PluginValue {
    fn from(value: PluginValueKindDto) -> Self {
        match value {
            PluginValueKindDto::Float(v) => Self::Float(v),
            PluginValueKindDto::Int(v) => Self::Int(v),
            PluginValueKindDto::Bool(v) => Self::Bool(v),
            PluginValueKindDto::Choice(v) => Self::Choice(v),
            PluginValueKindDto::Vector(v) => Self::Vector(Vec3::from_array(v)),
            PluginValueKindDto::Color(c) => Self::Color(c),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum KindDto {
    Null,
    Cube {
        size: [f32; 3],
        segments: [u32; 3],
    },
    Sphere {
        radius: f32,
        segments: u32,
    },
    Cylinder {
        radius: f32,
        height: f32,
        rotation_segments: u32,
        height_segments: u32,
    },
    Plane {
        width: f32,
        depth: f32,
        width_segments: u32,
        depth_segments: u32,
    },
    Cone {
        bottom_radius: f32,
        top_radius: f32,
        height: f32,
        rotation_segments: u32,
        height_segments: u32,
    },
    Torus {
        ring_radius: f32,
        pipe_radius: f32,
        ring_segments: u32,
        pipe_segments: u32,
    },
    Disc {
        inner_radius: f32,
        outer_radius: f32,
        rotation_segments: u32,
        disc_segments: u32,
    },
    Capsule {
        radius: f32,
        height: f32,
        rotation_segments: u32,
        height_segments: u32,
        cap_segments: u32,
    },
    Camera {
        fov_y_deg: f32,
    },
    Light {
        color: [f32; 3],
        intensity: f32,
        /// Omni when absent (added in step 7.4).
        #[serde(default, skip_serializing_if = "LightTypeDto::is_omni")]
        light_type: LightTypeDto,
        /// The defaults when absent (added in step 7.4).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        details: Option<LightDetailsDto>,
        /// None when absent (added in step 7.6).
        #[serde(default, skip_serializing_if = "LightShadowDto::is_none")]
        shadow: LightShadowDto,
    },
    /// `mesh` indexes the file's `meshes`; objects sharing a mesh share an index.
    /// `selected` / `hidden` flag components (optional, added in step 4.1);
    /// `seams` the UV seams, as `edges` ranges (version 4, step 15.7b).
    Polygon {
        mesh: usize,
        #[serde(default, skip_serializing_if = "SetDto::is_empty")]
        selected: SetDto,
        #[serde(default, skip_serializing_if = "SetDto::is_empty")]
        hidden: SetDto,
        #[serde(default, skip_serializing_if = "SetDto::is_empty")]
        seams: SetDto,
    },
    /// Added to v3 before its release (additive): files without it still load.
    SubdivisionSurface {
        viewport_levels: u32,
        render_levels: u32,
        linear_pre: u32,
        boundary: BoundaryDto,
    },
    /// An Extrude generator (added in step 9.4); defaults omitted.
    Extrude {
        #[serde(default, skip_serializing_if = "ExtrudeDirectionDto::is_auto")]
        direction: ExtrudeDirectionDto,
        offset: f32,
        movement: [f32; 3],
        subdivision: u32,
        #[serde(default, skip_serializing_if = "is_false")]
        reverse_normals: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        hierarchical: bool,
        #[serde(default = "yes", skip_serializing_if = "is_true")]
        start_cap: bool,
        #[serde(default = "yes", skip_serializing_if = "is_true")]
        end_cap: bool,
        #[serde(default, skip_serializing_if = "CapTypeDto::is_ngons")]
        cap_type: CapTypeDto,
    },
    /// A Lathe generator (added in step 9.5), the angle in radians; defaults
    /// omitted.
    Lathe {
        angle: f32,
        subdivision: u32,
        #[serde(default, skip_serializing_if = "is_zero")]
        movement: f32,
        #[serde(default = "one", skip_serializing_if = "is_one")]
        scaling: f32,
        #[serde(default, skip_serializing_if = "is_false")]
        reverse_normals: bool,
        #[serde(default = "yes", skip_serializing_if = "is_true")]
        start_cap: bool,
        #[serde(default = "yes", skip_serializing_if = "is_true")]
        end_cap: bool,
        #[serde(default, skip_serializing_if = "CapTypeDto::is_ngons")]
        cap_type: CapTypeDto,
    },
    /// A Sweep generator (added in step 9.6), the rotation in radians;
    /// defaults omitted.
    Sweep {
        #[serde(default = "one", skip_serializing_if = "is_one")]
        end_scale: f32,
        #[serde(default, skip_serializing_if = "is_zero")]
        end_rotation: f32,
        #[serde(default, skip_serializing_if = "is_zero")]
        start_growth: f32,
        #[serde(default = "one", skip_serializing_if = "is_one")]
        end_growth: f32,
        #[serde(default, skip_serializing_if = "is_false")]
        parallel_movement: bool,
        #[serde(default = "yes", skip_serializing_if = "is_true")]
        constant_cross_section: bool,
        #[serde(default = "yes", skip_serializing_if = "is_true")]
        banking: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        reverse_normals: bool,
        #[serde(default = "yes", skip_serializing_if = "is_true")]
        start_cap: bool,
        #[serde(default = "yes", skip_serializing_if = "is_true")]
        end_cap: bool,
        #[serde(default, skip_serializing_if = "CapTypeDto::is_ngons")]
        cap_type: CapTypeDto,
    },
    /// A Loft generator (added in step 9.7); defaults omitted.
    Loft {
        subdivision_u: u32,
        subdivision_v: u32,
        #[serde(default, skip_serializing_if = "is_false")]
        organic: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        per_segment: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        looped: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        linear: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        reverse_normals: bool,
        #[serde(default = "yes", skip_serializing_if = "is_true")]
        start_cap: bool,
        #[serde(default = "yes", skip_serializing_if = "is_true")]
        end_cap: bool,
        #[serde(default, skip_serializing_if = "CapTypeDto::is_ngons")]
        cap_type: CapTypeDto,
    },
    /// A Cloner (added in step 10.6); angles in radians, defaults omitted.
    Cloner {
        layout: ClonerLayoutDto,
        /// Object mode's object, by index in `objects` (added in step 10.7;
        /// written by the object, resolved once every object exists).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        object: Option<usize>,
        /// Its effectors, by index in `objects` (added in step 10.8; resolved
        /// like `object`).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        effectors: Vec<usize>,
        #[serde(default, skip_serializing_if = "CloneOrderDto::is_iterate")]
        clones: CloneOrderDto,
        seed: u32,
        #[serde(default = "yes", skip_serializing_if = "is_true")]
        reset_coordinates: bool,
    },
    /// A Plain effector (added in step 10.8).
    PlainEffector(EffectorDto),
    /// A Random effector (added in step 10.8).
    RandomEffector(EffectorDto),
    /// A Bend deformer (added in step 10.2); angles in radians, defaults omitted.
    Bend {
        size: [f32; 3],
        #[serde(default, skip_serializing_if = "DeformerModeDto::is_limited")]
        mode: DeformerModeDto,
        strength: f32,
        #[serde(default, skip_serializing_if = "is_zero")]
        angle: f32,
        #[serde(default, skip_serializing_if = "is_false")]
        keep_length: bool,
        #[serde(default, skip_serializing_if = "BendOriginDto::is_bottom")]
        origin: BendOriginDto,
    },
    /// A Twist deformer (added in step 10.3); the angle in radians.
    Twist {
        size: [f32; 3],
        #[serde(default, skip_serializing_if = "DeformerModeDto::is_limited")]
        mode: DeformerModeDto,
        angle: f32,
    },
    /// A Taper deformer (added in step 10.3); strength and curvature as fractions.
    Taper {
        size: [f32; 3],
        #[serde(default, skip_serializing_if = "DeformerModeDto::is_limited")]
        mode: DeformerModeDto,
        strength: f32,
        #[serde(default, skip_serializing_if = "is_zero")]
        curvature: f32,
        #[serde(default, skip_serializing_if = "is_false")]
        fillet: bool,
    },
    /// A Bulge deformer (added in step 10.3); strength and curvature as fractions.
    Bulge {
        size: [f32; 3],
        #[serde(default, skip_serializing_if = "DeformerModeDto::is_limited")]
        mode: DeformerModeDto,
        strength: f32,
        #[serde(default, skip_serializing_if = "is_zero")]
        curvature: f32,
        #[serde(default, skip_serializing_if = "is_false")]
        fillet: bool,
    },
    /// An FFD deformer (added in step 10.4): its lattice points (absent while
    /// regular) and their selected / hidden ranges.
    Ffd {
        size: [f32; 3],
        grid: [u32; 3],
        #[serde(default, skip_serializing_if = "DeformerModeDto::is_limited")]
        mode: DeformerModeDto,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        lattice: Vec<[f32; 3]>,
        #[serde(default, skip_serializing_if = "SetDto::is_empty")]
        selected: SetDto,
        #[serde(default, skip_serializing_if = "SetDto::is_empty")]
        hidden: SetDto,
    },
    /// A spline primitive (added in step 9.1). Angles in radians.
    Spline {
        spline: SplineShapeDto,
        #[serde(default, skip_serializing_if = "SplinePlaneDto::is_xy")]
        plane: SplinePlaneDto,
        #[serde(default, skip_serializing_if = "is_false")]
        reverse: bool,
        /// Adaptive at 5° when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        intermediate_points: Option<SplinePointsDto>,
    },
    /// A Text spline (added in step 9.8): its parameters, not its outline.
    Text {
        text: String,
        height: f32,
        #[serde(default, skip_serializing_if = "TextAlignDto::is_left")]
        align: TextAlignDto,
        #[serde(default, skip_serializing_if = "is_zero")]
        horizontal_spacing: f32,
        #[serde(default, skip_serializing_if = "is_zero")]
        vertical_spacing: f32,
        #[serde(default, skip_serializing_if = "SplinePlaneDto::is_xy")]
        plane: SplinePlaneDto,
        #[serde(default, skip_serializing_if = "is_false")]
        reverse: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        intermediate_points: Option<SplinePointsDto>,
    },
    /// A plugin's object (added in step 13.1): which plugin, and its values
    /// by name — kept as they are when the plugin is missing.
    Plugin {
        plugin: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        values: Vec<PluginValueDto>,
    },
    /// An editable spline (added in step 9.2), its arrays inline.
    EditableSpline {
        spline_type: SplineTypeDto,
        points: Vec<[f32; 3]>,
        /// Per point: incoming and outgoing tangent. Absent when all are zero.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tangents: Vec<[[f32; 3]; 2]>,
        segments: Vec<SegmentDto>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        intermediate_points: Option<SplinePointsDto>,
        /// Selected and hidden points (`points` ranges only).
        #[serde(default, skip_serializing_if = "SetDto::is_empty")]
        selected: SetDto,
        #[serde(default, skip_serializing_if = "SetDto::is_empty")]
        hidden: SetDto,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SplineTypeDto {
    Linear,
    Cubic,
    Akima,
    BSpline,
    Bezier,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct SegmentDto {
    count: u32,
    closed: bool,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_false(value: &bool) -> bool {
    !*value
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_true(value: &bool) -> bool {
    *value
}

const fn yes() -> bool {
    true
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_zero(value: &f32) -> bool {
    *value == 0.0
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_one(value: &f32) -> bool {
    *value == 1.0
}

const fn one() -> f32 {
    1.0
}

impl From<CapType> for CapTypeDto {
    fn from(cap_type: CapType) -> Self {
        match cap_type {
            CapType::NGons => Self::NGons,
            CapType::Triangles => Self::Triangles,
        }
    }
}

impl From<CapTypeDto> for CapType {
    fn from(cap_type: CapTypeDto) -> Self {
        match cap_type {
            CapTypeDto::NGons => Self::NGons,
            CapTypeDto::Triangles => Self::Triangles,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ExtrudeDirectionDto {
    #[default]
    Auto,
    X,
    Y,
    Z,
    Custom,
    Absolute,
}

impl ExtrudeDirectionDto {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )]
    fn is_auto(&self) -> bool {
        *self == Self::Auto
    }
}

/// An effector's settings (added in step 10.8); angles in radians, the
/// scale change as a fraction, defaults omitted.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct EffectorDto {
    strength: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    position: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scale: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "is_false")]
    uniform_scale: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rotation: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    color: Option<[f32; 3]>,
    seed: u32,
    #[serde(default, skip_serializing_if = "EffectorFalloffDto::is_infinite")]
    falloff: EffectorFalloffDto,
}

/// Omitted while it is the default falloff (Infinite, 200 cm, half soft).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct EffectorFalloffDto {
    shape: FalloffShapeDto,
    size: [f32; 3],
    soft: f32,
}

impl Default for EffectorFalloffDto {
    fn default() -> Self {
        Self {
            shape: FalloffShapeDto::Infinite,
            size: [200.0; 3],
            soft: 0.5,
        }
    }
}

impl EffectorFalloffDto {
    fn is_infinite(&self) -> bool {
        self.shape == FalloffShapeDto::Infinite && self.size == [200.0; 3] && self.soft == 0.5
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FalloffShapeDto {
    #[default]
    Infinite,
    Sphere,
    Box,
    Linear,
}

impl EffectorDto {
    fn new(e: &Effector) -> Self {
        Self {
            strength: e.strength,
            position: e.use_position.then(|| e.position.to_array()),
            scale: e.use_scale.then(|| e.scale.to_array()),
            uniform_scale: e.uniform_scale,
            rotation: e.use_rotation.then(|| e.rotation.to_array()),
            color: e.use_color.then_some(e.color),
            seed: e.seed,
            falloff: EffectorFalloffDto {
                shape: match e.falloff.shape {
                    FalloffShape::Infinite => FalloffShapeDto::Infinite,
                    FalloffShape::Sphere => FalloffShapeDto::Sphere,
                    FalloffShape::Box => FalloffShapeDto::Box,
                    FalloffShape::Linear => FalloffShapeDto::Linear,
                },
                size: e.falloff.size.to_array(),
                soft: e.falloff.soft,
            },
        }
    }

    /// The effector, starting from `defaults` for what the file leaves out.
    fn into_effector(self, defaults: Effector) -> Effector {
        Effector {
            strength: self.strength,
            use_position: self.position.is_some(),
            position: self.position.map_or(defaults.position, Vec3::from_array),
            use_scale: self.scale.is_some(),
            scale: self.scale.map_or(defaults.scale, Vec3::from_array),
            uniform_scale: self.uniform_scale,
            use_rotation: self.rotation.is_some(),
            rotation: self.rotation.map_or(defaults.rotation, Vec3::from_array),
            use_color: self.color.is_some(),
            color: self.color.unwrap_or(defaults.color),
            seed: self.seed,
            falloff: EffectorFalloff {
                shape: match self.falloff.shape {
                    FalloffShapeDto::Infinite => FalloffShape::Infinite,
                    FalloffShapeDto::Sphere => FalloffShape::Sphere,
                    FalloffShapeDto::Box => FalloffShape::Box,
                    FalloffShapeDto::Linear => FalloffShape::Linear,
                },
                size: Vec3::from_array(self.falloff.size).abs(),
                soft: self.falloff.soft.clamp(0.0, 1.0),
            },
        }
    }

    fn parameters(&self) -> Vec<f32> {
        let mut numbers = vec![self.strength, self.falloff.soft];
        numbers.extend(self.falloff.size);
        for values in [self.position, self.scale, self.rotation, self.color]
            .into_iter()
            .flatten()
        {
            numbers.extend(values);
        }
        numbers
    }
}

/// A Cloner's layout, by mode.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
enum ClonerLayoutDto {
    Linear {
        count: u32,
        #[serde(default, skip_serializing_if = "is_zero_count")]
        offset: u32,
        #[serde(default, skip_serializing_if = "StepModeDto::is_per_step")]
        step: StepModeDto,
        position: [f32; 3],
        #[serde(default, skip_serializing_if = "is_zero_vector")]
        rotation: [f32; 3],
        scale: [f32; 3],
    },
    Radial {
        count: u32,
        radius: f32,
        plane: SplinePlaneDto,
        align: bool,
        #[serde(default, skip_serializing_if = "is_zero")]
        start_angle: f32,
        end_angle: f32,
        #[serde(default, skip_serializing_if = "is_zero")]
        offset: f32,
    },
    Grid {
        count: [u32; 3],
        #[serde(default, skip_serializing_if = "StepModeDto::is_per_step")]
        step: StepModeDto,
        size: [f32; 3],
    },
    /// Added in step 10.7.
    Object {
        #[serde(default, skip_serializing_if = "DistributionDto::is_vertex")]
        distribution: DistributionDto,
        align: bool,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DistributionDto {
    #[default]
    Vertex,
    PolygonCenter,
}

impl DistributionDto {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )]
    fn is_vertex(&self) -> bool {
        *self == Self::Vertex
    }
}

impl ClonerLayoutDto {
    fn new(layout: Layout) -> Self {
        match layout {
            Layout::Linear(linear) => Self::Linear {
                count: linear.count,
                offset: linear.offset,
                step: linear.mode.into(),
                position: linear.position.to_array(),
                rotation: linear.rotation.to_array(),
                scale: linear.scale.to_array(),
            },
            Layout::Radial(radial) => Self::Radial {
                count: radial.count,
                radius: radial.radius,
                plane: match radial.plane {
                    SplinePlane::XY => SplinePlaneDto::Xy,
                    SplinePlane::ZY => SplinePlaneDto::Zy,
                    SplinePlane::XZ => SplinePlaneDto::Xz,
                },
                align: radial.align,
                start_angle: radial.start_angle,
                end_angle: radial.end_angle,
                offset: radial.offset,
            },
            Layout::Grid(grid) => Self::Grid {
                count: grid.count,
                step: grid.mode.into(),
                size: grid.size.to_array(),
            },
            Layout::Object(surface) => Self::Object {
                distribution: match surface.distribution {
                    Distribution::Vertex => DistributionDto::Vertex,
                    Distribution::PolygonCenter => DistributionDto::PolygonCenter,
                },
                align: surface.align,
            },
        }
    }

    /// Counts a file claims are clamped (never trusted).
    fn into_layout(self) -> Layout {
        let count = |c: u32| c.min(Cloner::MAX_COUNT);
        match self {
            Self::Linear {
                count: n,
                offset,
                step,
                position,
                rotation,
                scale,
            } => Layout::Linear(Linear {
                count: count(n),
                offset: count(offset),
                mode: step.into(),
                position: Vec3::from_array(position),
                rotation: Vec3::from_array(rotation),
                scale: Vec3::from_array(scale),
            }),
            Self::Radial {
                count: n,
                radius,
                plane,
                align,
                start_angle,
                end_angle,
                offset,
            } => Layout::Radial(Radial {
                count: count(n),
                radius,
                plane: match plane {
                    SplinePlaneDto::Xy => SplinePlane::XY,
                    SplinePlaneDto::Zy => SplinePlane::ZY,
                    SplinePlaneDto::Xz => SplinePlane::XZ,
                },
                align,
                start_angle,
                end_angle,
                offset,
            }),
            Self::Grid { count: n, step, size } => Layout::Grid(GridArray {
                count: n.map(|c| count(c).max(1)),
                mode: step.into(),
                size: Vec3::from_array(size),
            }),
            Self::Object { distribution, align } => Layout::Object(Surface {
                distribution: match distribution {
                    DistributionDto::Vertex => Distribution::Vertex,
                    DistributionDto::PolygonCenter => Distribution::PolygonCenter,
                },
                align,
            }),
        }
    }

    fn parameters(&self) -> Vec<f32> {
        match *self {
            Self::Linear {
                position,
                rotation,
                scale,
                ..
            } => [position, rotation, scale].concat(),
            Self::Radial {
                radius,
                start_angle,
                end_angle,
                offset,
                ..
            } => vec![radius, start_angle, end_angle, offset],
            Self::Grid { size, .. } => size.to_vec(),
            Self::Object { .. } => Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StepModeDto {
    #[default]
    PerStep,
    EndPoint,
}

impl StepModeDto {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )]
    fn is_per_step(&self) -> bool {
        *self == Self::PerStep
    }
}

impl From<StepMode> for StepModeDto {
    fn from(mode: StepMode) -> Self {
        match mode {
            StepMode::PerStep => Self::PerStep,
            StepMode::EndPoint => Self::EndPoint,
        }
    }
}

impl From<StepModeDto> for StepMode {
    fn from(mode: StepModeDto) -> Self {
        match mode {
            StepModeDto::PerStep => Self::PerStep,
            StepModeDto::EndPoint => Self::EndPoint,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CloneOrderDto {
    #[default]
    Iterate,
    Random,
}

impl CloneOrderDto {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )]
    fn is_iterate(&self) -> bool {
        *self == Self::Iterate
    }
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
const fn is_zero_count(value: &u32) -> bool {
    *value == 0
}

fn is_zero_vector(value: &[f32; 3]) -> bool {
    value.iter().all(|&v| v == 0.0)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DeformerModeDto {
    #[default]
    Limited,
    WithinBox,
    Unlimited,
}

impl From<DeformerMode> for DeformerModeDto {
    fn from(mode: DeformerMode) -> Self {
        match mode {
            DeformerMode::Limited => Self::Limited,
            DeformerMode::WithinBox => Self::WithinBox,
            DeformerMode::Unlimited => Self::Unlimited,
        }
    }
}

impl From<DeformerModeDto> for DeformerMode {
    fn from(mode: DeformerModeDto) -> Self {
        match mode {
            DeformerModeDto::Limited => Self::Limited,
            DeformerModeDto::WithinBox => Self::WithinBox,
            DeformerModeDto::Unlimited => Self::Unlimited,
        }
    }
}

impl DeformerModeDto {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )]
    fn is_limited(&self) -> bool {
        *self == Self::Limited
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BendOriginDto {
    #[default]
    Bottom,
    CenterC,
    CenterS,
    Top,
}

impl BendOriginDto {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )]
    fn is_bottom(&self) -> bool {
        *self == Self::Bottom
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TextAlignDto {
    #[default]
    Left,
    Center,
    Right,
}

impl TextAlignDto {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )]
    fn is_left(&self) -> bool {
        *self == Self::Left
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CapTypeDto {
    #[default]
    NGons,
    Triangles,
}

impl CapTypeDto {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )]
    fn is_ngons(&self) -> bool {
        *self == Self::NGons
    }
}

/// A spline primitive's shape; optional parts (an ellipse's second radius, a
/// ring, rounding) are absent when off.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
enum SplineShapeDto {
    Circle {
        radius: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ellipse: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ring: Option<f32>,
    },
    Rectangle {
        width: f32,
        height: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rounding: Option<f32>,
    },
    Arc {
        arc_type: ArcTypeDto,
        radius: f32,
        inner_radius: f32,
        start: f32,
        end: f32,
    },
    NSide {
        radius: f32,
        sides: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rounding: Option<f32>,
    },
    Star {
        inner_radius: f32,
        outer_radius: f32,
        twist: f32,
        points: u32,
    },
    Helix {
        start_radius: f32,
        start_angle: f32,
        end_radius: f32,
        end_angle: f32,
        radial_bias: f32,
        height: f32,
        height_bias: f32,
        subdivision: u32,
    },
    Flower {
        inner_radius: f32,
        outer_radius: f32,
        petals: u32,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ArcTypeDto {
    Arc,
    Sector,
    Segment,
    Ring,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SplinePlaneDto {
    #[default]
    Xy,
    Zy,
    Xz,
}

impl SplinePlaneDto {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )]
    fn is_xy(&self) -> bool {
        *self == Self::Xy
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
enum SplinePointsDto {
    None,
    Natural { number: u32 },
    Uniform { number: u32 },
    Adaptive { angle: f32 },
    Subdivided { angle: f32, max_length: f32 },
}

impl SplineShapeDto {
    fn new(shape: SplineShape) -> Self {
        match shape {
            SplineShape::Circle { radius, ellipse, ring } => Self::Circle { radius, ellipse, ring },
            SplineShape::Rectangle {
                width,
                height,
                rounding,
            } => Self::Rectangle {
                width,
                height,
                rounding,
            },
            SplineShape::Arc {
                kind,
                radius,
                inner_radius,
                start,
                end,
            } => Self::Arc {
                arc_type: match kind {
                    ArcType::Arc => ArcTypeDto::Arc,
                    ArcType::Sector => ArcTypeDto::Sector,
                    ArcType::Segment => ArcTypeDto::Segment,
                    ArcType::Ring => ArcTypeDto::Ring,
                },
                radius,
                inner_radius,
                start,
                end,
            },
            SplineShape::NSide {
                radius,
                sides,
                rounding,
            } => Self::NSide {
                radius,
                sides,
                rounding,
            },
            SplineShape::Star {
                inner_radius,
                outer_radius,
                twist,
                points,
            } => Self::Star {
                inner_radius,
                outer_radius,
                twist,
                points,
            },
            SplineShape::Helix {
                start_radius,
                start_angle,
                end_radius,
                end_angle,
                radial_bias,
                height,
                height_bias,
                subdivision,
            } => Self::Helix {
                start_radius,
                start_angle,
                end_radius,
                end_angle,
                radial_bias,
                height,
                height_bias,
                subdivision,
            },
            SplineShape::Flower {
                inner_radius,
                outer_radius,
                petals,
            } => Self::Flower {
                inner_radius,
                outer_radius,
                petals,
            },
        }
    }

    /// The shape, counts kept in their ranges (never trust a count a file claims).
    fn into_shape(self) -> SplineShape {
        let sides = |n: u32| n.clamp(3, SplineShape::MAX_SIDES);
        match self {
            Self::Circle { radius, ellipse, ring } => SplineShape::Circle { radius, ellipse, ring },
            Self::Rectangle {
                width,
                height,
                rounding,
            } => SplineShape::Rectangle {
                width,
                height,
                rounding,
            },
            Self::Arc {
                arc_type,
                radius,
                inner_radius,
                start,
                end,
            } => SplineShape::Arc {
                kind: match arc_type {
                    ArcTypeDto::Arc => ArcType::Arc,
                    ArcTypeDto::Sector => ArcType::Sector,
                    ArcTypeDto::Segment => ArcType::Segment,
                    ArcTypeDto::Ring => ArcType::Ring,
                },
                radius,
                inner_radius,
                start,
                end,
            },
            Self::NSide {
                radius,
                sides: n,
                rounding,
            } => SplineShape::NSide {
                radius,
                sides: sides(n),
                rounding,
            },
            Self::Star {
                inner_radius,
                outer_radius,
                twist,
                points,
            } => SplineShape::Star {
                inner_radius,
                outer_radius,
                twist,
                points: sides(points),
            },
            Self::Helix {
                start_radius,
                start_angle,
                end_radius,
                end_angle,
                radial_bias,
                height,
                height_bias,
                subdivision,
            } => SplineShape::Helix {
                start_radius,
                start_angle,
                end_radius,
                end_angle,
                radial_bias: radial_bias.clamp(0.0, 1.0),
                height,
                height_bias: height_bias.clamp(0.0, 1.0),
                subdivision: subdivision.clamp(3, SplineShape::MAX_SUBDIVISION),
            },
            Self::Flower {
                inner_radius,
                outer_radius,
                petals,
            } => SplineShape::Flower {
                inner_radius,
                outer_radius,
                petals: petals.clamp(1, SplineShape::MAX_SIDES),
            },
        }
    }

    fn parameters(self) -> Vec<f32> {
        match self {
            Self::Circle { radius, ellipse, ring } => [Some(radius), ellipse, ring].into_iter().flatten().collect(),
            Self::Rectangle {
                width,
                height,
                rounding,
            } => [Some(width), Some(height), rounding].into_iter().flatten().collect(),
            Self::Arc {
                radius,
                inner_radius,
                start,
                end,
                ..
            } => vec![radius, inner_radius, start, end],
            Self::NSide { radius, rounding, .. } => [Some(radius), rounding].into_iter().flatten().collect(),
            Self::Star {
                inner_radius,
                outer_radius,
                twist,
                ..
            } => vec![inner_radius, outer_radius, twist],
            Self::Helix {
                start_radius,
                start_angle,
                end_radius,
                end_angle,
                radial_bias,
                height,
                height_bias,
                ..
            } => vec![
                start_radius,
                start_angle,
                end_radius,
                end_angle,
                radial_bias,
                height,
                height_bias,
            ],
            Self::Flower {
                inner_radius,
                outer_radius,
                ..
            } => vec![inner_radius, outer_radius],
        }
    }
}

impl SplinePointsDto {
    /// `None` for the default (Adaptive, 5°), which files leave out.
    fn new(interpolation: SplineInterpolation) -> Option<Self> {
        if interpolation == SplineInterpolation::default() {
            return None;
        }
        Some(match interpolation {
            SplineInterpolation::None => Self::None,
            SplineInterpolation::Natural { number } => Self::Natural { number },
            SplineInterpolation::Uniform { number } => Self::Uniform { number },
            SplineInterpolation::Adaptive { angle } => Self::Adaptive { angle },
            SplineInterpolation::Subdivided { angle, max_length } => Self::Subdivided { angle, max_length },
        })
    }

    fn into_interpolation(self) -> SplineInterpolation {
        match self {
            Self::None => SplineInterpolation::None,
            Self::Natural { number } => SplineInterpolation::Natural { number },
            Self::Uniform { number } => SplineInterpolation::Uniform { number },
            Self::Adaptive { angle } => SplineInterpolation::Adaptive { angle },
            Self::Subdivided { angle, max_length } => SplineInterpolation::Subdivided { angle, max_length },
        }
        .validated()
    }

    fn parameters(self) -> Vec<f32> {
        match self {
            Self::None | Self::Natural { .. } | Self::Uniform { .. } => Vec::new(),
            Self::Adaptive { angle } => vec![angle],
            Self::Subdivided { angle, max_length } => vec![angle, max_length],
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum LightTypeDto {
    #[default]
    Omni,
    Spot,
    Infinite,
}

impl LightTypeDto {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )]
    fn is_omni(&self) -> bool {
        *self == Self::Omni
    }
}

impl From<LightType> for LightTypeDto {
    fn from(light_type: LightType) -> Self {
        match light_type {
            LightType::Omni => Self::Omni,
            LightType::Spot => Self::Spot,
            LightType::Infinite => Self::Infinite,
        }
    }
}

impl From<LightTypeDto> for LightType {
    fn from(light_type: LightTypeDto) -> Self {
        match light_type {
            LightTypeDto::Omni => Self::Omni,
            LightTypeDto::Spot => Self::Spot,
            LightTypeDto::Infinite => Self::Infinite,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum LightShadowDto {
    #[default]
    None,
    ShadowMaps,
    Raytraced,
    Area,
}

impl LightShadowDto {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )]
    fn is_none(&self) -> bool {
        *self == Self::None
    }
}

impl From<LightShadow> for LightShadowDto {
    fn from(shadow: LightShadow) -> Self {
        match shadow {
            LightShadow::None => Self::None,
            LightShadow::ShadowMaps => Self::ShadowMaps,
            LightShadow::Raytraced => Self::Raytraced,
            LightShadow::Area => Self::Area,
        }
    }
}

impl From<LightShadowDto> for LightShadow {
    fn from(shadow: LightShadowDto) -> Self {
        match shadow {
            LightShadowDto::None => Self::None,
            LightShadowDto::ShadowMaps => Self::ShadowMaps,
            LightShadowDto::Raytraced => Self::Raytraced,
            LightShadowDto::Area => Self::Area,
        }
    }
}

/// A light's cone and falloff; written only when they differ from the defaults.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct LightDetailsDto {
    use_inner: bool,
    inner_angle_deg: f32,
    outer_angle_deg: f32,
    falloff: FalloffDto,
    radius: f32,
}

impl LightDetailsDto {
    fn new(details: LightDetails) -> Option<Self> {
        (details != LightDetails::default()).then_some(Self {
            use_inner: details.use_inner,
            inner_angle_deg: details.inner_angle_deg,
            outer_angle_deg: details.outer_angle_deg,
            falloff: match details.falloff {
                Falloff::None => FalloffDto::None,
                Falloff::Linear => FalloffDto::Linear,
                Falloff::InverseSquare => FalloffDto::InverseSquare,
            },
            radius: details.radius,
        })
    }

    /// The details, angles and radius brought into range (never trust a file).
    fn details(self) -> LightDetails {
        let angle = |a: f32| a.clamp(0.0, LightDetails::MAX_ANGLE_DEG);
        LightDetails {
            use_inner: self.use_inner,
            inner_angle_deg: angle(self.inner_angle_deg),
            outer_angle_deg: angle(self.outer_angle_deg),
            falloff: match self.falloff {
                FalloffDto::None => Falloff::None,
                FalloffDto::Linear => Falloff::Linear,
                FalloffDto::InverseSquare => Falloff::InverseSquare,
            },
            radius: self.radius.max(LightDetails::MIN_RADIUS),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FalloffDto {
    None,
    Linear,
    InverseSquare,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BoundaryDto {
    Edge,
    EdgeAndCorner,
}

impl From<Boundary> for BoundaryDto {
    fn from(boundary: Boundary) -> Self {
        match boundary {
            Boundary::Edge => Self::Edge,
            Boundary::EdgeAndCorner => Self::EdgeAndCorner,
        }
    }
}

impl From<BoundaryDto> for Boundary {
    fn from(boundary: BoundaryDto) -> Self {
        match boundary {
            BoundaryDto::Edge => Self::Edge,
            BoundaryDto::EdgeAndCorner => Self::EdgeAndCorner,
        }
    }
}

/// Flagged components as inclusive index ranges `[first, last]`, ascending;
/// edges by corner (see `grid4d_geom::ComponentSet`). Empty lists are omitted.
#[derive(Debug, Default, Serialize, Deserialize)]
struct SetDto {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    points: Vec<[u32; 2]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    edges: Vec<[u32; 2]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    polygons: Vec<[u32; 2]>,
}

impl SetDto {
    fn is_empty(&self) -> bool {
        self.points.is_empty() && self.edges.is_empty() && self.polygons.is_empty()
    }

    fn new(set: &ComponentSet) -> Self {
        // Indices of a valid mesh always fit a u32.
        let ranges = |bits: &BitSet| {
            bits.ranges()
                .map(|(first, last)| [first, last].map(|i| u32::try_from(i).unwrap_or(u32::MAX)))
                .collect()
        };
        Self {
            points: ranges(&set.points),
            edges: ranges(&set.edges),
            polygons: ranges(&set.polygons),
        }
    }

    /// The flags, checked against `mesh` (`what`/`object` name them in errors).
    fn into_set(self, mesh: &PolyMesh, object: &str, what: &str) -> Result<ComponentSet, IoError> {
        self.into_counts(mesh.point_count(), mesh.corner_count(), mesh.face_count(), object, what)
    }

    /// The flags, checked against counts of points, corners and polygons.
    fn into_counts(
        self,
        points: usize,
        corners: usize,
        polygons: usize,
        object: &str,
        what: &str,
    ) -> Result<ComponentSet, IoError> {
        let bits = |ranges: Vec<[u32; 2]>, count: usize, kind: &str| {
            let mut bits = BitSet::new();
            for [first, last] in ranges {
                let (first, last) = (
                    usize::try_from(first).unwrap_or(usize::MAX),
                    usize::try_from(last).unwrap_or(usize::MAX),
                );
                if first > last || last >= count {
                    return Err(IoError::Invalid(format!(
                        "object \"{object}\": {what} range {first}…{last} of {kind} does not fit its {count} {kind}"
                    )));
                }
                bits.insert_range(first..last + 1);
            }
            Ok(bits)
        };
        Ok(ComponentSet {
            points: bits(self.points, points, "points")?,
            edges: bits(self.edges, corners, "corners")?,
            polygons: bits(self.polygons, polygons, "polygons")?,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct MeshDto {
    points: usize,
    faces: usize,
    corners: usize,
    uvs: bool,
}

impl MeshDto {
    fn of(mesh: &PolyMesh) -> Self {
        Self {
            points: mesh.point_count(),
            faces: mesh.face_count(),
            corners: mesh.corner_count(),
            uvs: mesh.uvs().is_some(),
        }
    }

    /// Size of the mesh's binary file; `None` if it would not even fit a `u64`.
    fn byte_len(&self) -> Option<u64> {
        let n = |count: usize, bytes: u64| u64::try_from(count).ok()?.checked_mul(bytes);
        let uvs = if self.uvs { n(self.corners, 8)? } else { 0 };
        n(self.points, 12)?
            .checked_add(n(self.faces, 4)?)?
            .checked_add(n(self.corners, 4)?)?
            .checked_add(uvs)
    }
}

/// The meshes of a document being written, each once.
#[derive(Default)]
struct MeshTable {
    index_of: HashMap<MeshId, usize>,
    /// Each mesh with the name of the first object using it (for error messages).
    meshes: Vec<(MeshHandle, String)>,
}

impl MeshTable {
    fn index(&mut self, mesh: &MeshHandle, object: &str) -> usize {
        *self.index_of.entry(mesh.id()).or_insert_with(|| {
            self.meshes.push((mesh.clone(), object.to_owned()));
            self.meshes.len() - 1
        })
    }
}

/// The document's takes for the file (`None` without any).
fn takes_dto(doc: &Document, refs: &Refs, meshes: &mut MeshTable) -> Result<Option<TakesDto>, IoError> {
    let takes = &doc.takes;
    if takes.is_empty() {
        return Ok(None);
    }
    let index_of = |id: Option<TakeId>| id.and_then(|id| takes.ids().iter().position(|&t| t == id));
    let mut overrides = |objects: &ObjectOverrides| -> Vec<OverridesDto> {
        objects
            .iter()
            .filter_map(|(&id, values)| OverridesDto::new(id, values, refs, meshes))
            .collect()
    };
    let list = takes
        .iter()
        .map(|(_, take)| TakeDto {
            name: take.name.clone(),
            camera: take.camera.and_then(|c| refs.objects.get(&c).copied()),
            render: take.render.as_ref().map(OwnRenderDto::new),
            objects: overrides(&take.objects),
        })
        .collect();
    let dto = TakesDto {
        takes: list,
        active: index_of(takes.active()),
        render: index_of(takes.render_take),
        main_values: overrides(takes.main()),
        main_render: takes.main_render().map(OwnRenderDto::new),
    };
    for overrides in dto.takes.iter().flat_map(|t| &t.objects).chain(&dto.main_values) {
        overrides.check_finite()?;
    }
    Ok(Some(dto))
}

/// Rebuilds the takes; the objects already hold the active take's values.
fn read_takes(
    doc: &mut Document,
    dto: TakesDto,
    ids: &[NodeId],
    materials: &[MaterialId],
    meshes: &[MeshHandle],
) -> Result<(), IoError> {
    let objects = |list: Vec<OverridesDto>| -> Result<ObjectOverrides, IoError> {
        let mut out = ObjectOverrides::new();
        for overrides in list {
            overrides.check_finite()?;
            if let Some((id, values)) = overrides.into_values(ids, materials, meshes)? {
                out.insert(id, values);
            }
        }
        Ok(out)
    };
    let mut takes = Takes::default();
    let mut take_ids = Vec::with_capacity(dto.takes.len());
    for take in dto.takes {
        take_ids.push(takes.insert(Take {
            name: take.name,
            camera: take.camera.and_then(|i| ids.get(i).copied()),
            render: take.render.map(OwnRenderDto::into_settings),
            objects: objects(take.objects)?,
        }));
    }
    takes.render_take = dto.render.and_then(|i| take_ids.get(i).copied());
    let active = dto.active.and_then(|i| take_ids.get(i).copied());
    takes.restore(
        active,
        objects(dto.main_values)?,
        dto.main_render.map(OwnRenderDto::into_settings),
    );
    doc.takes = takes;
    Ok(())
}

/// Writes `doc` as a version 5 archive.
pub(crate) fn write(
    doc: &Document,
    cache: Option<&SimulationCache>,
    folder: Option<&std::path::Path>,
) -> Result<Vec<u8>, IoError> {
    // Pre-order: a parent is always written (and indexed) before its children.
    // Indexed first, so tags can refer to objects written after them.
    let mut order: Vec<NodeId> = Vec::with_capacity(doc.scene.len());
    doc.scene.walk(&mut Vec::new(), |id, _, _, _| {
        order.push(id);
        true
    });
    let refs = Refs {
        objects: order.iter().enumerate().map(|(i, &id)| (id, i)).collect(),
        materials: doc
            .scene
            .materials()
            .ids()
            .iter()
            .enumerate()
            .map(|(i, &id)| (id, i))
            .collect(),
        layers: doc
            .scene
            .layers()
            .ids()
            .iter()
            .enumerate()
            .map(|(i, &id)| (id, i))
            .collect(),
        textures: doc
            .scene
            .textures()
            .ids()
            .iter()
            .enumerate()
            .map(|(i, &id)| (id, i))
            .collect(),
    };
    let mut objects = Vec::with_capacity(order.len());
    let mut table = MeshTable::default();
    for &id in &order {
        let Some(node) = doc.scene.get(id) else { continue };
        let parent = node.parent().and_then(|p| refs.objects.get(&p).copied());
        objects.push(ObjectDto::new(&node.data, parent, &mut table, &refs));
    }
    for (_, material) in doc.scene.materials().iter() {
        check_material_finite(material)?;
    }
    for (_, layer) in doc.scene.layers().iter() {
        if !layer.color.iter().all(|v| v.is_finite()) {
            return Err(IoError::Invalid(format!(
                "layer \"{}\" has a colour that is not a finite number",
                layer.name
            )));
        }
    }
    for object in &objects {
        object.check_finite()?;
    }
    for (mesh, object) in &table.meshes {
        check_mesh_finite(mesh.mesh(), object)?;
    }
    let takes = takes_dto(doc, &refs, &mut table)?;
    let saved_cache = cache.and_then(|c| SimCacheDto::new(c, &refs));
    let settings = doc.time.settings();
    let (simulation_cache, cache_bytes) = saved_cache.map_or((None, None), |(dto, bytes)| (Some(dto), Some(bytes)));
    let file = File {
        format: FORMAT_TAG.to_owned(),
        version: VERSION,
        document: DocumentDto {
            name: doc.name.clone(),
            time: TimeDto {
                fps: settings.fps,
                start: settings.start,
                end: settings.end,
                looping: settings.looping,
            },
            objects,
            materials: doc
                .scene
                .materials()
                .iter()
                .map(|(_, m)| MaterialDto::new(m, &refs))
                .collect(),
            layers: doc.scene.layers().iter().map(|(_, l)| LayerDto::new(l)).collect(),
            takes,
            render: RenderDto::new(&doc.render),
            simulation: SimulationDto::new(&doc.simulation),
            simulation_cache,
            textures: doc
                .scene
                .textures()
                .iter()
                .map(|(_, t)| TextureDto::new(t, folder))
                .collect(),
        },
        meshes: table.meshes.iter().map(|(mesh, _)| MeshDto::of(mesh.mesh())).collect(),
    };
    let mut json = serde_json::to_string_pretty(&file)?;
    json.push('\n');

    // Fixed timestamps: saving the same document twice gives the same bytes.
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(DateTime::default());
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(DOCUMENT_ENTRY, options)?;
    zip.write_all(json.as_bytes())?;
    for (index, (mesh, _)) in table.meshes.iter().enumerate() {
        let bytes = encode_mesh(mesh.mesh());
        let large = u64::try_from(bytes.len()).unwrap_or(u64::MAX) >= u64::from(u32::MAX);
        zip.start_file(mesh_entry(index), options.large_file(large))?;
        zip.write_all(&bytes)?;
    }
    for (index, (_, texture)) in doc.scene.textures().iter().enumerate() {
        if let TextureSource::Embedded { file_name, bytes } = &texture.source {
            let large = u64::try_from(bytes.len()).unwrap_or(u64::MAX) >= u64::from(u32::MAX);
            zip.start_file(texture_entry(index, file_name), options.large_file(large))?;
            zip.write_all(bytes)?;
        }
    }
    if let Some(bytes) = cache_bytes {
        let large = u64::try_from(bytes.len()).unwrap_or(u64::MAX) >= u64::from(u32::MAX);
        zip.start_file(SIMULATION_ENTRY, options.large_file(large))?;
        zip.write_all(&bytes)?;
    }
    Ok(zip.finish()?.into_inner())
}

impl File {
    pub(crate) fn parse(text: &str) -> Result<Self, IoError> {
        Ok(serde_json::from_str(text)?)
    }

    /// Reads an already parsed `document.json` (an older version's, upgraded).
    pub(crate) fn from_value(json: serde_json::Value) -> Result<Self, IoError> {
        Ok(serde_json::from_value(json)?)
    }

    /// Builds the document, reading the meshes (and a saved simulation cache)
    /// from `archive`.
    pub(crate) fn into_document(
        self,
        archive: &mut Archive<'_>,
        folder: Option<&std::path::Path>,
    ) -> Result<(Document, Option<SimulationCache>), IoError> {
        let meshes = self
            .meshes
            .iter()
            .enumerate()
            .map(|(index, dto)| {
                let expected = dto
                    .byte_len()
                    .ok_or_else(|| IoError::Invalid(format!("mesh {index} is impossibly large")))?;
                let bytes = archive.read(&mesh_entry(index), expected)?;
                if u64::try_from(bytes.len()).ok() != Some(expected) {
                    return Err(IoError::Invalid(format!(
                        "mesh {index} has {} bytes, but its sizes need {expected}",
                        bytes.len()
                    )));
                }
                decode_mesh(dto, &bytes)
                    .map(MeshHandle::new)
                    .map_err(|message| IoError::Invalid(format!("mesh {index}: {message}")))
            })
            .collect::<Result<Vec<_>, IoError>>()?;

        let DocumentDto {
            name,
            time,
            objects,
            materials,
            layers,
            takes,
            render,
            simulation,
            simulation_cache,
            textures,
        } = self.document;
        // Textures first: materials refer to them.
        let mut texture_table = grid4d_scene::Textures::default();
        let mut texture_ids = Vec::with_capacity(textures.len());
        for (index, dto) in textures.into_iter().enumerate() {
            texture_ids.push(texture_table.insert(dto.into_texture(index, archive, folder)?));
        }
        let materials = materials
            .into_iter()
            .map(|dto| dto.into_material(&texture_ids))
            .collect::<Result<Vec<_>, IoError>>()?;
        let layers = layers
            .into_iter()
            .map(LayerDto::into_layer)
            .collect::<Result<Vec<_>, IoError>>()?;
        let object_layers: Vec<Option<usize>> = objects.iter().map(|o| o.layer).collect();
        let mut tags: Vec<Vec<TagDto>> = Vec::with_capacity(objects.len());
        let mut links: Vec<(Option<usize>, Vec<usize>)> = Vec::with_capacity(objects.len());
        let objects = objects
            .into_iter()
            .map(|object| {
                object.check_finite()?;
                links.push(match &object.kind {
                    KindDto::Cloner { object, effectors, .. } => (*object, effectors.clone()),
                    _ => (None, Vec::new()),
                });
                let parent = object.parent;
                let (data, object_tags) = object.into_data(&meshes)?;
                tags.push(object_tags);
                Ok((parent, data))
            })
            .collect::<Result<Vec<_>, IoError>>()?;
        let time = TimeSettings {
            fps: time.fps,
            start: time.start,
            end: time.end,
            looping: time.looping,
        };
        let (mut doc, ids) = assemble_with_ids(name, time, objects)?;
        doc.render = render.map(RenderDto::into_settings).unwrap_or_default();
        doc.simulation = simulation
            .map(SimulationDto::into_settings)
            .transpose()?
            .unwrap_or_default();
        let material_ids: Vec<MaterialId> = materials
            .into_iter()
            .map(|material| doc.scene.materials_mut().insert(material))
            .collect();
        let layer_ids: Vec<LayerId> = layers
            .into_iter()
            .map(|layer| doc.scene.layers_mut().insert(layer))
            .collect();
        // An index that names no layer leaves the object on none.
        for (&id, layer) in ids.iter().zip(object_layers) {
            if let Some(index) = layer
                && let Some(node) = doc.scene.get_mut(id)
            {
                node.data.layer = layer_ids.get(index).copied();
            }
        }
        // Object links and tags last: they may refer to any object.
        for (&id, (link, effector_links)) in ids.iter().zip(links) {
            if (link.is_some() || !effector_links.is_empty())
                && let Some(node) = doc.scene.get_mut(id)
                && let ObjectKind::Cloner { object, effectors, .. } = &mut node.data.kind
            {
                *object = link.and_then(|index| ids.get(index).copied());
                *effectors = effector_links
                    .iter()
                    .filter_map(|&index| ids.get(index).copied())
                    .collect();
            }
        }
        for (&id, object_tags) in ids.iter().zip(tags) {
            if !object_tags.is_empty()
                && let Some(node) = doc.scene.get_mut(id)
            {
                node.data.tags = object_tags
                    .into_iter()
                    .map(|tag| tag.into_tag(&ids, &material_ids))
                    .collect();
            }
        }
        if let Some(takes) = takes {
            read_takes(&mut doc, takes, &ids, &material_ids, &meshes)?;
        }
        *doc.scene.textures_mut() = texture_table;
        let cache = simulation_cache.map(|dto| dto.into_cache(archive, &ids)).transpose()?;
        Ok((doc, cache))
    }
}

impl ObjectDto {
    fn new(data: &NodeData, parent: Option<usize>, meshes: &mut MeshTable, refs: &Refs) -> Self {
        let t = &data.transform;
        Self {
            name: data.name.clone(),
            parent,
            kind: {
                let mut kind = KindDto::new(&data.kind, &data.name, meshes);
                if let (
                    KindDto::Cloner { object, effectors, .. },
                    ObjectKind::Cloner {
                        object: target,
                        effectors: links,
                        ..
                    },
                ) = (&mut kind, &data.kind)
                {
                    *object = target.and_then(|id| refs.objects.get(&id).copied());
                    effectors.extend(links.iter().filter_map(|id| refs.objects.get(id).copied()));
                }
                kind
            },
            position: t.position.to_array(),
            rotation_hpb: t.rotation.to_array(),
            scale: t.scale.to_array(),
            display_color: data.display_color,
            visible_in_editor: data.visible_in_editor,
            visible_in_render: data.visible_in_render,
            enabled: data.enabled,
            tags: data.tags.iter().map(|tag| TagDto::new(tag, refs)).collect(),
            tracks: data.tracks.iter().map(|t| TrackDto::new(t, &data.kind)).collect(),
            layer: data.layer.and_then(|l| refs.layers.get(&l).copied()),
        }
    }

    /// The object's data; its tags come back separately, still referring to
    /// objects by index (they are resolved once every object exists).
    fn into_data(self, meshes: &[MeshHandle]) -> Result<(NodeData, Vec<TagDto>), IoError> {
        let kind = self.kind.into_kind(meshes, &self.name)?;
        // One track per parameter: a later one replaces an earlier one.
        let tracks = self.tracks.into_iter().fold(Vec::new(), |mut tracks: Vec<Track>, dto| {
            if let Some(track) = dto.into_track(&kind) {
                tracks.retain(|t| t.param != track.param);
                tracks.push(track);
            }
            tracks
        });
        Ok(NodeData {
            name: self.name,
            kind,
            transform: Transform {
                position: Vec3::from_array(self.position),
                rotation: Vec3::from_array(self.rotation_hpb),
                scale: Vec3::from_array(self.scale),
            },
            display_color: self.display_color,
            visible_in_editor: self.visible_in_editor,
            visible_in_render: self.visible_in_render,
            enabled: self.enabled,
            tags: Vec::new(),
            tracks,
            unkeyed: None,
            layer: None,
        })
        .map(|data| (data, self.tags))
    }

    fn check_finite(&self) -> Result<(), IoError> {
        if !self.tracks.iter().all(TrackDto::is_finite) {
            return Err(IoError::Invalid(format!(
                "object \"{}\" has an animation key that is not a finite number",
                self.name
            )));
        }
        check_finite(
            &self.name,
            &[
                ("position", &self.position),
                ("rotation", &self.rotation_hpb),
                ("scale", &self.scale),
                ("display colour", &self.display_color),
                ("parameters", &self.kind.parameters()),
                (
                    "tag",
                    &self.tags.iter().flat_map(TagDto::parameters).collect::<Vec<f32>>(),
                ),
            ],
        )
    }
}

impl KindDto {
    fn new(kind: &ObjectKind, object: &str, meshes: &mut MeshTable) -> Self {
        match *kind {
            ObjectKind::Null => Self::Null,
            ObjectKind::Plugin(ref plugin) => Self::Plugin {
                plugin: plugin.type_id.clone(),
                values: plugin
                    .values
                    .iter()
                    .map(|(name, value)| PluginValueDto {
                        name: name.clone(),
                        value: (*value).into(),
                    })
                    .collect(),
            },
            ObjectKind::Cube { size, segments } => Self::Cube {
                size: size.to_array(),
                segments,
            },
            ObjectKind::Sphere { radius, segments } => Self::Sphere { radius, segments },
            ObjectKind::Cylinder {
                radius,
                height,
                rotation_segments,
                height_segments,
            } => Self::Cylinder {
                radius,
                height,
                rotation_segments,
                height_segments,
            },
            ObjectKind::Plane {
                width,
                depth,
                width_segments,
                depth_segments,
            } => Self::Plane {
                width,
                depth,
                width_segments,
                depth_segments,
            },
            ObjectKind::Cone {
                bottom_radius,
                top_radius,
                height,
                rotation_segments,
                height_segments,
            } => Self::Cone {
                bottom_radius,
                top_radius,
                height,
                rotation_segments,
                height_segments,
            },
            ObjectKind::Torus {
                ring_radius,
                pipe_radius,
                ring_segments,
                pipe_segments,
            } => Self::Torus {
                ring_radius,
                pipe_radius,
                ring_segments,
                pipe_segments,
            },
            ObjectKind::Disc {
                inner_radius,
                outer_radius,
                rotation_segments,
                disc_segments,
            } => Self::Disc {
                inner_radius,
                outer_radius,
                rotation_segments,
                disc_segments,
            },
            ObjectKind::Capsule {
                radius,
                height,
                rotation_segments,
                height_segments,
                cap_segments,
            } => Self::Capsule {
                radius,
                height,
                rotation_segments,
                height_segments,
                cap_segments,
            },
            ObjectKind::Camera { fov_y_deg } => Self::Camera { fov_y_deg },
            ObjectKind::Light {
                color,
                intensity,
                light_type,
                details,
                shadow,
            } => Self::Light {
                color,
                intensity,
                light_type: light_type.into(),
                details: LightDetailsDto::new(details),
                shadow: shadow.into(),
            },
            ObjectKind::Polygon {
                ref mesh,
                ref components,
            } => Self::Polygon {
                mesh: meshes.index(mesh, object),
                selected: SetDto::new(components.selected()),
                hidden: SetDto::new(components.hidden()),
                seams: SetDto::new(&ComponentSet {
                    edges: components.state().seams.clone(),
                    ..ComponentSet::default()
                }),
            },
            ObjectKind::SubdivisionSurface {
                viewport_levels,
                render_levels,
                linear_pre,
                boundary,
            } => Self::SubdivisionSurface {
                viewport_levels,
                render_levels,
                linear_pre,
                boundary: boundary.into(),
            },
            ObjectKind::Extrude {
                extrusion,
                hierarchical,
            } => Self::Extrude {
                direction: match extrusion.direction {
                    ExtrudeDirection::Auto => ExtrudeDirectionDto::Auto,
                    ExtrudeDirection::X => ExtrudeDirectionDto::X,
                    ExtrudeDirection::Y => ExtrudeDirectionDto::Y,
                    ExtrudeDirection::Z => ExtrudeDirectionDto::Z,
                    ExtrudeDirection::Custom => ExtrudeDirectionDto::Custom,
                    ExtrudeDirection::Absolute => ExtrudeDirectionDto::Absolute,
                },
                offset: extrusion.offset,
                movement: extrusion.movement.to_array(),
                subdivision: extrusion.subdivision,
                reverse_normals: extrusion.reverse_normals,
                hierarchical,
                start_cap: extrusion.start_cap,
                end_cap: extrusion.end_cap,
                cap_type: extrusion.cap_type.into(),
            },
            ObjectKind::Text(ref text) => Self::Text {
                text: text.text.to_string(),
                height: text.height,
                align: match text.align {
                    TextAlign::Left => TextAlignDto::Left,
                    TextAlign::Center => TextAlignDto::Center,
                    TextAlign::Right => TextAlignDto::Right,
                },
                horizontal_spacing: text.horizontal_spacing,
                vertical_spacing: text.vertical_spacing,
                plane: match text.plane {
                    SplinePlane::XY => SplinePlaneDto::Xy,
                    SplinePlane::ZY => SplinePlaneDto::Zy,
                    SplinePlane::XZ => SplinePlaneDto::Xz,
                },
                reverse: text.reverse,
                intermediate_points: SplinePointsDto::new(text.interpolation),
            },
            ObjectKind::Bend(bend) => Self::Bend {
                size: bend.size.to_array(),
                mode: match bend.mode {
                    DeformerMode::Limited => DeformerModeDto::Limited,
                    DeformerMode::WithinBox => DeformerModeDto::WithinBox,
                    DeformerMode::Unlimited => DeformerModeDto::Unlimited,
                },
                strength: bend.strength,
                angle: bend.angle,
                keep_length: bend.keep_length,
                origin: match bend.origin {
                    BendOrigin::Bottom => BendOriginDto::Bottom,
                    BendOrigin::CenterC => BendOriginDto::CenterC,
                    BendOrigin::CenterS => BendOriginDto::CenterS,
                    BendOrigin::Top => BendOriginDto::Top,
                },
            },
            ObjectKind::Ffd {
                ffd,
                ref lattice,
                ref components,
            } => {
                let points = &lattice.spline().points;
                Self::Ffd {
                    size: ffd.size.to_array(),
                    grid: ffd.grid,
                    mode: ffd.mode.into(),
                    lattice: if *points == ffd.grid_points() {
                        Vec::new()
                    } else {
                        points.iter().map(|p| p.to_array()).collect()
                    },
                    selected: SetDto::new(components.selected()),
                    hidden: SetDto::new(components.hidden()),
                }
            }
            ObjectKind::Twist(d) => Self::Twist {
                size: d.size.to_array(),
                mode: d.mode.into(),
                angle: d.angle,
            },
            ObjectKind::Taper(d) => Self::Taper {
                size: d.size.to_array(),
                mode: d.mode.into(),
                strength: d.strength,
                curvature: d.shape.curvature,
                fillet: d.shape.fillet,
            },
            ObjectKind::Bulge(d) => Self::Bulge {
                size: d.size.to_array(),
                mode: d.mode.into(),
                strength: d.strength,
                curvature: d.shape.curvature,
                fillet: d.shape.fillet,
            },
            ObjectKind::Lathe(lathe) => Self::Lathe {
                angle: lathe.angle,
                subdivision: lathe.subdivision,
                movement: lathe.movement,
                scaling: lathe.scaling,
                reverse_normals: lathe.reverse_normals,
                start_cap: lathe.start_cap,
                end_cap: lathe.end_cap,
                cap_type: lathe.cap_type.into(),
            },
            ObjectKind::Cloner { cloner, .. } => Self::Cloner {
                layout: ClonerLayoutDto::new(cloner.layout),
                object: None,
                effectors: Vec::new(),
                clones: match cloner.clones {
                    CloneOrder::Iterate => CloneOrderDto::Iterate,
                    CloneOrder::Random => CloneOrderDto::Random,
                },
                seed: cloner.seed,
                reset_coordinates: cloner.reset_coordinates,
            },
            ObjectKind::PlainEffector(effector) => Self::PlainEffector(EffectorDto::new(&effector)),
            ObjectKind::RandomEffector(effector) => Self::RandomEffector(EffectorDto::new(&effector)),
            ObjectKind::Loft(loft) => Self::Loft {
                subdivision_u: loft.subdivision_u,
                subdivision_v: loft.subdivision_v,
                organic: loft.organic,
                per_segment: loft.per_segment,
                looped: loft.looped,
                linear: loft.linear,
                reverse_normals: loft.reverse_normals,
                start_cap: loft.start_cap,
                end_cap: loft.end_cap,
                cap_type: loft.cap_type.into(),
            },
            ObjectKind::Sweep(sweep) => Self::Sweep {
                end_scale: sweep.end_scale,
                end_rotation: sweep.end_rotation,
                start_growth: sweep.start_growth,
                end_growth: sweep.end_growth,
                parallel_movement: sweep.parallel_movement,
                constant_cross_section: sweep.constant_cross_section,
                banking: sweep.banking,
                reverse_normals: sweep.reverse_normals,
                start_cap: sweep.start_cap,
                end_cap: sweep.end_cap,
                cap_type: sweep.cap_type.into(),
            },
            ObjectKind::Spline {
                ref spline,
                interpolation,
                ref components,
            } => {
                let spline = spline.spline();
                let tangents = if spline.tangents.iter().all(|t| *t == [Vec3::ZERO; 2]) {
                    Vec::new()
                } else {
                    spline
                        .tangents
                        .iter()
                        .map(|[l, r]| [l.to_array(), r.to_array()])
                        .collect()
                };
                Self::EditableSpline {
                    spline_type: match spline.kind {
                        SplineType::Linear => SplineTypeDto::Linear,
                        SplineType::Cubic => SplineTypeDto::Cubic,
                        SplineType::Akima => SplineTypeDto::Akima,
                        SplineType::BSpline => SplineTypeDto::BSpline,
                        SplineType::Bezier => SplineTypeDto::Bezier,
                    },
                    points: spline.points.iter().map(|p| p.to_array()).collect(),
                    tangents,
                    segments: spline
                        .segments
                        .iter()
                        .map(|s| SegmentDto {
                            count: s.count,
                            closed: s.closed,
                        })
                        .collect(),
                    intermediate_points: SplinePointsDto::new(interpolation),
                    selected: SetDto::new(components.selected()),
                    hidden: SetDto::new(components.hidden()),
                }
            }
            ObjectKind::SplinePrimitive(primitive) => Self::Spline {
                spline: SplineShapeDto::new(primitive.shape),
                plane: match primitive.plane {
                    SplinePlane::XY => SplinePlaneDto::Xy,
                    SplinePlane::ZY => SplinePlaneDto::Zy,
                    SplinePlane::XZ => SplinePlaneDto::Xz,
                },
                reverse: primitive.reverse,
                intermediate_points: SplinePointsDto::new(primitive.interpolation),
            },
        }
    }

    fn into_kind(self, meshes: &[MeshHandle], object: &str) -> Result<ObjectKind, IoError> {
        Ok(match self {
            Self::Null => ObjectKind::Null,
            Self::Cube { size, segments } => ObjectKind::Cube {
                size: Vec3::from_array(size),
                segments,
            },
            Self::Sphere { radius, segments } => ObjectKind::Sphere { radius, segments },
            Self::Cylinder {
                radius,
                height,
                rotation_segments,
                height_segments,
            } => ObjectKind::Cylinder {
                radius,
                height,
                rotation_segments,
                height_segments,
            },
            Self::Plane {
                width,
                depth,
                width_segments,
                depth_segments,
            } => ObjectKind::Plane {
                width,
                depth,
                width_segments,
                depth_segments,
            },
            Self::Cone {
                bottom_radius,
                top_radius,
                height,
                rotation_segments,
                height_segments,
            } => ObjectKind::Cone {
                bottom_radius,
                top_radius,
                height,
                rotation_segments,
                height_segments,
            },
            Self::Torus {
                ring_radius,
                pipe_radius,
                ring_segments,
                pipe_segments,
            } => ObjectKind::Torus {
                ring_radius,
                pipe_radius,
                ring_segments,
                pipe_segments,
            },
            Self::Disc {
                inner_radius,
                outer_radius,
                rotation_segments,
                disc_segments,
            } => ObjectKind::Disc {
                inner_radius,
                outer_radius,
                rotation_segments,
                disc_segments,
            },
            Self::Capsule {
                radius,
                height,
                rotation_segments,
                height_segments,
                cap_segments,
            } => ObjectKind::Capsule {
                radius,
                height,
                rotation_segments,
                height_segments,
                cap_segments,
            },
            Self::Camera { fov_y_deg } => ObjectKind::Camera { fov_y_deg },
            Self::Light {
                color,
                intensity,
                light_type,
                details,
                shadow,
            } => ObjectKind::Light {
                color,
                intensity,
                light_type: light_type.into(),
                details: details.map_or_else(LightDetails::default, LightDetailsDto::details),
                shadow: shadow.into(),
            },
            Self::Polygon {
                mesh,
                selected,
                hidden,
                seams,
            } => {
                let mesh = meshes.get(mesh).cloned().ok_or_else(|| {
                    IoError::Invalid(format!(
                        "object \"{object}\" uses mesh {mesh}, but the file has {} meshes",
                        meshes.len()
                    ))
                })?;
                let state = ComponentState {
                    selected: selected.into_set(mesh.mesh(), object, "selection")?,
                    hidden: hidden.into_set(mesh.mesh(), object, "hidden")?,
                    seams: seams.into_set(mesh.mesh(), object, "seams")?.edges,
                };
                ObjectKind::Polygon {
                    mesh,
                    components: Components::new(state),
                }
            }
            // Never trust a level a file claims: each one multiplies the faces by 4.
            Self::SubdivisionSurface {
                viewport_levels,
                render_levels,
                linear_pre,
                boundary,
            } => ObjectKind::SubdivisionSurface {
                viewport_levels: viewport_levels.min(MAX_LEVEL),
                render_levels: render_levels.min(MAX_LEVEL),
                linear_pre: linear_pre.min(MAX_LEVEL),
                boundary: boundary.into(),
            },
            Self::Extrude {
                direction,
                offset,
                movement,
                subdivision,
                reverse_normals,
                hierarchical,
                start_cap,
                end_cap,
                cap_type,
            } => ObjectKind::Extrude {
                extrusion: Extrusion {
                    direction: match direction {
                        ExtrudeDirectionDto::Auto => ExtrudeDirection::Auto,
                        ExtrudeDirectionDto::X => ExtrudeDirection::X,
                        ExtrudeDirectionDto::Y => ExtrudeDirection::Y,
                        ExtrudeDirectionDto::Z => ExtrudeDirection::Z,
                        ExtrudeDirectionDto::Custom => ExtrudeDirection::Custom,
                        ExtrudeDirectionDto::Absolute => ExtrudeDirection::Absolute,
                    },
                    offset,
                    movement: Vec3::from_array(movement),
                    // Never trust a count a file claims: each ring adds a shell.
                    subdivision: subdivision.clamp(1, Extrusion::MAX_SUBDIVISION),
                    reverse_normals,
                    start_cap,
                    end_cap,
                    cap_type: cap_type.into(),
                },
                hierarchical,
            },
            Self::Bend {
                size,
                mode,
                strength,
                angle,
                keep_length,
                origin,
            } => ObjectKind::Bend(Bend {
                size: Vec3::from_array(size).abs(),
                mode: match mode {
                    DeformerModeDto::Limited => DeformerMode::Limited,
                    DeformerModeDto::WithinBox => DeformerMode::WithinBox,
                    DeformerModeDto::Unlimited => DeformerMode::Unlimited,
                },
                strength,
                angle,
                keep_length,
                origin: match origin {
                    BendOriginDto::Bottom => BendOrigin::Bottom,
                    BendOriginDto::CenterC => BendOrigin::CenterC,
                    BendOriginDto::CenterS => BendOrigin::CenterS,
                    BendOriginDto::Top => BendOrigin::Top,
                },
            }),
            Self::Ffd {
                size,
                grid,
                mode,
                lattice,
                selected,
                hidden,
            } => {
                let ffd = Ffd {
                    size: Vec3::from_array(size).abs(),
                    // Never trust a count a file claims.
                    grid: grid.map(|n| n.clamp(Ffd::MIN_GRID, Ffd::MAX_GRID)),
                    mode: mode.into(),
                };
                let regular = ffd.grid_points();
                let points: Vec<Vec3> = if lattice.is_empty() {
                    regular
                } else if lattice.len() == regular.len() {
                    lattice.into_iter().map(Vec3::from_array).collect()
                } else {
                    return Err(IoError::Invalid(format!(
                        "FFD \"{object}\" has {} lattice points for a grid of {}",
                        lattice.len(),
                        regular.len()
                    )));
                };
                let count = points.len();
                let state = ComponentState {
                    selected: selected.into_counts(count, 0, 0, object, "selected")?,
                    hidden: hidden.into_counts(count, 0, 0, object, "hidden")?,
                    seams: BitSet::new(),
                };
                ObjectKind::Ffd {
                    ffd,
                    lattice: SplineHandle::new(Spline::new(SplineType::Linear, points, false)),
                    components: Components::new(state),
                }
            }
            Self::Twist { size, mode, angle } => ObjectKind::Twist(Twist {
                size: Vec3::from_array(size).abs(),
                mode: mode.into(),
                angle,
            }),
            Self::Taper {
                size,
                mode,
                strength,
                curvature,
                fillet,
            } => ObjectKind::Taper(Taper {
                size: Vec3::from_array(size).abs(),
                mode: mode.into(),
                strength,
                shape: Shape {
                    curvature: curvature.max(0.0),
                    fillet,
                },
            }),
            Self::Bulge {
                size,
                mode,
                strength,
                curvature,
                fillet,
            } => ObjectKind::Bulge(Bulge {
                size: Vec3::from_array(size).abs(),
                mode: mode.into(),
                strength,
                shape: Shape {
                    curvature: curvature.max(0.0),
                    fillet,
                },
            }),
            Self::Plugin { plugin, values } => ObjectKind::Plugin(PluginObject {
                type_id: plugin,
                values: values.into_iter().map(|v| (v.name, v.value.into())).collect(),
            }),
            Self::Text {
                text,
                height,
                align,
                horizontal_spacing,
                vertical_spacing,
                plane,
                reverse,
                intermediate_points,
            } => ObjectKind::Text(Text {
                // Never trust a size a file claims: long texts are cut.
                text: text.chars().take(Text::MAX_CHARS).collect::<String>().into(),
                height: height.max(0.0),
                align: match align {
                    TextAlignDto::Left => TextAlign::Left,
                    TextAlignDto::Center => TextAlign::Center,
                    TextAlignDto::Right => TextAlign::Right,
                },
                horizontal_spacing,
                vertical_spacing,
                plane: match plane {
                    SplinePlaneDto::Xy => SplinePlane::XY,
                    SplinePlaneDto::Zy => SplinePlane::ZY,
                    SplinePlaneDto::Xz => SplinePlane::XZ,
                },
                reverse,
                interpolation: intermediate_points
                    .map_or_else(SplineInterpolation::default, SplinePointsDto::into_interpolation),
            }),
            Self::Lathe {
                angle,
                subdivision,
                movement,
                scaling,
                reverse_normals,
                start_cap,
                end_cap,
                cap_type,
            } => ObjectKind::Lathe(Lathe {
                angle,
                // Never trust a count a file claims: each step adds a ring.
                subdivision: subdivision.clamp(Lathe::MIN_SUBDIVISION, Lathe::MAX_SUBDIVISION),
                movement,
                scaling: scaling.max(0.0),
                reverse_normals,
                start_cap,
                end_cap,
                cap_type: cap_type.into(),
            }),
            Self::PlainEffector(dto) => ObjectKind::PlainEffector(dto.into_effector(Effector::plain())),
            Self::RandomEffector(dto) => ObjectKind::RandomEffector(dto.into_effector(Effector::random())),
            Self::Cloner {
                layout,
                object: _,
                effectors: _,
                clones,
                seed,
                reset_coordinates,
            } => ObjectKind::cloner(Cloner {
                layout: layout.into_layout(),
                clones: match clones {
                    CloneOrderDto::Iterate => CloneOrder::Iterate,
                    CloneOrderDto::Random => CloneOrder::Random,
                },
                seed,
                reset_coordinates,
            }),
            Self::Loft {
                subdivision_u,
                subdivision_v,
                organic,
                per_segment,
                looped,
                linear,
                reverse_normals,
                start_cap,
                end_cap,
                cap_type,
            } => ObjectKind::Loft(Loft {
                // Never trust a count a file claims.
                subdivision_u: subdivision_u.clamp(Loft::MIN_SUBDIVISION, Loft::MAX_SUBDIVISION),
                subdivision_v: subdivision_v.clamp(Loft::MIN_SUBDIVISION, Loft::MAX_SUBDIVISION),
                organic,
                per_segment,
                looped,
                linear,
                reverse_normals,
                start_cap,
                end_cap,
                cap_type: cap_type.into(),
            }),
            Self::Sweep {
                end_scale,
                end_rotation,
                start_growth,
                end_growth,
                parallel_movement,
                constant_cross_section,
                banking,
                reverse_normals,
                start_cap,
                end_cap,
                cap_type,
            } => ObjectKind::Sweep(Sweep {
                end_scale: end_scale.max(0.0),
                end_rotation,
                start_growth: start_growth.clamp(0.0, 1.0),
                end_growth: end_growth.clamp(0.0, 1.0),
                parallel_movement,
                constant_cross_section,
                banking,
                reverse_normals,
                start_cap,
                end_cap,
                cap_type: cap_type.into(),
            }),
            Self::Spline {
                spline,
                plane,
                reverse,
                intermediate_points,
            } => ObjectKind::SplinePrimitive(SplinePrimitive {
                shape: spline.into_shape(),
                plane: match plane {
                    SplinePlaneDto::Xy => SplinePlane::XY,
                    SplinePlaneDto::Zy => SplinePlane::ZY,
                    SplinePlaneDto::Xz => SplinePlane::XZ,
                },
                reverse,
                interpolation: intermediate_points
                    .map_or_else(SplineInterpolation::default, SplinePointsDto::into_interpolation),
            }),
            Self::EditableSpline {
                spline_type,
                points,
                tangents,
                segments,
                intermediate_points,
                selected,
                hidden,
            } => {
                let count = points.len();
                if !tangents.is_empty() && tangents.len() != count {
                    return Err(IoError::Invalid(format!(
                        "spline \"{object}\" has {} tangents for {count} points",
                        tangents.len()
                    )));
                }
                let claimed: u64 = segments.iter().map(|s| u64::from(s.count)).sum();
                if u64::try_from(count).ok() != Some(claimed) {
                    return Err(IoError::Invalid(format!(
                        "spline \"{object}\": its segments count {claimed} points, it has {count}"
                    )));
                }
                let spline = Spline {
                    kind: match spline_type {
                        SplineTypeDto::Linear => SplineType::Linear,
                        SplineTypeDto::Cubic => SplineType::Cubic,
                        SplineTypeDto::Akima => SplineType::Akima,
                        SplineTypeDto::BSpline => SplineType::BSpline,
                        SplineTypeDto::Bezier => SplineType::Bezier,
                    },
                    points: points.into_iter().map(Vec3::from_array).collect(),
                    tangents: if tangents.is_empty() {
                        vec![[Vec3::ZERO; 2]; count]
                    } else {
                        tangents
                            .into_iter()
                            .map(|[l, r]| [Vec3::from_array(l), Vec3::from_array(r)])
                            .collect()
                    },
                    segments: segments
                        .into_iter()
                        .map(|s| Segment {
                            count: s.count,
                            closed: s.closed,
                        })
                        .collect(),
                };
                let state = ComponentState {
                    selected: selected.into_counts(count, 0, 0, object, "selected")?,
                    hidden: hidden.into_counts(count, 0, 0, object, "hidden")?,
                    seams: BitSet::new(),
                };
                ObjectKind::Spline {
                    spline: SplineHandle::new(spline),
                    interpolation: intermediate_points
                        .map_or_else(SplineInterpolation::default, SplinePointsDto::into_interpolation),
                    components: Components::new(state),
                }
            }
        })
    }

    /// Every floating-point parameter, for validation.
    fn parameters(&self) -> Vec<f32> {
        match *self {
            Self::Null | Self::Polygon { .. } | Self::SubdivisionSurface { .. } | Self::Loft { .. } => Vec::new(),
            Self::Cloner { ref layout, .. } => layout.parameters(),
            Self::PlainEffector(ref dto) | Self::RandomEffector(ref dto) => dto.parameters(),
            Self::Extrude { offset, movement, .. } => vec![offset, movement[0], movement[1], movement[2]],
            Self::Lathe {
                angle,
                movement,
                scaling,
                ..
            } => vec![angle, movement, scaling],
            Self::Bend {
                size, strength, angle, ..
            } => vec![size[0], size[1], size[2], strength, angle],
            Self::Twist { size, angle, .. } => vec![size[0], size[1], size[2], angle],
            Self::Ffd { size, ref lattice, .. } => {
                let mut numbers = size.to_vec();
                numbers.extend(lattice.iter().flatten().copied());
                numbers
            }
            Self::Taper {
                size,
                strength,
                curvature,
                ..
            }
            | Self::Bulge {
                size,
                strength,
                curvature,
                ..
            } => vec![size[0], size[1], size[2], strength, curvature],
            Self::Plugin { ref values, .. } => values
                .iter()
                .flat_map(|v| PluginValue::from(v.value).numbers())
                .collect(),
            Self::Text {
                height,
                horizontal_spacing,
                vertical_spacing,
                intermediate_points,
                ..
            } => {
                let mut numbers = vec![height, horizontal_spacing, vertical_spacing];
                numbers.extend(intermediate_points.map(SplinePointsDto::parameters).unwrap_or_default());
                numbers
            }
            Self::Sweep {
                end_scale,
                end_rotation,
                start_growth,
                end_growth,
                ..
            } => vec![end_scale, end_rotation, start_growth, end_growth],
            Self::EditableSpline {
                ref points,
                ref tangents,
                intermediate_points,
                ..
            } => {
                let mut numbers: Vec<f32> = points.iter().flatten().copied().collect();
                numbers.extend(tangents.iter().flatten().flatten().copied());
                numbers.extend(intermediate_points.map(SplinePointsDto::parameters).unwrap_or_default());
                numbers
            }
            Self::Spline {
                spline,
                intermediate_points,
                ..
            } => {
                let mut numbers = spline.parameters();
                numbers.extend(intermediate_points.map(SplinePointsDto::parameters).unwrap_or_default());
                numbers
            }
            Self::Cube { size, .. } => size.to_vec(),
            Self::Sphere { radius, .. } => vec![radius],
            Self::Cylinder { radius, height, .. } | Self::Capsule { radius, height, .. } => vec![radius, height],
            Self::Plane { width, depth, .. } => vec![width, depth],
            Self::Cone {
                bottom_radius,
                top_radius,
                height,
                ..
            } => vec![bottom_radius, top_radius, height],
            Self::Torus {
                ring_radius,
                pipe_radius,
                ..
            } => vec![ring_radius, pipe_radius],
            Self::Disc {
                inner_radius,
                outer_radius,
                ..
            } => vec![inner_radius, outer_radius],
            Self::Camera { fov_y_deg } => vec![fov_y_deg],
            Self::Light {
                color,
                intensity,
                details,
                ..
            } => {
                let mut numbers = vec![color[0], color[1], color[2], intensity];
                if let Some(details) = details {
                    numbers.extend([details.inner_angle_deg, details.outer_angle_deg, details.radius]);
                }
                numbers
            }
        }
    }
}

fn check_mesh_finite(mesh: &PolyMesh, object: &str) -> Result<(), IoError> {
    let points_ok = mesh.points().iter().all(|p| p.is_finite());
    let uvs_ok = mesh.uvs().is_none_or(|uvs| uvs.iter().all(|uv| uv.is_finite()));
    if points_ok && uvs_ok {
        Ok(())
    } else {
        Err(IoError::Invalid(format!(
            "object \"{object}\" has a mesh point or UV that is not a finite number"
        )))
    }
}

fn encode_mesh(mesh: &PolyMesh) -> Vec<u8> {
    let dto = MeshDto::of(mesh);
    let mut out = Vec::with_capacity(dto.byte_len().and_then(|n| usize::try_from(n).ok()).unwrap_or(0));
    let f32s = |values: &[f32], out: &mut Vec<u8>| {
        for value in values {
            out.extend_from_slice(&value.to_le_bytes());
        }
    };
    for point in mesh.points() {
        f32s(&point.to_array(), &mut out);
    }
    for face in mesh.faces() {
        // A validated mesh never has more corners than a u32 can count.
        out.extend_from_slice(&u32::try_from(face.len()).unwrap_or(u32::MAX).to_le_bytes());
    }
    for corner in mesh.corner_points() {
        out.extend_from_slice(&corner.to_le_bytes());
    }
    for uv in mesh.uvs().unwrap_or_default() {
        f32s(&uv.to_array(), &mut out);
    }
    out
}

/// Rebuilds a mesh from its binary file (`bytes` has exactly the expected length),
/// validating it like any other mesh.
fn decode_mesh(dto: &MeshDto, bytes: &[u8]) -> Result<PolyMesh, String> {
    let (words, _) = bytes.as_chunks::<4>();
    let mut words = words.iter();
    // The length was checked against the sizes, so the words never run out.
    let mut u32s = |n: usize| -> Vec<u32> { words.by_ref().take(n).map(|w| u32::from_le_bytes(*w)).collect() };
    let point_values = u32s(dto.points.saturating_mul(3));
    let sizes = u32s(dto.faces);
    let corners = u32s(dto.corners);
    let uv_values = if dto.uvs {
        u32s(dto.corners.saturating_mul(2))
    } else {
        Vec::new()
    };

    let floats = |values: &[u32]| -> Result<Vec<f32>, String> {
        let floats: Vec<f32> = values.iter().map(|&bits| f32::from_bits(bits)).collect();
        if floats.iter().all(|v| v.is_finite()) {
            Ok(floats)
        } else {
            Err("a point or UV is not a finite number".to_owned())
        }
    };
    let points: Vec<Vec3> = floats(&point_values)?
        .as_chunks::<3>()
        .0
        .iter()
        .map(|&p| Vec3::from_array(p))
        .collect();

    let total = sizes
        .iter()
        .try_fold(0usize, |sum, &n| sum.checked_add(usize::try_from(n).ok()?));
    if total != Some(corners.len()) {
        return Err(format!(
            "the faces' corner counts do not add up to its {} corners",
            corners.len()
        ));
    }
    let mut start = 0;
    let faces = sizes.iter().map(|&n| {
        let face = &corners[start..start + usize::try_from(n).unwrap_or(0)];
        start += face.len();
        face
    });
    let mut mesh = PolyMesh::from_faces(points, faces).map_err(|err| err.to_string())?;
    if dto.uvs {
        let uvs = floats(&uv_values)?
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&uv| Vec2::from_array(uv))
            .collect();
        mesh.set_uvs(Some(uvs)).map_err(|err| err.to_string())?;
    }
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use grid4d_scene::{Node, ObjectKindTag};
    use zip::ZipArchive;

    use super::*;
    use crate::{FORMAT_VERSION, from_bytes, to_bytes};

    /// What a file must preserve: document name, time settings, and the
    /// hierarchy in pre-order (depth + data). `NodeId`s are not part of a file.
    fn snapshot(doc: &Document) -> (String, TimeSettings, Vec<(u32, NodeData)>) {
        let mut nodes = Vec::new();
        doc.scene.walk(&mut Vec::new(), |_, node, _, depth| {
            nodes.push((depth, node.data.clone()));
            true
        });
        (doc.name.clone(), doc.time.settings(), nodes)
    }

    fn round_trip(doc: &Document) -> Document {
        from_bytes(&to_bytes(doc).unwrap()).unwrap()
    }

    /// The archive's entries, in order, with their contents.
    fn entries(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
        let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
        (0..archive.len())
            .map(|i| {
                let mut entry = archive.by_index(i).unwrap();
                let mut data = Vec::new();
                entry.read_to_end(&mut data).unwrap();
                (entry.name().unwrap().into_owned(), data)
            })
            .collect()
    }

    /// An archive with the given entries (to build broken files).
    fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, data) in entries {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn polygon(mesh: PolyMesh) -> ObjectKind {
        ObjectKind::polygon(MeshHandle::new(mesh))
    }

    /// A triangle and a concave hexagon (an L) sharing an edge, with UVs.
    fn ngon_mesh() -> PolyMesh {
        let points = [
            (0.0, 0.0),
            (2.0, 0.0),
            (2.0, 1.0),
            (1.0, 1.0),
            (1.0, 2.0),
            (0.0, 2.0),
            (3.0, 0.5),
        ]
        .map(|(x, y)| Vec3::new(x, y, 0.25))
        .to_vec();
        let mut mesh = PolyMesh::from_faces(points, [vec![0u32, 1, 2, 3, 4, 5], vec![1, 6, 2]]).unwrap();
        let uvs = (0..mesh.corner_count())
            .map(|c| Vec2::new(0.1, 0.01) * f32::from(u8::try_from(c).unwrap()))
            .collect();
        mesh.set_uvs(Some(uvs)).unwrap();
        mesh
    }

    #[test]
    fn the_demo_scene_round_trips_exactly() {
        let mut doc = Document::with_demo_scene();
        doc.name = "Shot 010 — ünïcode".to_owned();
        doc.time.set_settings(TimeSettings {
            fps: 25,
            start: -12,
            end: 250,
            looping: false,
        });
        assert_eq!(snapshot(&round_trip(&doc)), snapshot(&doc));
    }

    #[test]
    fn every_kind_and_awkward_values_round_trip() {
        let mut doc = Document::new("kinds");
        let mut parent = None;
        for (i, tag) in ObjectKindTag::ALL.into_iter().enumerate() {
            let mut data = NodeData::new(format!("{} \"quoted\" {i}", tag.label()), tag.default_kind());
            data.transform = Transform {
                position: Vec3::new(0.1, -1e-7, 123_456.79),
                rotation: Vec3::new(std::f32::consts::PI, -0.3, 1.0e-3),
                scale: Vec3::new(1.0 / 3.0, 2.0, 0.001),
            };
            data.display_color = [0.1, 0.2, 0.3];
            data.visible_in_editor = i % 2 == 0;
            data.visible_in_render = i % 3 == 0;
            data.enabled = i % 4 != 0;
            let id = doc.scene.insert_at(parent, usize::MAX, Node::from_data(data)).unwrap();
            parent = Some(id);
        }
        assert_eq!(snapshot(&round_trip(&doc)), snapshot(&doc));
    }

    #[test]
    fn tags_round_trip_with_their_object_references() {
        let mut doc = Document::with_demo_scene();
        let id = |doc: &Document, name: &str| doc.scene.iter().find(|(_, n)| n.data.name == name).unwrap().0;
        let (cube, sphere, camera) = (id(&doc, "Cube"), id(&doc, "Sphere"), id(&doc, "Camera"));
        doc.scene.get_mut(cube).unwrap().data.tags = vec![Tag::Phong {
            angle_limit: false,
            angle: 0.3,
        }];
        doc.scene.get_mut(sphere).unwrap().data.tags = vec![Tag::Protection {
            position: true,
            scale: false,
            rotation: true,
        }];
        // Aims at an object written before it and names one written after it as up.
        doc.scene.get_mut(camera).unwrap().data.tags = vec![Tag::Target {
            target: Some(cube),
            up: Some(sphere),
            pitch: false,
        }];
        let json = String::from_utf8(entries(&to_bytes(&doc).unwrap())[0].1.clone()).unwrap();
        assert!(json.contains("\"tags\""));

        let back = round_trip(&doc);
        let tags = |name: &str| back.scene.get(id(&back, name)).unwrap().data.tags.clone();
        assert_eq!(
            tags("Cube"),
            [Tag::Phong {
                angle_limit: false,
                angle: 0.3
            }]
        );
        assert_eq!(tags("Sphere"), doc.scene.get(sphere).unwrap().data.tags);
        assert_eq!(
            tags("Camera"),
            [Tag::Target {
                target: Some(id(&back, "Cube")),
                up: Some(id(&back, "Sphere")),
                pitch: false
            }],
            "references follow the objects"
        );
        assert!(tags("Box").is_empty());

        // A reference to an object that is not saved (deleted) is dropped.
        let detached = doc.scene.detach(sphere).unwrap();
        let back = round_trip(&doc);
        let Tag::Target { up, target, .. } = tags_of(&back, "Camera")[0] else {
            panic!("the Target tag is kept")
        };
        assert_eq!((target.is_some(), up), (true, None));
        let _ = detached;
    }

    #[test]
    fn tracks_round_trip_exactly_and_non_finite_keys_are_refused() {
        let mut doc = Document::with_demo_scene();
        let cube = doc.scene.iter().find(|(_, n)| n.data.name == "Cube").unwrap().0;
        let tracks = vec![
            Track {
                param: Param::Position(Axis::Y),
                curve: Curve::from_keys([
                    Key::new(0.0, 100.0),
                    Key {
                        interpolation: Interpolation::Linear,
                        ..Key::new(12.5, 300.0)
                    },
                    Key {
                        interpolation: Interpolation::Step,
                        tangents: Tangents::Manual {
                            left: (2.0, -10.0),
                            right: (4.0, 25.0),
                        },
                        ..Key::new(30.0, 100.0)
                    },
                ]),
            },
            Track {
                param: Param::Rotation(Axis::Z),
                curve: Curve::from_keys([Key::new(0.0, 0.0), Key::new(90.0, 1.5)]),
            },
        ];
        doc.scene.get_mut(cube).unwrap().data.tracks = tracks.clone();
        let json = String::from_utf8(entries(&to_bytes(&doc).unwrap())[0].1.clone()).unwrap();
        assert!(json.contains("\"position_y\"") && json.contains("\"rotation_b\""));
        assert!(!json.contains("\"spline\""), "defaults are left out");

        let back = round_trip(&doc);
        let loaded = back.scene.iter().find(|(_, n)| n.data.name == "Cube").unwrap().1;
        assert_eq!(loaded.data.tracks, tracks);
        assert!(
            back.scene
                .iter()
                .filter(|(_, n)| n.data.name != "Cube")
                .all(|(_, n)| n.data.tracks.is_empty())
        );

        doc.scene.get_mut(cube).unwrap().data.tracks[0]
            .curve
            .set_key(Key::new(40.0, f32::NAN));
        let err = to_bytes(&doc).unwrap_err();
        assert!(err.to_string().contains("animation key"), "{err}");
    }

    #[test]
    fn takes_round_trip_with_the_active_one_and_mains_values() {
        let mut doc = Document::with_demo_scene();
        let id = |doc: &Document, name: &str| doc.scene.iter().find(|(_, n)| n.data.name == name).unwrap().0;
        let red = doc.scene.materials_mut().insert(Material::new("Red"));
        let (cube, sphere, camera) = (id(&doc, "Cube"), id(&doc, "Sphere"), id(&doc, "Camera"));
        let main_cube = doc.scene.get(cube).unwrap().data.clone();
        let take = doc.takes.insert(Take::new("Night"));
        let other = doc.takes.insert(Take::new("Day"));
        doc.takes.render_take = Some(other);
        doc.takes.get_mut(take).unwrap().camera = Some(camera);
        doc.apply(grid4d_scene::SceneEdit::SwitchTake(Some(take))).unwrap();
        // Recorded as the editor records: an edit, then Auto Take.
        let mut data = main_cube.clone();
        data.transform.position.x = 77.0;
        data.visible_in_editor = false;
        data.display_color = [1.0, 0.0, 0.0];
        data.tags = vec![Tag::Material(MaterialTag::new(Some(red)))];
        data.kind = ObjectKind::Cube {
            size: Vec3::splat(33.0),
            segments: [2, 2, 2],
        };
        let inverse = doc.apply(grid4d_scene::SceneEdit::SetData { id: cube, data }).unwrap();
        doc.record_in_take(&[inverse]).unwrap();
        let mut render = doc.render.clone();
        render.width = 640;
        let inverse = doc.apply(grid4d_scene::SceneEdit::SetRender(render)).unwrap();
        doc.record_in_take(&[inverse]).unwrap();
        // A take that is not active: its values stay in it.
        let mut moved = doc.scene.get(sphere).unwrap().data.clone();
        moved.transform.position.y = -5.0;
        let values = vec![Override::Transform(moved.transform)];
        doc.takes.get_mut(other).unwrap().objects.insert(sphere, values);

        let mut loaded = round_trip(&doc);
        let names: Vec<&str> = loaded.takes.iter().map(|(_, t)| t.name.as_str()).collect();
        assert_eq!(names, ["Night", "Day"]);
        let (night, day) = (loaded.takes.ids()[0], loaded.takes.ids()[1]);
        assert_eq!(loaded.takes.active(), Some(night));
        assert_eq!(loaded.takes.render_take, Some(day));
        assert_eq!(loaded.takes.get(night).unwrap().camera, Some(id(&loaded, "Camera")));
        assert_eq!(loaded.takes.get(night).unwrap().override_count(), 6);
        let shown = &loaded.scene.get(id(&loaded, "Cube")).unwrap().data;
        assert_eq!(shown.transform.position.x, 77.0, "the active take's values");
        assert_eq!(loaded.render.width, 640);
        // Back to Main: Main's values, kept in the file beside the take's.
        loaded.switch_take(None);
        let main = &loaded.scene.get(id(&loaded, "Cube")).unwrap().data;
        assert_eq!(main.transform, main_cube.transform);
        assert_eq!(main.kind, main_cube.kind);
        assert!(main.visible_in_editor && main.tags.is_empty());
        assert_eq!(loaded.render, RenderSettings::default());
        loaded.switch_take(Some(day));
        assert_eq!(
            loaded
                .scene
                .get(id(&loaded, "Sphere"))
                .unwrap()
                .data
                .transform
                .position
                .y,
            -5.0
        );
        loaded.switch_take(Some(night));
        let [Tag::Material(MaterialTag { material: Some(m), .. })] =
            loaded.scene.get(id(&loaded, "Cube")).unwrap().data.tags[..]
        else {
            panic!("the take's Material tag");
        };
        assert_eq!(loaded.scene.materials().get(m).unwrap().name, "Red");
        assert!(
            !document_json(&Document::with_demo_scene()).contains("\"takes\""),
            "omitted when there are none"
        );
    }

    #[test]
    fn plugin_objects_round_trip_whether_or_not_the_plugin_is_here() {
        let mut doc = Document::new("plugins");
        let mut gear = PluginObject::new("grid4d.gear");
        gear.set("Teeth", PluginValue::Int(20));
        gear.set("Radius", PluginValue::Float(80.0));
        gear.set("Spin", PluginValue::Bool(true));
        gear.set("Mode", PluginValue::Choice(2));
        gear.set("Axis", PluginValue::Vector(Vec3::new(0.0, 1.0, 0.0)));
        gear.set("Tint", PluginValue::Color([1.0, 0.5, 0.0]));
        let mut node = Node::new("Gear", ObjectKind::Plugin(gear.clone()));
        // Its Radius (the second value) keyed: saved by name.
        let radius = Track {
            param: Param::Object(ObjectParam::Plugin(1)),
            curve: Curve::from_keys([Key::new(0.0, 80.0), Key::new(10.0, 120.0)]),
        };
        node.data.tracks.push(radius.clone());
        doc.scene.insert(node);
        let loaded = round_trip(&doc);
        let (_, node) = loaded.scene.iter().next().unwrap();
        assert_eq!(node.data.kind, ObjectKind::Plugin(gear), "type and values, in order");
        assert_eq!(node.data.tracks, [radius], "the keyed value found by its name");
        let json = document_json(&doc);
        assert!(json.contains("\"plugin\": \"grid4d.gear\""), "{json}");
        assert!(json.contains("\"int\": 20"), "{json}");
        assert!(json.contains("\"plugin\": \"Radius\""), "{json}");

        let mut broken = Document::new("broken");
        let mut bad = PluginObject::new("grid4d.gear");
        bad.set("Radius", PluginValue::Float(f32::INFINITY));
        broken.scene.insert(Node::new("Gear", ObjectKind::Plugin(bad)));
        assert!(to_bytes(&broken).is_err(), "numbers that are not finite are refused");
    }

    #[test]
    fn layers_and_objects_on_them_round_trip() {
        let mut doc = Document::with_demo_scene();
        let id = |doc: &Document, name: &str| doc.scene.iter().find(|(_, n)| n.data.name == name).unwrap().0;
        let mut lights = Layer::new("Lights", [0.9, 0.7, 0.1]);
        lights.switches.render = false;
        lights.switches.locked = true;
        let lights_id = doc.scene.layers_mut().insert(lights.clone());
        let set = Layer::new("Set", [0.1, 0.2, 0.8]);
        let set_id = doc.scene.layers_mut().insert(set.clone());
        let gone = doc.scene.layers_mut().insert(Layer::new("Gone", [0.0; 3]));
        doc.scene.layers_mut().remove(gone);
        doc.scene.get_mut(id(&doc, "Light")).unwrap().data.layer = Some(lights_id);
        doc.scene.get_mut(id(&doc, "Group")).unwrap().data.layer = Some(set_id);
        doc.scene.get_mut(id(&doc, "Cube")).unwrap().data.layer = Some(gone);

        let loaded = round_trip(&doc);
        let layers: Vec<Layer> = loaded.scene.layers().iter().map(|(_, l)| l.clone()).collect();
        assert_eq!(layers, [lights, set], "in Layer Manager order, switches and all");
        let layer_name = |name: &str| {
            let node = loaded.scene.get(id(&loaded, name)).unwrap();
            node.data
                .layer
                .map(|l| loaded.scene.layers().get(l).unwrap().name.clone())
        };
        assert_eq!(layer_name("Light").as_deref(), Some("Lights"));
        assert_eq!(layer_name("Group").as_deref(), Some("Set"));
        assert_eq!(layer_name("Box"), None, "inherited, not its own");
        assert_eq!(
            loaded.scene.effective_layer(id(&loaded, "Box")),
            loaded.scene.get(id(&loaded, "Group")).unwrap().data.layer
        );
        assert_eq!(layer_name("Cube"), None, "a link to a deleted layer is dropped");
        assert!(document_json(&doc).contains("\"layers\""));
        assert!(
            !document_json(&Document::with_demo_scene()).contains("\"layer"),
            "omitted when there are none"
        );

        doc.scene.layers_mut().get_mut(set_id).unwrap().color[1] = f32::INFINITY;
        let err = to_bytes(&doc).unwrap_err();
        assert!(err.to_string().contains("\"Set\""), "{err}");
    }

    #[test]
    fn materials_and_material_tags_round_trip() {
        let mut doc = Document::with_demo_scene();
        let mut red = Material::new("Red");
        red.color = [0.9, 0.1, 0.1];
        red.roughness = 0.25;
        red.metallic = 1.0;
        red.emission = [0.5, 0.0, 0.0];
        let red_id = doc.scene.materials_mut().insert(red.clone());
        let plain = Material::new("Mat");
        doc.scene.materials_mut().insert(plain.clone());
        let gone = doc.scene.materials_mut().insert(Material::new("Gone"));
        doc.scene.materials_mut().remove(gone);
        let id = |doc: &Document, name: &str| doc.scene.iter().find(|(_, n)| n.data.name == name).unwrap().0;
        let tag = |material| Tag::Material(MaterialTag::new(material));
        doc.scene.get_mut(id(&doc, "Cube")).unwrap().data.tags = vec![tag(Some(red_id))];
        doc.scene.get_mut(id(&doc, "Sphere")).unwrap().data.tags = vec![tag(Some(gone))];
        doc.scene.get_mut(id(&doc, "Box")).unwrap().data.tags = vec![tag(None)];

        let loaded = round_trip(&doc);
        let materials: Vec<Material> = loaded.scene.materials().iter().map(|(_, m)| m.clone()).collect();
        assert_eq!(materials, [red, plain], "in Material Manager order, exactly");
        let [Tag::Material(MaterialTag { material: Some(m), .. })] = tags_of(&loaded, "Cube")[..] else {
            panic!("the Cube keeps its Material tag");
        };
        assert_eq!(loaded.scene.materials().get(m).unwrap().name, "Red");
        assert_eq!(
            tags_of(&loaded, "Sphere"),
            [tag(None)],
            "a link to a deleted material is dropped"
        );
        assert_eq!(tags_of(&loaded, "Box"), [tag(None)]);
        assert!(document_json(&doc).contains("\"materials\""));
        assert!(
            !document_json(&Document::with_demo_scene()).contains("\"materials\""),
            "omitted when there are none"
        );

        doc.scene.materials_mut().get_mut(red_id).unwrap().roughness = f32::NAN;
        let err = to_bytes(&doc).unwrap_err();
        assert!(err.to_string().contains("\"Red\""), "{err}");
    }

    fn tags_of(doc: &Document, name: &str) -> Vec<Tag> {
        doc.scene
            .iter()
            .find(|(_, n)| n.data.name == name)
            .unwrap()
            .1
            .data
            .tags
            .clone()
    }

    #[test]
    fn subdivision_surfaces_keep_their_parameters_and_levels_are_clamped() {
        let mut doc = Document::new("sds");
        let kind = ObjectKind::SubdivisionSurface {
            viewport_levels: 3,
            render_levels: 5,
            linear_pre: 1,
            boundary: Boundary::Edge,
        };
        doc.scene.insert(Node::new("Subdivision Surface", kind.clone()));
        let loaded = round_trip(&doc);
        assert_eq!(loaded.scene.get(loaded.scene.roots()[0]).unwrap().data.kind, kind);
        let json = r#"{"type": "subdivision_surface", "viewport_levels": 1000, "render_levels": 7,
            "linear_pre": 99, "boundary": "edge_and_corner"}"#;
        let dto: KindDto = serde_json::from_str(json).unwrap();
        let ObjectKind::SubdivisionSurface {
            viewport_levels,
            render_levels,
            linear_pre,
            ..
        } = dto.into_kind(&[], "test").unwrap()
        else {
            panic!("not a generator")
        };
        assert_eq!(
            (viewport_levels, render_levels, linear_pre),
            (MAX_LEVEL, MAX_LEVEL, MAX_LEVEL)
        );
    }

    #[test]
    fn extrude_generators_round_trip_and_defaults_are_omitted() {
        let mut doc = Document::new("extrude");
        let plain = ObjectKindTag::Extrude.default_kind();
        let custom = ObjectKind::Extrude {
            extrusion: Extrusion {
                direction: ExtrudeDirection::Custom,
                offset: -35.0,
                movement: Vec3::new(1.0, 2.0, 3.0),
                subdivision: 7,
                reverse_normals: true,
                start_cap: false,
                end_cap: true,
                cap_type: CapType::Triangles,
            },
            hierarchical: true,
        };
        doc.scene.insert(Node::new("Extrude", plain.clone()));
        doc.scene.insert(Node::new("Extrude.1", custom.clone()));
        let loaded = round_trip(&doc);
        let kinds: Vec<&ObjectKind> = loaded
            .scene
            .roots()
            .iter()
            .map(|&id| &loaded.scene.get(id).unwrap().data.kind)
            .collect();
        assert_eq!(kinds, [&plain, &custom]);
        let json = serde_json::to_string(&KindDto::new(&plain, "Extrude", &mut MeshTable::default())).unwrap();
        assert_eq!(
            json,
            r#"{"type":"extrude","offset":20.0,"movement":[0.0,0.0,20.0],"subdivision":1}"#
        );
        // Never trust a subdivision a file claims.
        let json = r#"{"type": "extrude", "offset": 1.0, "movement": [0, 0, 1], "subdivision": 4000000}"#;
        let dto: KindDto = serde_json::from_str(json).unwrap();
        let ObjectKind::Extrude { extrusion, .. } = dto.into_kind(&[], "test").unwrap() else {
            panic!("not an extrude")
        };
        assert_eq!(extrusion.subdivision, Extrusion::MAX_SUBDIVISION);
    }

    #[test]
    fn lathe_generators_round_trip_and_defaults_are_omitted() {
        let mut doc = Document::new("lathe");
        let plain = ObjectKindTag::Lathe.default_kind();
        let custom = ObjectKind::Lathe(Lathe {
            angle: -3.0,
            subdivision: 9,
            movement: 120.0,
            scaling: 0.25,
            reverse_normals: true,
            start_cap: true,
            end_cap: false,
            cap_type: CapType::Triangles,
        });
        doc.scene.insert(Node::new("Lathe", plain.clone()));
        doc.scene.insert(Node::new("Lathe.1", custom.clone()));
        let loaded = round_trip(&doc);
        let kinds: Vec<&ObjectKind> = loaded
            .scene
            .roots()
            .iter()
            .map(|&id| &loaded.scene.get(id).unwrap().data.kind)
            .collect();
        assert_eq!(kinds, [&plain, &custom]);
        let json = serde_json::to_string(&KindDto::new(&plain, "Lathe", &mut MeshTable::default())).unwrap();
        assert_eq!(json, r#"{"type":"lathe","angle":6.2831855,"subdivision":24}"#);
        let json = r#"{"type": "lathe", "angle": 1.0, "subdivision": 1, "scaling": -2.0}"#;
        let dto: KindDto = serde_json::from_str(json).unwrap();
        let ObjectKind::Lathe(lathe) = dto.into_kind(&[], "test").unwrap() else {
            panic!("not a lathe")
        };
        assert_eq!((lathe.subdivision, lathe.scaling), (Lathe::MIN_SUBDIVISION, 0.0));
    }

    #[test]
    fn sweep_generators_round_trip_and_defaults_are_omitted() {
        let mut doc = Document::new("sweep");
        let plain = ObjectKindTag::Sweep.default_kind();
        let custom = ObjectKind::Sweep(Sweep {
            end_scale: 0.5,
            end_rotation: 1.5,
            start_growth: 0.1,
            end_growth: 0.9,
            parallel_movement: true,
            constant_cross_section: false,
            banking: false,
            reverse_normals: true,
            start_cap: false,
            end_cap: false,
            cap_type: CapType::Triangles,
        });
        doc.scene.insert(Node::new("Sweep", plain.clone()));
        doc.scene.insert(Node::new("Sweep.1", custom.clone()));
        let loaded = round_trip(&doc);
        let kinds: Vec<&ObjectKind> = loaded
            .scene
            .roots()
            .iter()
            .map(|&id| &loaded.scene.get(id).unwrap().data.kind)
            .collect();
        assert_eq!(kinds, [&plain, &custom]);
        let json = serde_json::to_string(&KindDto::new(&plain, "Sweep", &mut MeshTable::default())).unwrap();
        assert_eq!(json, r#"{"type":"sweep"}"#);
        let json = r#"{"type": "sweep", "end_scale": -1.0, "start_growth": -3.0, "end_growth": 7.0}"#;
        let dto: KindDto = serde_json::from_str(json).unwrap();
        let ObjectKind::Sweep(sweep) = dto.into_kind(&[], "test").unwrap() else {
            panic!("not a sweep")
        };
        assert_eq!((sweep.end_scale, sweep.start_growth, sweep.end_growth), (0.0, 0.0, 1.0));
    }

    #[test]
    fn loft_generators_round_trip_and_defaults_are_omitted() {
        let mut doc = Document::new("loft");
        let plain = ObjectKindTag::Loft.default_kind();
        let custom = ObjectKind::Loft(Loft {
            subdivision_u: 7,
            subdivision_v: 40,
            organic: true,
            per_segment: true,
            looped: true,
            linear: true,
            reverse_normals: true,
            start_cap: false,
            end_cap: false,
            cap_type: CapType::Triangles,
        });
        doc.scene.insert(Node::new("Loft", plain.clone()));
        doc.scene.insert(Node::new("Loft.1", custom.clone()));
        let loaded = round_trip(&doc);
        let kinds: Vec<&ObjectKind> = loaded
            .scene
            .roots()
            .iter()
            .map(|&id| &loaded.scene.get(id).unwrap().data.kind)
            .collect();
        assert_eq!(kinds, [&plain, &custom]);
        let json = serde_json::to_string(&KindDto::new(&plain, "Loft", &mut MeshTable::default())).unwrap();
        assert_eq!(json, r#"{"type":"loft","subdivision_u":30,"subdivision_v":10}"#);
        let json = r#"{"type": "loft", "subdivision_u": 0, "subdivision_v": 9000000}"#;
        let dto: KindDto = serde_json::from_str(json).unwrap();
        let ObjectKind::Loft(loft) = dto.into_kind(&[], "test").unwrap() else {
            panic!("not a loft")
        };
        assert_eq!(
            (loft.subdivision_u, loft.subdivision_v),
            (Loft::MIN_SUBDIVISION, Loft::MAX_SUBDIVISION)
        );
    }

    #[test]
    fn text_splines_round_trip_and_long_texts_are_cut() {
        let mut doc = Document::new("text");
        let plain = ObjectKindTag::Text.default_kind();
        let custom = ObjectKind::Text(Text {
            text: "Two\nlines".into(),
            height: 50.0,
            align: TextAlign::Center,
            horizontal_spacing: 3.0,
            vertical_spacing: -4.0,
            plane: SplinePlane::XZ,
            reverse: true,
            interpolation: SplineInterpolation::Uniform { number: 4 },
        });
        doc.scene.insert(Node::new("Text", plain.clone()));
        doc.scene.insert(Node::new("Text.1", custom.clone()));
        let loaded = round_trip(&doc);
        let kinds: Vec<&ObjectKind> = loaded
            .scene
            .roots()
            .iter()
            .map(|&id| &loaded.scene.get(id).unwrap().data.kind)
            .collect();
        assert_eq!(kinds, [&plain, &custom]);
        let json = serde_json::to_string(&KindDto::new(&plain, "Text", &mut MeshTable::default())).unwrap();
        assert_eq!(json, r#"{"type":"text","text":"Text","height":200.0}"#);
        let long = "x".repeat(Text::MAX_CHARS + 50);
        let json = format!(r#"{{"type": "text", "text": "{long}", "height": -3.0}}"#);
        let dto: KindDto = serde_json::from_str(&json).unwrap();
        let ObjectKind::Text(text) = dto.into_kind(&[], "test").unwrap() else {
            panic!("not a text")
        };
        assert_eq!((text.text.chars().count(), text.height), (Text::MAX_CHARS, 0.0));
    }

    #[test]
    fn parameter_tracks_round_trip_by_name() {
        let mut doc = Document::new("params");
        let mut node = Node::new("Cube", ObjectKindTag::Cube.default_kind());
        for (param, value) in [
            (Param::Object(ObjectParam::Size(Axis::Y)), 300.0),
            (Param::Object(ObjectParam::Radius), 5.0),
            (Param::Position(Axis::X), 1.0),
        ] {
            node.data.tracks.push(Track {
                param,
                curve: Curve::from_keys([Key::new(0.0, value), Key::new(10.0, value * 2.0)]),
            });
        }
        doc.scene.insert(node);
        let loaded = round_trip(&doc);
        let original = &doc.scene.iter().next().unwrap().1.data.tracks;
        assert_eq!(&loaded.scene.iter().next().unwrap().1.data.tracks, original);
        let json = serde_json::to_string(&ParamDto::from(Param::Object(ObjectParam::Size(Axis::Y)))).unwrap();
        assert_eq!(json, r#""size_y""#);
        let json = serde_json::to_string(&ParamDto::from(Param::Object(ObjectParam::Rise))).unwrap();
        assert_eq!(json, r#""rise""#);
    }

    #[test]
    fn bend_deformers_round_trip_and_defaults_are_omitted() {
        let mut doc = Document::new("bend");
        let plain = ObjectKindTag::Bend.default_kind();
        let custom = ObjectKind::Bend(Bend {
            size: Vec3::new(10.0, 20.0, 30.0),
            mode: DeformerMode::Unlimited,
            strength: 1.5,
            angle: -0.5,
            keep_length: true,
            origin: BendOrigin::CenterS,
        });
        doc.scene.insert(Node::new("Bend", plain.clone()));
        doc.scene.insert(Node::new("Bend.1", custom.clone()));
        let loaded = round_trip(&doc);
        let kinds: Vec<&ObjectKind> = loaded
            .scene
            .roots()
            .iter()
            .map(|&id| &loaded.scene.get(id).unwrap().data.kind)
            .collect();
        assert_eq!(kinds, [&plain, &custom]);
        let json = serde_json::to_string(&KindDto::new(&plain, "Bend", &mut MeshTable::default())).unwrap();
        assert_eq!(json, r#"{"type":"bend","size":[250.0,250.0,250.0],"strength":0.0}"#);
        let json = serde_json::to_string(&ParamDto::from(Param::Object(ObjectParam::Strength))).unwrap();
        assert_eq!(json, r#""strength""#);
    }

    #[test]
    fn twist_taper_and_bulge_round_trip() {
        let mut doc = Document::new("deformers");
        let shape = Shape {
            curvature: 0.5,
            fillet: true,
        };
        let kinds = [
            ObjectKindTag::Twist.default_kind(),
            ObjectKind::Twist(Twist {
                size: Vec3::new(1.0, 2.0, 3.0),
                mode: DeformerMode::WithinBox,
                angle: 2.0,
            }),
            ObjectKind::Taper(Taper {
                size: Vec3::splat(9.0),
                mode: DeformerMode::Unlimited,
                strength: -0.5,
                shape,
            }),
            ObjectKind::Bulge(Bulge {
                size: Vec3::splat(7.0),
                mode: DeformerMode::Limited,
                strength: 0.75,
                shape,
            }),
        ];
        for (i, kind) in kinds.iter().enumerate() {
            doc.scene.insert(Node::new(format!("d{i}"), kind.clone()));
        }
        let loaded = round_trip(&doc);
        let back: Vec<ObjectKind> = loaded
            .scene
            .roots()
            .iter()
            .map(|&id| loaded.scene.get(id).unwrap().data.kind.clone())
            .collect();
        assert_eq!(back, kinds);
        for (param, name) in [(ObjectParam::Amount, "amount"), (ObjectParam::Curvature, "curvature")] {
            let json = serde_json::to_string(&ParamDto::from(Param::Object(param))).unwrap();
            assert_eq!(json, format!("\"{name}\""));
        }
    }

    #[test]
    fn a_saved_simulation_cache_round_trips_and_bad_sizes_are_refused() {
        let mut doc = Document::new("cache");
        let ball = doc
            .scene
            .insert(Node::new("Ball", ObjectKindTag::Sphere.default_kind()));
        let cloner = doc
            .scene
            .insert(Node::new("Cloner", ObjectKindTag::Cloner.default_kind()));
        let body = |node, clone| CachedBody {
            node,
            clone,
            scale: Vec3::new(1.0, 2.0, 1.0),
        };
        let cache = SimulationCache {
            start: 5,
            bodies: vec![body(ball, None), body(cloner, Some(3))],
            poses: (0..6)
                .map(|i| {
                    let i = f32::from(u8::try_from(i).unwrap());
                    (Vec3::new(i, -i, 2.0 * i), glam::Quat::from_rotation_y(i * 0.1))
                })
                .collect(),
        };
        let bytes = crate::to_bytes_with_cache(&doc, Some(&cache)).unwrap();
        let (loaded, back) = crate::from_bytes_with_cache(&bytes).unwrap();
        let back = back.unwrap();
        let roots = loaded.scene.roots();
        assert_eq!(back.bodies, vec![body(roots[0], None), body(roots[1], Some(3))]);
        assert_eq!((back.start, back.frames()), (5, 3));
        for (a, b) in back.poses.iter().zip(&cache.poses) {
            assert!(a.0.abs_diff_eq(b.0, 1e-6) && a.1.abs_diff_eq(b.1, 1e-6));
        }
        assert!(
            crate::from_bytes_with_cache(&to_bytes(&doc).unwrap())
                .unwrap()
                .1
                .is_none()
        );

        // A file claiming more frames than it holds is refused.
        let mut archive = Archive::new(&bytes).unwrap();
        let json = String::from_utf8(archive.read(DOCUMENT_ENTRY, 1 << 20).unwrap()).unwrap();
        let lying = File::parse(&json.replace("\"frames\": 3", "\"frames\": 4")).unwrap();
        assert!(lying.into_document(&mut archive, None).is_err());
    }

    #[test]
    fn saved_rotations_stay_exact_and_never_become_nan() {
        let unit = glam::Quat::from_rotation_y(0.3);
        assert_eq!(unit_rotation(unit.x, unit.y, unit.z, unit.w), unit, "bit for bit");
        assert_eq!(unit_rotation(0.0, 0.0, 0.0, 0.0), glam::Quat::IDENTITY);
        let long = unit_rotation(0.0, 0.0, 0.0, 2.0);
        assert!((long.length() - 1.0).abs() < 1e-6);
        assert_eq!(unit_rotation(1e30, 1e30, 0.0, 0.0), glam::Quat::IDENTITY, "overflowing");
    }

    #[test]
    fn dynamics_tags_and_simulation_settings_round_trip() {
        let mut doc = Document::new("bodies");
        let rigid = Tag::Dynamics(Body::rigid());
        let custom = Tag::Dynamics(Body {
            enabled: false,
            inherit: Inherit::Compound,
            shape: CollisionShape::ConvexHull,
            bounce: 0.9,
            friction: 0.1,
            mass_mode: MassMode::CustomMass,
            mass: 12.5,
            rotational_mass: 2.0,
            linear_damping: 0.25,
            initial_velocity: Vec3::new(100.0, 0.0, -50.0),
            initial_spin: Vec3::new(0.0, 3.0, 0.0),
            ..Body::collider()
        });
        let mut ball = NodeData::new("Ball", ObjectKindTag::Sphere.default_kind());
        ball.tags.push(rigid.clone());
        let mut floor = NodeData::new("Floor", ObjectKindTag::Plane.default_kind());
        floor.tags.push(custom.clone());
        doc.scene.insert(Node::from_data(ball));
        doc.scene.insert(Node::from_data(floor));
        doc.simulation = SimulationSettings {
            enabled: false,
            gravity: Vec3::new(0.0, -100.0, 0.0),
            steps_per_frame: 10,
            solver_iterations: 8,
            time_scale: 0.5,
            memory_limit_mb: 1024,
        };
        let loaded = round_trip(&doc);
        let roots = loaded.scene.roots();
        let tags = |i: usize| loaded.scene.get(roots[i]).unwrap().data.tags.clone();
        assert_eq!((tags(0), tags(1)), (vec![rigid.clone()], vec![custom]));
        assert_eq!(loaded.simulation, doc.simulation);
        assert_eq!(
            round_trip(&Document::new("plain")).simulation,
            SimulationSettings::default()
        );

        let refs = Refs {
            objects: HashMap::new(),
            materials: HashMap::new(),
            layers: HashMap::new(),
            textures: HashMap::new(),
        };
        let json = serde_json::to_string(&TagDto::new(&rigid, &refs)).unwrap();
        assert_eq!(
            json,
            r#"{"type":"dynamics","motion":"dynamic","bounce":0.5,"friction":0.3}"#
        );
        assert!(
            SimulationDto::new(&SimulationSettings::default()).is_none(),
            "defaults are left out"
        );
        let bad = SimulationDto {
            enabled: true,
            gravity: [f32::NAN, 0.0, 0.0],
            steps_per_frame: 5,
            solver_iterations: 4,
            time_scale: 1.0,
            memory_limit_mb: 256,
        };
        assert!(bad.into_settings().is_err());
    }

    #[test]
    fn effectors_and_a_cloners_effector_list_round_trip() {
        use grid4d_geom::effector::{Effector, Falloff, FalloffShape};
        let mut doc = Document::new("effectors");
        let plain = ObjectKindTag::PlainEffector.default_kind();
        let random = ObjectKind::RandomEffector(Effector {
            strength: 0.75,
            use_position: false,
            use_scale: true,
            scale: Vec3::new(0.5, -0.25, 1.0),
            use_rotation: true,
            rotation: Vec3::new(0.1, 0.2, 0.3),
            use_color: true,
            seed: 99,
            falloff: Falloff {
                shape: FalloffShape::Box,
                size: Vec3::new(100.0, 200.0, 300.0),
                soft: 0.25,
            },
            ..Effector::random()
        });
        let a = doc.scene.insert(Node::new("Plain", plain.clone()));
        let b = doc.scene.insert(Node::new("Random", random.clone()));
        doc.scene.insert(Node::new(
            "Cloner",
            ObjectKind::Cloner {
                cloner: Cloner::default(),
                object: None,
                effectors: vec![b, a],
            },
        ));
        let loaded = round_trip(&doc);
        let roots = loaded.scene.roots();
        let kind = |i: usize| &loaded.scene.get(roots[i]).unwrap().data.kind;
        assert_eq!(kind(0), &plain);
        assert_eq!(kind(1), &random);
        let ObjectKind::Cloner { effectors, .. } = kind(2) else {
            panic!("not a Cloner")
        };
        assert_eq!(effectors, &[roots[1], roots[0]], "in order");
        let json = serde_json::to_string(&KindDto::new(&plain, "Plain", &mut MeshTable::default())).unwrap();
        assert_eq!(
            json,
            r#"{"type":"plain_effector","strength":1.0,"position":[0.0,100.0,0.0],"seed":12345}"#
        );
    }

    #[test]
    fn object_mode_cloners_keep_their_object_link() {
        use grid4d_geom::cloner::{Cloner, Distribution, Layout, Surface};
        let mut doc = Document::new("surface");
        let sphere = doc
            .scene
            .insert(Node::new("Sphere", ObjectKindTag::Sphere.default_kind()));
        let surface = Layout::Object(Surface {
            distribution: Distribution::PolygonCenter,
            align: false,
        });
        doc.scene.insert(Node::new(
            "Cloner",
            ObjectKind::Cloner {
                cloner: Cloner {
                    layout: surface,
                    ..Cloner::default()
                },
                object: Some(sphere),
                effectors: Vec::new(),
            },
        ));
        let loaded = round_trip(&doc);
        let roots = loaded.scene.roots();
        let ObjectKind::Cloner { cloner, object, .. } = &loaded.scene.get(roots[1]).unwrap().data.kind else {
            panic!("not a Cloner")
        };
        assert_eq!(cloner.layout, surface);
        assert_eq!(*object, Some(roots[0]), "the link points at the loaded sphere");
    }

    #[test]
    fn cloners_round_trip_and_counts_are_clamped() {
        use grid4d_geom::cloner::{CloneOrder, Cloner, GridArray, Layout, Linear, Radial, StepMode};
        let mut doc = Document::new("clones");
        let kinds = [
            ObjectKindTag::Cloner.default_kind(),
            ObjectKind::cloner(Cloner {
                layout: Layout::Linear(Linear {
                    count: 7,
                    offset: 2,
                    mode: StepMode::EndPoint,
                    position: Vec3::new(1.0, 2.0, 3.0),
                    rotation: Vec3::new(0.1, 0.2, 0.3),
                    scale: Vec3::new(0.5, 1.5, 1.0),
                }),
                clones: CloneOrder::Random,
                seed: 42,
                reset_coordinates: false,
            }),
            ObjectKind::cloner(Cloner {
                layout: Layout::Radial(Radial {
                    count: 12,
                    plane: SplinePlane::ZY,
                    align: false,
                    start_angle: 0.5,
                    offset: 0.25,
                    ..Radial::default()
                }),
                ..Cloner::default()
            }),
            ObjectKind::cloner(Cloner {
                layout: Layout::Grid(GridArray {
                    count: [2, 3, 4],
                    mode: StepMode::EndPoint,
                    size: Vec3::new(10.0, 20.0, 30.0),
                }),
                ..Cloner::default()
            }),
        ];
        for (i, kind) in kinds.iter().enumerate() {
            doc.scene.insert(Node::new(format!("Cloner.{i}"), kind.clone()));
        }
        let loaded = round_trip(&doc);
        let back: Vec<&ObjectKind> = loaded
            .scene
            .roots()
            .iter()
            .map(|&id| &loaded.scene.get(id).unwrap().data.kind)
            .collect();
        assert_eq!(back, kinds.iter().collect::<Vec<_>>());
        let json = serde_json::to_string(&KindDto::new(&kinds[0], "Cloner", &mut MeshTable::default())).unwrap();
        assert_eq!(
            json,
            r#"{"type":"cloner","layout":{"mode":"linear","count":3,"position":[0.0,250.0,0.0],"scale":[1.0,1.0,1.0]},"seed":1234567}"#
        );
        let huge = r#"{"type": "cloner", "seed": 1, "layout": {"mode": "grid", "count": [4000000000, 0, 5], "size": [1, 1, 1]}}"#;
        let dto: KindDto = serde_json::from_str(huge).unwrap();
        let ObjectKind::Cloner { cloner, .. } = dto.into_kind(&[], "test").unwrap() else {
            panic!()
        };
        assert_eq!(
            cloner.layout,
            Layout::Grid(GridArray {
                count: [Cloner::MAX_COUNT, 1, 5],
                mode: StepMode::PerStep,
                size: Vec3::ONE,
            })
        );
        assert_eq!(cloner.count(), Cloner::MAX_COUNT as usize);
    }

    #[test]
    fn ffd_lattices_round_trip_and_bad_counts_are_refused() {
        let mut doc = Document::new("ffd");
        let plain = ObjectKindTag::Ffd.default_kind();
        let mut moved = ObjectKind::ffd(Ffd {
            grid: [2, 4, 3],
            mode: DeformerMode::Unlimited,
            ..Ffd::default()
        });
        if let ObjectKind::Ffd {
            lattice, components, ..
        } = &mut moved
        {
            *lattice = lattice.edited(|s| s.points[5] += Vec3::X * 12.0);
            components.make_mut().selected.points.insert(5);
        }
        doc.scene.insert(Node::new("FFD", plain.clone()));
        doc.scene.insert(Node::new("FFD.1", moved.clone()));
        let loaded = round_trip(&doc);
        let kinds: Vec<&ObjectKind> = loaded
            .scene
            .roots()
            .iter()
            .map(|&id| &loaded.scene.get(id).unwrap().data.kind)
            .collect();
        assert_eq!(kinds, [&plain, &moved]);
        let json = serde_json::to_string(&KindDto::new(&plain, "FFD", &mut MeshTable::default())).unwrap();
        assert_eq!(json, r#"{"type":"ffd","size":[250.0,250.0,250.0],"grid":[3,3,3]}"#);
        let bad = r#"{"type": "ffd", "size": [1, 1, 1], "grid": [2, 2, 2], "lattice": [[0, 0, 0]]}"#;
        let dto: KindDto = serde_json::from_str(bad).unwrap();
        assert!(dto.into_kind(&[], "test").is_err());
    }

    #[test]
    fn light_types_and_details_round_trip_and_defaults_are_omitted() {
        let mut doc = Document::new("lights");
        let spot = ObjectKind::Light {
            color: [1.0, 0.8, 0.6],
            intensity: 3.0,
            light_type: LightType::Spot,
            details: LightDetails {
                use_inner: false,
                inner_angle_deg: 10.0,
                outer_angle_deg: 70.0,
                falloff: Falloff::InverseSquare,
                radius: 250.0,
            },
            shadow: LightShadow::Raytraced,
        };
        let sun = ObjectKind::Light {
            color: [1.0; 3],
            intensity: 1.0,
            light_type: LightType::Infinite,
            details: LightDetails::default(),
            shadow: LightShadow::Area,
        };
        for (name, kind) in [("Spot", spot), ("Sun", sun), ("Bulb", ObjectKind::light([1.0; 3], 1.0))] {
            doc.scene.insert(Node::new(name, kind));
        }
        assert_eq!(snapshot(&round_trip(&doc)), snapshot(&doc));

        // An Omni light with the default details writes nothing new.
        let dto = KindDto::new(&ObjectKind::light([1.0; 3], 1.0), "Bulb", &mut MeshTable::default());
        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"type": "light", "color": [1.0, 1.0, 1.0], "intensity": 1.0})
        );

        // Angles and radii out of range are brought into it; non-finite ones refused.
        let json = r#"{"type": "light", "color": [1, 1, 1], "intensity": 1, "light_type": "spot",
            "details": {"use_inner": true, "inner_angle_deg": -5, "outer_angle_deg": 400,
            "falloff": "linear", "radius": 0}}"#;
        let dto: KindDto = serde_json::from_str(json).unwrap();
        let ObjectKind::Light { details, .. } = dto.into_kind(&[], "test").unwrap() else {
            panic!("not a light")
        };
        assert_eq!(
            (details.inner_angle_deg, details.outer_angle_deg, details.radius),
            (0.0, LightDetails::MAX_ANGLE_DEG, LightDetails::MIN_RADIUS)
        );
    }

    #[test]
    fn segment_counts_round_trip() {
        let mut doc = Document::new("segments");
        doc.scene.insert(Node::new(
            "fine",
            ObjectKind::Cube {
                size: Vec3::splat(10.0),
                segments: [3, 7, 11],
            },
        ));
        doc.scene.insert(Node::new(
            "torus",
            ObjectKind::Torus {
                ring_radius: 3.5,
                pipe_radius: 1.25,
                ring_segments: 64,
                pipe_segments: 5,
            },
        ));
        assert_eq!(snapshot(&round_trip(&doc)), snapshot(&doc));
    }

    #[test]
    fn meshes_round_trip_exactly_and_shared_meshes_are_stored_once() {
        let mut doc = Document::new("meshes");
        let shared = MeshHandle::new(grid4d_geom::primitives::sphere(50.0, 16));
        for name in ["a", "b"] {
            doc.scene.insert(Node::new(name, ObjectKind::polygon(shared.clone())));
        }
        doc.scene.insert(Node::new("ngons", polygon(ngon_mesh())));
        doc.scene.insert(Node::new("empty", polygon(PolyMesh::new())));

        let bytes = to_bytes(&doc).unwrap();
        let names: Vec<String> = entries(&bytes).into_iter().map(|(name, _)| name).collect();
        assert_eq!(names, ["document.json", "meshes/0.bin", "meshes/1.bin", "meshes/2.bin"]);

        let loaded = from_bytes(&bytes).unwrap();
        assert_eq!(snapshot(&loaded), snapshot(&doc));
        let mesh_of = |name: &str| {
            let node = loaded.scene.iter().find(|(_, n)| n.data.name == name).unwrap().1;
            let ObjectKind::Polygon { mesh, .. } = &node.data.kind else {
                panic!()
            };
            mesh.id()
        };
        assert_eq!(mesh_of("a"), mesh_of("b"), "still one shared mesh after loading");
        assert_ne!(mesh_of("a"), mesh_of("ngons"));
    }

    #[test]
    fn component_selections_round_trip_and_are_omitted_when_empty() {
        let mut state = ComponentState::default();
        state.selected.points.extend([0, 1, 2, 6]);
        state.selected.edges.insert_range(3..7);
        state.selected.polygons.insert(1);
        state.hidden.points.insert(5);
        let mut doc = Document::new("selections");
        let kind = ObjectKind::Polygon {
            mesh: MeshHandle::new(ngon_mesh()),
            components: Components::new(state),
        };
        doc.scene.insert(Node::new("ngons", kind));
        doc.scene.insert(Node::new("plain", polygon(ngon_mesh())));
        assert_eq!(snapshot(&round_trip(&doc)), snapshot(&doc));

        let json = document_json(&doc);
        assert!(json.contains("\"points\": [\n"), "ranges are written: {json}");
        assert_eq!(json.matches("\"selected\"").count(), 1, "omitted for the plain object");
        assert_eq!(json.matches("\"hidden\"").count(), 1);
    }

    #[test]
    fn selections_outside_the_mesh_are_rejected() {
        let mut doc = Document::new("mesh");
        doc.scene.insert(Node::new("ngons", polygon(ngon_mesh())));
        let json = document_json(&doc);
        let mesh = entries(&to_bytes(&doc).unwrap()).swap_remove(1).1;
        let with = |selection: &str| {
            let json = json.replacen("\"mesh\": 0", &format!("\"mesh\": 0, {selection}"), 1);
            from_bytes(&archive(&[(DOCUMENT_ENTRY, json.as_bytes()), ("meshes/0.bin", &mesh)]))
        };
        assert!(with("\"selected\": { \"points\": [[0, 6]] }").is_ok());
        for bad in [
            "\"selected\": { \"points\": [[0, 7]] }",
            "\"selected\": { \"edges\": [[9, 9]] }",
            "\"hidden\": { \"polygons\": [[1, 0]] }",
        ] {
            assert!(matches!(with(bad), Err(IoError::Invalid(_))), "{bad}");
        }
    }

    #[test]
    fn saving_twice_gives_the_same_bytes() {
        let mut doc = Document::with_demo_scene();
        doc.scene.insert(Node::new("ngons", polygon(ngon_mesh())));
        assert_eq!(to_bytes(&doc).unwrap(), to_bytes(&doc).unwrap());
    }

    #[test]
    fn deep_and_wide_hierarchies_round_trip() {
        let mut doc = Document::new("deep");
        let mut parent = None;
        for depth in 0..500 {
            let id = doc
                .scene
                .insert_at(
                    parent,
                    usize::MAX,
                    Node::new(format!("level {depth}"), ObjectKind::Null),
                )
                .unwrap();
            for sibling in 0..3 {
                doc.scene
                    .insert_at(
                        Some(id),
                        0,
                        Node::new(format!("leaf {depth}.{sibling}"), ObjectKind::Null),
                    )
                    .unwrap();
            }
            parent = Some(id);
        }
        let loaded = round_trip(&doc);
        assert_eq!(loaded.scene.len(), 2000);
        assert_eq!(snapshot(&loaded), snapshot(&doc));
    }

    #[test]
    fn detached_nodes_are_not_saved() {
        let mut doc = Document::with_demo_scene();
        let group = doc.scene.iter().find(|(_, n)| n.data.name == "Group").unwrap().0;
        doc.scene.detach(group).unwrap();
        let loaded = round_trip(&doc);
        assert_eq!(loaded.scene.len(), doc.scene.len());
        assert!(loaded.scene.detached_roots().is_empty());
        assert_eq!(snapshot(&loaded), snapshot(&doc));
    }

    #[test]
    fn non_finite_values_refuse_to_save() {
        let mut doc = Document::with_demo_scene();
        let cube = doc.scene.roots()[0];
        doc.scene.get_mut(cube).unwrap().data.transform.scale.y = f32::NAN;
        let err = to_bytes(&doc).unwrap_err();
        assert!(matches!(err, IoError::Invalid(_)), "{err:?}");
        assert!(err.to_string().contains("\"Cube\""), "{err}");

        let mut mesh = ngon_mesh();
        mesh.points_mut()[3].x = f32::INFINITY;
        let mut doc = Document::new("bad mesh");
        doc.scene.insert(Node::new("Spiky", polygon(mesh)));
        let err = to_bytes(&doc).unwrap_err();
        assert!(err.to_string().contains("\"Spiky\""), "{err}");
    }

    /// The `document.json` of `doc`'s archive, for tests that edit it.
    fn document_json(doc: &Document) -> String {
        let (name, json) = entries(&to_bytes(doc).unwrap()).swap_remove(0);
        assert_eq!(name, DOCUMENT_ENTRY);
        String::from_utf8(json).unwrap()
    }

    #[test]
    fn textures_are_linked_relative_to_the_folder_or_embedded() {
        use std::{path::Path, sync::Arc};

        use grid4d_scene::{ColorSpace, Texture, TextureSource};

        let folder = std::env::temp_dir().join("grid4d-textures").join("shot");
        let inside = folder.join("maps").join("wood.jpg");
        let outside = std::env::temp_dir().join("elsewhere").join("sky.hdr");
        let mut doc = Document::new("textures");
        let textures = doc.scene.textures_mut();
        textures.insert(Texture::linked(inside.clone()));
        textures.insert(Texture::linked(outside.clone()));
        textures.insert(Texture::linked(Path::new("relative").join("brick.png")));
        let logo = Texture {
            name: "Logo".to_owned(),
            source: TextureSource::Embedded {
                file_name: "Company Logo.PNG".to_owned(),
                bytes: Arc::from(b"picture bytes".as_slice()),
            },
            color_space: ColorSpace::Srgb,
        };
        textures.insert(logo.clone());
        textures.insert(Texture {
            source: TextureSource::Embedded {
                file_name: "no extension".to_owned(),
                bytes: Arc::from(b"more".as_slice()),
            },
            ..logo
        });

        let bytes = write(&doc, None, Some(&folder)).unwrap();
        let files = entries(&bytes);
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, [DOCUMENT_ENTRY, "textures/3.png", "textures/4.bin"]);
        assert_eq!(files[1].1, b"picture bytes");
        let json = String::from_utf8(files[0].1.clone()).unwrap();
        assert!(
            json.contains("\"file\": \"maps/wood.jpg\""),
            "relative inside the folder: {json}"
        );
        assert!(json.contains("\"file\": \"relative/brick.png\""), "{json}");
        assert!(json.contains("\"color_space\": \"linear\""), "the HDR's: {json}");

        // Read in another folder: the relative ones follow it.
        let moved = std::env::temp_dir().join("grid4d-textures").join("moved");
        let (back, _) = crate::from_bytes_at(&bytes, &moved).unwrap();
        let sources: Vec<TextureSource> = back.scene.textures().iter().map(|(_, t)| t.source.clone()).collect();
        assert_eq!(sources[0], TextureSource::Linked(moved.join("maps").join("wood.jpg")));
        assert_eq!(sources[1], TextureSource::Linked(outside));
        assert_eq!(
            sources[2],
            TextureSource::Linked(moved.join("relative").join("brick.png"))
        );
        assert_eq!(
            &sources[3..],
            [
                doc.scene.textures().iter().nth(3).unwrap().1.source.clone(),
                doc.scene.textures().iter().nth(4).unwrap().1.source.clone()
            ]
        );
        // Read back in the same folder: the same paths (a relative one now
        // taken from the folder).
        let (same, _) = crate::from_bytes_at(&bytes, &folder).unwrap();
        let paths: Vec<TextureSource> = same.scene.textures().iter().map(|(_, t)| t.source.clone()).collect();
        assert_eq!(paths[0], TextureSource::Linked(inside));
        assert_eq!(
            paths[2],
            TextureSource::Linked(folder.join("relative").join("brick.png"))
        );
        assert_eq!(
            write(&same, None, Some(&folder)).unwrap(),
            bytes,
            "saved again: the same file"
        );

        // Neither or both sources, or an embedded entry missing: errors.
        let ok = String::from_utf8(entries(&to_bytes(&doc).unwrap()).swap_remove(0).1).unwrap();
        let load = |json: &str| from_bytes(&archive(&[(DOCUMENT_ENTRY, json.as_bytes())]));
        let both = ok.replacen(
            "\"embedded\": \"no extension\"",
            "\"embedded\": \"no extension\", \"file\": \"x.png\"",
            1,
        );
        assert!(matches!(load(&both), Err(IoError::Invalid(_))));
        let neither = ok.replacen("\"file\": \"maps", "\"elsewhere\": \"maps", 1);
        assert!(matches!(load(&neither), Err(IoError::Invalid(_))));
        assert!(matches!(load(&ok), Err(IoError::Invalid(message)) if message.contains("textures/3.png")));
    }

    #[test]
    fn malformed_documents_give_clear_errors() {
        let ok = document_json(&Document::with_demo_scene());
        let load = |json: &str| from_bytes(&archive(&[(DOCUMENT_ENTRY, json.as_bytes())]));

        assert!(matches!(
            load("{ \"format\": \"grid4d\", "),
            Err(IoError::Parse { line: 1, .. })
        ));
        for not_ours in [
            "{}",
            "{\"format\": \"gltf\", \"version\": 1}",
            "{\"format\": \"grid4d\"}",
        ] {
            assert!(matches!(load(not_ours), Err(IoError::NotADocument)), "{not_ours}");
        }
        let newer = ok.replacen("\"version\": 5", "\"version\": 7", 1);
        assert!(matches!(
            load(&newer),
            Err(IoError::UnsupportedVersion {
                found: 7,
                newest: FORMAT_VERSION
            })
        ));
        // The same document as version 4 or 3 (what it holds, they hold too)
        // loads the same; their errors read without a position.
        let as_v4 = ok.replacen("\"version\": 5", "\"version\": 4", 1);
        assert_eq!(
            to_bytes(&load(&as_v4).unwrap()).unwrap(),
            to_bytes(&load(&ok).unwrap()).unwrap()
        );
        let v4_teapot = as_v4.replacen("\"type\": \"cube\"", "\"type\": \"teapot\"", 1);
        assert!(matches!(load(&v4_teapot), Err(IoError::Parse { line: 0, .. })));
        let older = ok.replacen("\"version\": 5", "\"version\": 3", 1);
        assert_eq!(
            to_bytes(&load(&older).unwrap()).unwrap(),
            to_bytes(&load(&ok).unwrap()).unwrap()
        );
        let older_teapot = older.replacen("\"type\": \"cube\"", "\"type\": \"teapot\"", 1);
        let err = load(&older_teapot).unwrap_err();
        assert!(matches!(err, IoError::Parse { line: 0, .. }), "{err:?}");
        assert!(err.to_string().starts_with("unknown variant `teapot`"), "{err}");
        let missing_field = ok.replacen("\"enabled\": true", "\"enabeld\": true", 1);
        assert!(matches!(load(&missing_field), Err(IoError::Parse { .. })));
        let unknown_kind = ok.replacen("\"type\": \"cube\"", "\"type\": \"teapot\"", 1);
        assert!(matches!(load(&unknown_kind), Err(IoError::Parse { .. })));
        let overflow = ok.replacen("\"radius\": 80.0", "\"radius\": 1e300", 1);
        assert!(matches!(load(&overflow), Err(IoError::Invalid(_))), "f32 overflow");
    }

    #[test]
    fn broken_archives_and_meshes_are_rejected() {
        let mut doc = Document::new("mesh");
        doc.scene.insert(Node::new("ngons", polygon(ngon_mesh())));
        let json = document_json(&doc);
        let good = entries(&to_bytes(&doc).unwrap()).swap_remove(1).1;
        let load = |json: &str, mesh: Option<&[u8]>| {
            let mut files = vec![(DOCUMENT_ENTRY, json.as_bytes())];
            files.extend(mesh.map(|m| ("meshes/0.bin", m)));
            from_bytes(&archive(&files))
        };
        assert!(load(&json, Some(&good)).is_ok());

        let invalid = |result: Result<Document, IoError>, what: &str| {
            assert!(matches!(result, Err(IoError::Invalid(_))), "{what}: {result:?}");
        };
        invalid(load(&json, None), "mesh file missing");
        invalid(load(&json, Some(&good[..good.len() - 4])), "mesh file too short");
        invalid(from_bytes(&archive(&[("other.json", b"{}")])), "no document.json");
        let mut out_of_range = good.clone();
        let corner = 7 * 12 + 2 * 4; // the first corner index, after points and face sizes
        out_of_range[corner..corner + 4].copy_from_slice(&99u32.to_le_bytes());
        invalid(load(&json, Some(&out_of_range)), "corner refers to a missing point");
        let mut bad_sizes = good.clone();
        bad_sizes[7 * 12..7 * 12 + 4].copy_from_slice(&5u32.to_le_bytes());
        invalid(load(&json, Some(&bad_sizes)), "corner counts do not add up");
        let mut nan = good.clone();
        nan[0..4].copy_from_slice(&f32::NAN.to_le_bytes());
        invalid(load(&json, Some(&nan)), "non-finite point");
        let wrong_index = json.replacen("\"mesh\": 0", "\"mesh\": 5", 1);
        invalid(load(&wrong_index, Some(&good)), "object uses a missing mesh");

        let mut truncated = to_bytes(&doc).unwrap();
        truncated.truncate(truncated.len() / 2);
        invalid(from_bytes(&truncated), "half an archive");
        assert!(matches!(from_bytes(&[0xff, 0xfe, 0x00]), Err(IoError::NotADocument)));
    }

    #[test]
    fn time_settings_are_validated_on_load() {
        let json = document_json(&Document::new("t")).replacen("\"fps\": 30", "\"fps\": 0", 1);
        let doc = from_bytes(&archive(&[(DOCUMENT_ENTRY, json.as_bytes())])).unwrap();
        assert_eq!(doc.time.fps(), 1);
    }

    #[test]
    fn render_settings_round_trip_and_are_left_out_while_default() {
        assert!(!document_json(&Document::with_demo_scene()).contains("\"render\""));

        let mut doc = Document::new("t");
        doc.render = RenderSettings {
            width: 1920,
            height: 1080,
            lock_ratio: true,
            frames: FrameRange::Manual { from: 5, to: 40 },
            frame_step: 5,
            samples: 128,
            bounces: 4,
            auto_light: false,
            save: false,
            output: "renders/shot 010".to_owned(),
            alpha: true,
            format: ImageFormat::Exr,
            naming: FrameNaming::NameDot0000,
        };
        assert_eq!(round_trip(&doc).render, doc.render);
        doc.render.frames = FrameRange::All;
        assert_eq!(round_trip(&doc).render, doc.render);
    }

    /// Files written in step 8.2 have no format or naming: the defaults.
    #[test]
    fn render_settings_without_format_and_naming_load() {
        let mut doc = Document::new("t");
        doc.render.samples = 64;
        let json = document_json(&doc);
        assert!(!json.contains("\"format\": \"png\"") && !json.contains("\"naming\""));
        let loaded = from_bytes(&archive(&[(DOCUMENT_ENTRY, json.as_bytes())])).unwrap();
        assert_eq!(loaded.render, doc.render);
        assert_eq!(
            (loaded.render.format, loaded.render.naming),
            (ImageFormat::Png, FrameNaming::Name0000)
        );
    }

    #[test]
    fn render_settings_are_validated_on_load() {
        let mut doc = Document::new("t");
        doc.render.samples = 64;
        let json = document_json(&doc)
            .replacen("\"samples\": 64", "\"samples\": 0", 1)
            .replacen("\"width\": 1280", "\"width\": 999999", 1);
        let loaded = from_bytes(&archive(&[(DOCUMENT_ENTRY, json.as_bytes())])).unwrap();
        assert_eq!(loaded.render.samples, 1);
        assert_eq!(loaded.render.width, RenderSettings::MAX_SIZE);
    }

    #[test]
    fn spline_primitives_round_trip_and_leave_defaults_out() {
        use grid4d_geom::spline::{ArcType, Interpolation as Points, SplinePlane, SplinePrimitive, SplineShape};
        let shapes = [
            SplineShape::Circle {
                radius: 150.0,
                ellipse: Some(80.0),
                ring: Some(40.0),
            },
            SplineShape::Rectangle {
                width: 300.0,
                height: 120.0,
                rounding: Some(20.0),
            },
            SplineShape::Arc {
                kind: ArcType::Ring,
                radius: 200.0,
                inner_radius: 90.0,
                start: -0.5,
                end: 2.0,
            },
            SplineShape::NSide {
                radius: 100.0,
                sides: 7,
                rounding: None,
            },
            SplineShape::Star {
                inner_radius: 30.0,
                outer_radius: 90.0,
                twist: 0.3,
                points: 5,
            },
            SplineShape::Helix {
                start_radius: 100.0,
                start_angle: 0.0,
                end_radius: 50.0,
                end_angle: 12.0,
                radial_bias: 0.3,
                height: 400.0,
                height_bias: 0.7,
                subdivision: 64,
            },
            SplineShape::Flower {
                inner_radius: 60.0,
                outer_radius: 120.0,
                petals: 9,
            },
        ];
        let mut doc = Document::new("t");
        for (i, shape) in shapes.into_iter().enumerate() {
            let primitive = SplinePrimitive {
                shape,
                plane: [SplinePlane::XY, SplinePlane::ZY, SplinePlane::XZ][i % 3],
                reverse: i % 2 == 1,
                interpolation: [
                    Points::default(),
                    Points::Natural { number: 3 },
                    Points::Uniform { number: 12 },
                    Points::Subdivided {
                        angle: 0.1,
                        max_length: 7.5,
                    },
                    Points::None,
                ][i % 5],
            };
            doc.scene
                .insert(Node::new(format!("s{i}"), ObjectKind::SplinePrimitive(primitive)));
        }
        let loaded = round_trip(&doc);
        let kinds = |d: &Document| d.scene.iter().map(|(_, n)| n.data.kind.clone()).collect::<Vec<_>>();
        assert_eq!(kinds(&loaded), kinds(&doc));

        let mut plain = Document::new("t");
        plain
            .scene
            .insert(Node::new("Circle", ObjectKindTag::Circle.default_kind()));
        let json = document_json(&plain);
        assert!(json.contains("\"type\": \"spline\"") && json.contains("\"shape\": \"circle\""));
        assert!(!json.contains("plane") && !json.contains("reverse") && !json.contains("intermediate_points"));
        assert_eq!(kinds(&round_trip(&plain)), kinds(&plain));
    }

    #[test]
    fn spline_counts_are_clamped_on_load() {
        let mut doc = Document::new("t");
        doc.scene.insert(Node::new("Star", ObjectKindTag::Star.default_kind()));
        let json = document_json(&doc).replacen("\"points\": 8", "\"points\": 4000000000", 1);
        let loaded = from_bytes(&archive(&[(DOCUMENT_ENTRY, json.as_bytes())])).unwrap();
        let ObjectKind::SplinePrimitive(star) = &loaded.scene.iter().next().unwrap().1.data.kind else {
            panic!()
        };
        let SplineShape::Star { points, .. } = star.shape else {
            panic!()
        };
        assert_eq!(points, SplineShape::MAX_SIDES);
    }

    #[test]
    fn editable_splines_round_trip_with_their_selection() {
        use grid4d_geom::spline::{Interpolation as Points, Spline, SplineType};
        let mut spline = Spline::new(
            SplineType::Bezier,
            vec![Vec3::ZERO, Vec3::new(100.0, 50.0, 0.0), Vec3::new(200.0, 0.0, 30.0)],
            false,
        );
        spline.smooth_tangents();
        spline.append(Spline::new(SplineType::Linear, vec![Vec3::X, Vec3::Y], true));
        let mut state = ComponentState::default();
        state.selected.points.extend([1, 3]);
        state.hidden.points.insert(4);
        let mut doc = Document::new("t");
        doc.scene.insert(Node::new(
            "Spline",
            ObjectKind::Spline {
                spline: SplineHandle::new(spline),
                interpolation: Points::Uniform { number: 4 },
                components: Components::new(state),
            },
        ));
        let kinds = |d: &Document| d.scene.iter().map(|(_, n)| n.data.kind.clone()).collect::<Vec<_>>();
        assert_eq!(kinds(&round_trip(&doc)), kinds(&doc));

        // A linear spline writes no tangents.
        let mut plain = Document::new("t");
        plain.scene.insert(Node::new(
            "Line",
            ObjectKind::Spline {
                spline: SplineHandle::new(Spline::new(SplineType::Linear, vec![Vec3::ZERO, Vec3::X], false)),
                interpolation: Points::default(),
                components: Components::default(),
            },
        ));
        let json = document_json(&plain);
        assert!(json.contains("\"type\": \"editable_spline\"") && !json.contains("tangents"));
        assert_eq!(kinds(&round_trip(&plain)), kinds(&plain));

        // Segments that do not add up to the points are an error.
        let broken = json.replacen("\"count\": 2", "\"count\": 3", 1);
        assert!(from_bytes(&archive(&[(DOCUMENT_ENTRY, broken.as_bytes())])).is_err());
    }
}
