//! # oc4d-geom
//!
//! Polygon geometry for OpenC4D: n-gon meshes ([`PolyMesh`]), their triangulation
//! for the GPU ([`Triangulator`] → [`TriMesh`]), vertex normals ([`Smoothing`]) and
//! adjacency ([`Topology`]).
//!
//! ## Architecture rule
//!
//! Depends only on `glam`: no scene, no GPU, no UI. The scene stores meshes from
//! this crate; the renderer, picking, modelling tools and file formats consume
//! them. Indices are `u32` (what GPUs index with); a mesh refuses to grow past
//! `u32::MAX` points or corners instead of wrapping.
//!
//! ## Conventions
//!
//! Faces are wound counter-clockwise when seen from the side their normal points
//! to (right-handed), like the rest of OpenC4D. A face is a simple polygon of three
//! or more distinct points; it may be non-convex and slightly non-planar.

mod mesh;
mod normals;
pub mod primitives;
mod topology;
mod triangulate;

#[cfg(test)]
mod shapes;

pub use mesh::{MeshError, PolyMesh};
pub use normals::Smoothing;
pub use topology::Topology;
pub use triangulate::{TriMesh, Triangulator};

/// `u32` index → `usize`. Lossless on every platform OpenC4D supports (≥ 32-bit).
pub(crate) fn us(index: u32) -> usize {
    usize::try_from(index).unwrap_or(usize::MAX)
}

/// `usize` count → `u32` index. Callers only pass counts a [`PolyMesh`] has
/// already validated to fit.
pub(crate) fn u32_index(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}
