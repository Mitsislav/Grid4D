//! # grid4d-raytrace
//!
//! Grid4D's final renderer: a CPU path tracer over the evaluated scene (see
//! `docs/reference/render-engine.md`).
//!
//! ```text
//!  EvaluatedScene::for_render ──▶ RenderScene ──render(camera, options)──▶ Image
//!  (grid4d-eval: render          (world triangles   (rows in parallel,      (linear RGBA,
//!   visibility and levels)        in one BVH,         one seeded random      premultiplied;
//!  + Texturing: materials,        material layers,    stream per pixel)      8-bit sRGB out)
//!    pictures, cameras            lights)
//! ```
//!
//! Deterministic: a pixel's samples depend only on the scene, the options and its
//! position, never on how threads share the work.

mod camera;
mod channels;
mod environment;
mod image;
mod lights;
mod passes;
mod picture;
mod sample;
mod scene;
mod shade;
mod trace;

pub use camera::{CameraProjection, RenderCamera, ThinLens};
pub use image::Image;
pub use passes::PassLayer;
pub use picture::{Picture, Pictures, Texels};
pub use scene::{RenderScene, Texturing};
pub use trace::{Progressive, RenderControl, RenderOptions, render};
