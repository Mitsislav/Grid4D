//! # oc4d-render
//!
//! GPU viewport rendering for OpenC4D, built directly on `wgpu`.
//!
//! ```text
//!  Scene ──extract──▶ FrameData ──prepare──▶ GPU buffers ──paint──▶ render pass
//!  (oc4d-scene)       (CPU, reused)          (ViewportRenderer)     (owned by the host)
//! ```
//!
//! ## Architecture rules
//!
//! * Reads the scene, never mutates it.
//! * Knows nothing about egui: the UI layer adapts [`ViewportRenderer`] to whatever
//!   paint-callback mechanism it uses (`egui_wgpu::CallbackTrait` today).
//! * The per-frame path (`FrameData::extract`, `ViewportRenderer::prepare`,
//!   `ViewportRenderer::paint`) is allocation-free in steady state: CPU vectors are
//!   cleared and refilled, and GPU buffers only grow (geometrically) when needed.

mod camera;
mod frame;
mod gpu;
mod mesh;
mod picking;

pub use camera::{OrbitCamera, Ray};
pub use frame::{FrameData, InstanceRaw, MeshKind};
pub use gpu::{GpuTargetConfig, ViewportRenderer};
pub use mesh::{MeshData, Vertex};
pub use picking::pick;

/// WGSL sources, exposed for offline validation and tooling.
pub mod shaders {
    pub const MESH: &str = include_str!("shaders/mesh.wgsl");
    pub const GRID: &str = include_str!("shaders/grid.wgsl");
}
