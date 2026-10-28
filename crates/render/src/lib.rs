//! # grid4d-render
//!
//! GPU viewport rendering for Grid4D, built directly on `wgpu`.
//!
//! ```text
//!  EvaluatedScene ──extract──▶ FrameData ──render──▶ ViewportTarget ──▶ texture the UI shows
//!  (grid4d-eval)                 (CPU, reused)        (own MSAA + depth, resolved)
//! ```
//!
//! ## Architecture rules
//!
//! * Reads the evaluated scene (`grid4d-eval`), never the document itself.
//! * Knows nothing about egui: it renders into its own [`ViewportTarget`], and the UI
//!   layer registers the resolved texture with its toolkit to display it.
//! * The per-frame path (`FrameData::extract`, `ViewportRenderer::prepare`,
//!   `ViewportRenderer::paint`) is allocation-free in steady state: CPU vectors are
//!   cleared and refilled, and GPU buffers only grow (geometrically) when needed.

mod camera;
mod components;
mod display;
mod effects;
mod filter;
mod frame;
mod gpu;
mod mesh;
mod picking;
mod snap;
mod target;

pub use camera::{OrbitCamera, Projection, Ray};
pub use components::{
    Brush, BrushCoverage, ComponentHit, ComponentStyle, RegionRules, ScreenView, components_in_rect, nearest_face_edge,
    paint_brush, pick_component,
};
pub use display::{DisplayMode, DisplayStyle, ShadingMode};
pub use effects::ViewEffects;
pub use filter::{FilterItem, ViewFilter};
pub use frame::{FrameData, GridPlane, InstanceRaw, LightRaw};
pub use gpu::{GpuTargetConfig, ViewportRenderer};
pub use mesh::{TessellationScratch, Vertex, tessellate};
pub use picking::{pick, pick_curve};
pub use snap::{SnapCache, SnapHit, SnapKind, SnapQuery, SnapTargets, find_snap, snapped_position};
pub use target::ViewportTarget;

/// WGSL sources, exposed for offline validation and tooling.
pub mod shaders {
    pub const MESH: &str = include_str!("shaders/mesh.wgsl");
    pub const GRID: &str = include_str!("shaders/grid.wgsl");
    pub const OVERLAY: &str = include_str!("shaders/overlay.wgsl");
    pub const SHADOW: &str = include_str!("shaders/shadow.wgsl");
    pub const SSAO: &str = include_str!("shaders/ssao.wgsl");
}
