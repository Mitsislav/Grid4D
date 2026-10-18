//! # oc4d-scene
//!
//! The document model of OpenC4D: the scene graph (objects, hierarchy, transforms)
//! and document time. This crate is the single source of truth for scene data.
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

mod bounds;
mod document;
mod node;
mod scene;
mod time;
mod transform;

pub use bounds::Aabb;
pub use document::Document;
pub use node::{CAMERA_MARKER_SIZE, LIGHT_MARKER_RADIUS, Node, NodeId, ObjectKind, ObjectKindTag};
pub use scene::{Scene, SceneError, WalkItem};
pub use time::{Playback, TimeState};
pub use transform::Transform;
