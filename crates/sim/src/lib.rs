//! # grid4d-sim
//!
//! Rigid body simulation for Grid4D (see `docs/reference/simulation-rigid-body.md`).
//!
//! The engine is [`rapier3d`]; its types never leave this crate. Rapier does
//! its maths in its own glam version, so values cross the edge as plain arrays
//! (`convert.rs`). Inside, everything is in **metres** (the engine's
//! tolerances assume them); outside, in centimetres like the rest of Grid4D.
//! The engine runs on one thread, so a run is deterministic.

mod convert;
mod world;

pub use world::{BodyId, World};
