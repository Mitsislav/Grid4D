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
    subdivision::{Boundary, MAX_LEVEL},
};
use grid4d_scene::{
    Axis, ComponentState, Components, Curve, Document, Interpolation, Key, MeshHandle, MeshId, NodeData, NodeId,
    ObjectKind, Param, Tag, Tangents, TimeSettings, Track, Transform,
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
}

#[derive(Debug, Serialize, Deserialize)]
struct TimeDto {
    fps: u32,
    start: i32,
    end: i32,
    looping: bool,
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
}

impl TagDto {
    fn new(tag: &Tag, index_of: &HashMap<NodeId, usize>) -> Self {
        let index = |id: Option<NodeId>| id.and_then(|id| index_of.get(&id).copied());
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
        }
    }

    /// The tag, with object references resolved through `ids` (an index that names
    /// no object drops the reference).
    fn into_tag(self, ids: &[NodeId]) -> Tag {
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
        }
    }

    fn parameters(&self) -> Vec<f32> {
        match self {
            Self::Phong { angle, .. } => vec![*angle],
            Self::Protection { .. } | Self::Target { .. } => Vec::new(),
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
            points: bits(self.points, mesh.point_count(), "points")?,
            edges: bits(self.edges, mesh.corner_count(), "corners")?,
            polygons: bits(self.polygons, mesh.face_count(), "polygons")?,
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
    let index_of: HashMap<NodeId, usize> = order.iter().enumerate().map(|(i, &id)| (id, i)).collect();
    let mut objects = Vec::with_capacity(order.len());
    let mut table = MeshTable::default();
    for &id in &order {
        let Some(node) = doc.scene.get(id) else { continue };
        let parent = node.parent().and_then(|p| index_of.get(&p).copied());
        objects.push(ObjectDto::new(&node.data, parent, &mut table, &index_of));
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

        let DocumentDto { name, time, objects } = self.document;
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
        // Tags last: they may refer to any object.
        for (&id, object_tags) in ids.iter().zip(tags) {
            if !object_tags.is_empty()
                && let Some(node) = doc.scene.get_mut(id)
            {
                node.data.tags = object_tags.into_iter().map(|tag| tag.into_tag(&ids)).collect();
            }
        }
        Ok(doc)
    }
}

impl ObjectDto {
    fn new(data: &NodeData, parent: Option<usize>, meshes: &mut MeshTable, index_of: &HashMap<NodeId, usize>) -> Self {
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
            tags: data.tags.iter().map(|tag| TagDto::new(tag, index_of)).collect(),
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
            ObjectKind::Light { color, intensity } => Self::Light { color, intensity },
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
            Self::Light { color, intensity } => ObjectKind::Light { color, intensity },
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
        })
    }

    /// Every floating-point parameter, for validation.
    fn parameters(&self) -> Vec<f32> {
        match *self {
            Self::Null | Self::Polygon { .. } | Self::SubdivisionSurface { .. } => Vec::new(),
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
            Self::Light { color, intensity } => vec![color[0], color[1], color[2], intensity],
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
}
