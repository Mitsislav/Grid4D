//! Format version 3 (current): a zip archive. `document.json` holds version 2's
//! document plus polygon objects (under the tag `"grid4d"`), and each mesh is a
//! binary file `meshes/<index>.bin`. Never change these types once released: add
//! a `v4` module and migrate instead.
//!
//! A mesh file is little-endian, with the sizes given in `document.json`:
//! `points × 3` f32 (x, y, z), then `faces` u32 corner counts, then `corners` u32
//! point indices (faces counter-clockwise, in order), then — if `uvs` —
//! `corners × 2` f32 UVs.
//!
//! Until a release freezes it, version 3 may only grow **additively**: optional
//! fields that are omitted when empty (component selections, step 4.1), new
//! kinds. Every earlier v3 fixture must keep loading.

use std::{
    collections::HashMap,
    io::{Cursor, Write as _},
};

use glam::{Vec2, Vec3};
use grid4d_geom::{
    BitSet, ComponentSet, PolyMesh,
    deform::{Bend, BendOrigin, DeformerMode},
    spline::{
        ArcType, Interpolation as SplineInterpolation, Segment, Spline, SplinePlane, SplinePrimitive, SplineShape,
        SplineType, Text, TextAlign,
    },
    subdivision::{Boundary, MAX_LEVEL},
    surface::{CapType, ExtrudeDirection, Extrusion, Lathe, Loft, Sweep},
};
use grid4d_scene::{
    Axis, ComponentState, Components, Curve, Document, Falloff, FrameNaming, FrameRange, ImageFormat, Interpolation,
    Key, LightDetails, LightShadow, LightType, Material, MaterialId, MeshHandle, MeshId, NodeData, NodeId, ObjectKind,
    ObjectParam, Param, RenderSettings, SplineHandle, Tag, Tangents, TimeSettings, Track, Transform,
};
use serde::{Deserialize, Serialize};
use zip::{CompressionMethod, DateTime, ZipWriter, write::SimpleFileOptions};

use crate::{
    FORMAT_TAG, IoError,
    common::{Archive, assemble_with_ids, check_finite},
};

pub(crate) const VERSION: u32 = 3;
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
    /// Added in the course of version 3 (step 8.2); absent while they are the
    /// defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    render: Option<RenderDto>,
}

/// A material; colours are linear RGB.
#[derive(Debug, Serialize, Deserialize)]
struct MaterialDto {
    name: String,
    color: [f32; 3],
    roughness: f32,
    metallic: f32,
    emission: [f32; 3],
}

impl MaterialDto {
    fn new(material: &Material) -> Self {
        Self {
            name: material.name.clone(),
            color: material.color,
            roughness: material.roughness,
            metallic: material.metallic,
            emission: material.emission,
        }
    }

    fn into_material(self) -> Result<Material, IoError> {
        let material = Material {
            name: self.name,
            color: self.color.map(|c| c.max(0.0)),
            roughness: self.roughness.clamp(0.0, 1.0),
            metallic: self.metallic.clamp(0.0, 1.0),
            emission: self.emission.map(|c| c.max(0.0)),
        };
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
}

/// An animated parameter and its keys.
#[derive(Debug, Serialize, Deserialize)]
struct TrackDto {
    param: ParamDto,
    keys: Vec<KeyDto>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
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
}

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
        }
    }
}

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
    fn new(track: &Track) -> Self {
        Self {
            param: track.param.into(),
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

    fn into_track(self) -> Track {
        Track {
            param: self.param.into(),
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
        }
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
    /// Added with materials (step 6.6): the material's index in `materials`.
    Material {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        material: Option<usize>,
    },
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
            Tag::Material { material } => Self::Material {
                material: material.and_then(|m| refs.materials.get(&m).copied()),
            },
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
            Self::Material { material } => Tag::Material {
                material: material.and_then(|i| materials.get(i).copied()),
            },
        }
    }

    fn parameters(&self) -> Vec<f32> {
        match self {
            Self::Phong { angle, .. } => vec![*angle],
            Self::Protection { .. } | Self::Target { .. } | Self::Material { .. } => Vec::new(),
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
    /// `selected` / `hidden` flag components (optional, added in step 4.1).
    Polygon {
        mesh: usize,
        #[serde(default, skip_serializing_if = "SetDto::is_empty")]
        selected: SetDto,
        #[serde(default, skip_serializing_if = "SetDto::is_empty")]
        hidden: SetDto,
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DeformerModeDto {
    #[default]
    Limited,
    WithinBox,
    Unlimited,
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

/// Writes `doc` as a version 3 archive.
pub(crate) fn write(doc: &Document) -> Result<Vec<u8>, IoError> {
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
    for object in &objects {
        object.check_finite()?;
    }
    for (mesh, object) in &table.meshes {
        check_mesh_finite(mesh.mesh(), object)?;
    }
    let settings = doc.time.settings();
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
            materials: doc.scene.materials().iter().map(|(_, m)| MaterialDto::new(m)).collect(),
            render: RenderDto::new(&doc.render),
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
    Ok(zip.finish()?.into_inner())
}

impl File {
    pub(crate) fn parse(text: &str) -> Result<Self, IoError> {
        Ok(serde_json::from_str(text)?)
    }

    /// Builds the document, reading the meshes from `archive`.
    pub(crate) fn into_document(self, archive: &mut Archive<'_>) -> Result<Document, IoError> {
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
            render,
        } = self.document;
        let materials = materials
            .into_iter()
            .map(MaterialDto::into_material)
            .collect::<Result<Vec<_>, IoError>>()?;
        let mut tags: Vec<Vec<TagDto>> = Vec::with_capacity(objects.len());
        let objects = objects
            .into_iter()
            .map(|object| {
                object.check_finite()?;
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
        let material_ids: Vec<MaterialId> = materials
            .into_iter()
            .map(|material| doc.scene.materials_mut().insert(material))
            .collect();
        // Tags last: they may refer to any object.
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
        Ok(doc)
    }
}

impl ObjectDto {
    fn new(data: &NodeData, parent: Option<usize>, meshes: &mut MeshTable, refs: &Refs) -> Self {
        let t = &data.transform;
        Self {
            name: data.name.clone(),
            parent,
            kind: KindDto::new(&data.kind, &data.name, meshes),
            position: t.position.to_array(),
            rotation_hpb: t.rotation.to_array(),
            scale: t.scale.to_array(),
            display_color: data.display_color,
            visible_in_editor: data.visible_in_editor,
            visible_in_render: data.visible_in_render,
            enabled: data.enabled,
            tags: data.tags.iter().map(|tag| TagDto::new(tag, refs)).collect(),
            tracks: data.tracks.iter().map(TrackDto::new).collect(),
        }
    }

    /// The object's data; its tags come back separately, still referring to
    /// objects by index (they are resolved once every object exists).
    fn into_data(self, meshes: &[MeshHandle]) -> Result<(NodeData, Vec<TagDto>), IoError> {
        let kind = self.kind.into_kind(meshes, &self.name)?;
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
            // One track per parameter: a later one replaces an earlier one.
            tracks: self.tracks.into_iter().fold(Vec::new(), |mut tracks: Vec<Track>, dto| {
                let track = dto.into_track();
                tracks.retain(|t| t.param != track.param);
                tracks.push(track);
                tracks
            }),
            unkeyed: None,
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
            Self::Polygon { mesh, selected, hidden } => {
                let mesh = meshes.get(mesh).cloned().ok_or_else(|| {
                    IoError::Invalid(format!(
                        "object \"{object}\" uses mesh {mesh}, but the file has {} meshes",
                        meshes.len()
                    ))
                })?;
                let state = ComponentState {
                    selected: selected.into_set(mesh.mesh(), object, "selection")?,
                    hidden: hidden.into_set(mesh.mesh(), object, "hidden")?,
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
        let tag = |material| Tag::Material { material };
        doc.scene.get_mut(id(&doc, "Cube")).unwrap().data.tags = vec![tag(Some(red_id))];
        doc.scene.get_mut(id(&doc, "Sphere")).unwrap().data.tags = vec![tag(Some(gone))];
        doc.scene.get_mut(id(&doc, "Box")).unwrap().data.tags = vec![tag(None)];

        let loaded = round_trip(&doc);
        let materials: Vec<Material> = loaded.scene.materials().iter().map(|(_, m)| m.clone()).collect();
        assert_eq!(materials, [red, plain], "in Material Manager order, exactly");
        let [Tag::Material { material: Some(m) }] = tags_of(&loaded, "Cube")[..] else {
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
        let newer = ok.replacen("\"version\": 3", "\"version\": 7", 1);
        assert!(matches!(
            load(&newer),
            Err(IoError::UnsupportedVersion {
                found: 7,
                newest: FORMAT_VERSION
            })
        ));
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
