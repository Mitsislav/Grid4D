//! # grid4d-scene
//!
//! The document model of Grid4D: the scene graph (objects, hierarchy, transforms),
//! document time and render settings. This crate is the single source of truth
//! for scene data.
//!
//! ## Architecture rule
//!
//! This crate must **never** depend on rendering (`wgpu`) or UI (`egui`) crates.
//! Renderers and panels *read* from it; structural mutations go through its API
//! so that invariants (parent/child links, no cycles) always hold.
//!
//! ## Conventions
//!
//! * Units are centimetres (1.0 = 1 cm), like Cinema 4D.
//! * World space is right-handed and Y-up.
//! * Rotations are stored as HPB Euler angles in radians (see [`Transform`]).
//! * Colours are linear RGB.

mod anim;
mod bounds;
mod document;
mod edit;
mod light;
mod material;
mod mesh;
mod node;
mod render;
mod scene;
mod tag;
mod time;
mod transform;

#[cfg(test)]
mod test_util;

pub use anim::{
    Axis, Curve, Interpolation, Key, KeyState, Param, ParamSet, TangentPreset, Tangents, Track, Tracks, Unkeyed,
};
pub use bounds::Aabb;
pub use document::Document;
pub use edit::SceneEdit;
pub use light::{Falloff, LightDetails, LightShadow, LightType};
pub use material::{Material, MaterialId, Materials};
pub use mesh::{ComponentState, Components, MeshHandle, MeshId};
pub use node::{CAMERA_MARKER_SIZE, LIGHT_MARKER_RADIUS, Node, NodeData, NodeId, ObjectKind, ObjectKindTag};
pub use render::{FrameRange, RenderSettings};
pub use scene::{Detached, Scene, SceneError, WalkItem};
pub use tag::{Aim, DEFAULT_PHONG_ANGLE, Protection, Tag, TagKind, Tags};
pub use time::{Playback, TimeSettings, TimeState};
pub use transform::Transform;
