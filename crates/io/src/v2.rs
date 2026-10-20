//! Format version 2 — read-only. JSON files with segment counts on every
//! primitive and the Cone, Torus, Disc and Capsule objects, but no polygon
//! objects; they load unchanged into the current scene types. These types are
//! frozen.

use glam::Vec3;
use grid4d_scene::{Document, NodeData, ObjectKind, TimeSettings, Transform};
use serde::Deserialize;

use crate::{
    IoError,
    common::{assemble, check_finite},
};

#[derive(Debug, Deserialize)]
pub(crate) struct File {
    document: DocumentDto,
}

#[derive(Debug, Deserialize)]
struct DocumentDto {
    name: String,
    time: TimeDto,
    objects: Vec<ObjectDto>,
}

#[derive(Debug, Deserialize)]
struct TimeDto {
    fps: u32,
    start: i32,
    end: i32,
    looping: bool,
}

#[derive(Debug, Deserialize)]
struct ObjectDto {
    name: String,
    /// Index of the parent in `objects`, always smaller than this object's own.
    #[serde(default)]
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
}

#[derive(Debug, Deserialize)]
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
}

impl File {
    pub(crate) fn parse(text: &str) -> Result<Self, IoError> {
        Ok(serde_json::from_str(text)?)
    }

    pub(crate) fn into_document(self) -> Result<Document, IoError> {
        let DocumentDto { name, time, objects } = self.document;
        let objects = objects
            .into_iter()
            .map(|object| {
                object.check_finite()?;
                Ok((object.parent, object.into_data()))
            })
            .collect::<Result<Vec<_>, IoError>>()?;
        let time = TimeSettings {
            fps: time.fps,
            start: time.start,
            end: time.end,
            looping: time.looping,
        };
        assemble(name, time, objects)
    }
}

impl ObjectDto {
    fn into_data(self) -> NodeData {
        NodeData {
            name: self.name,
            kind: self.kind.into(),
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
        }
    }

    fn check_finite(&self) -> Result<(), IoError> {
        check_finite(
            &self.name,
            &[
                ("position", &self.position),
                ("rotation", &self.rotation_hpb),
                ("scale", &self.scale),
                ("display colour", &self.display_color),
                ("parameters", &self.kind.parameters()),
            ],
        )
    }
}

impl KindDto {
    /// Every floating-point parameter, for validation.
    fn parameters(&self) -> Vec<f32> {
        match *self {
            Self::Null => Vec::new(),
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

impl From<KindDto> for ObjectKind {
    fn from(kind: KindDto) -> Self {
        match kind {
            KindDto::Null => Self::Null,
            KindDto::Cube { size, segments } => Self::Cube {
                size: Vec3::from_array(size),
                segments,
            },
            KindDto::Sphere { radius, segments } => Self::Sphere { radius, segments },
            KindDto::Cylinder {
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
            KindDto::Plane {
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
            KindDto::Cone {
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
            KindDto::Torus {
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
            KindDto::Disc {
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
            KindDto::Capsule {
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
            KindDto::Camera { fov_y_deg } => Self::Camera { fov_y_deg },
            KindDto::Light { color, intensity } => Self::Light { color, intensity },
        }
    }
}
