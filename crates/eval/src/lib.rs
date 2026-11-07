//! # grid4d-eval
//!
//! Turns the document — what the user authored — into what viewers draw and pick
//! this frame: an [`EvaluatedScene`] of visible nodes with world matrices,
//! [`Drawable`]s (unit geometry named by a [`MeshKey`], plus scale and colour)
//! and pick shapes. [`editable_mesh`] is the geometry "Make Editable" bakes;
//! [`generator_mesh`] what a generator (Subdivision Surface) shows.
//!
//! ## Architecture rule
//!
//! Sits between the scene and its consumers:
//!
//! ```text
//! grid4d-geom ◀── grid4d-scene ◀── grid4d-eval ◀── grid4d-render ◀── grid4d-ui
//! ```
//!
//! It reads the scene and never mutates it, and knows nothing about GPUs or UI.
//! Generators, deformers and animation (later phases) evaluate here, so the
//! renderer and picking never need to know about them.
//!
//! [`EvaluatedScene::update`] is on the per-frame hot path: it skips all work
//! when the scene's revision is unchanged and allocates nothing in steady state.

mod curve;
mod deform;
mod drawable;
mod evaluated;
mod generator;
mod key;
mod material_layer;
mod overrides;

pub use curve::{CurveHandle, CurveId};
pub use deform::is_deformer;
pub use drawable::{Drawable, Shading, Surface, drawable, editable_mesh};
pub use evaluated::{EvaluatedInstance, EvaluatedLight, EvaluatedNode, EvaluatedScene, PickShape, Purpose};
pub use generator::{generator_color, generator_mesh};
pub use key::{MeshKey, Ratio};
pub use material_layer::{MaterialLayer, MaterialLayers};
pub use overrides::Overrides;
