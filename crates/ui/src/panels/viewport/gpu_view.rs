//! Glue between egui and `grid4d_render`: the viewport renders into its own
//! [`ViewportTarget`], and the resolved texture is shown as an egui image.
//!
//! This is the only place that talks to `egui_wgpu` (texture registration); the
//! renderer itself knows nothing about egui.

use std::fmt;

use egui_wgpu::wgpu;
use grid4d_io::Pixels;
use grid4d_render::{FrameData, MISSING_VERSION, TexelFormat, TextureLevel, ViewportRenderer, ViewportTarget};
use grid4d_scene::{ColorSpace, Textures};

use crate::textures::{Status, TextureCache};

/// The GPU side of the viewport: one renderer (pipelines, mesh cache) shared by
/// the views, and an offscreen target per view.
pub(super) struct GpuView {
    render_state: egui_wgpu::RenderState,
    renderer: ViewportRenderer,
    samples: u32,
    /// Per view: its target and the target's resolved colour, registered with egui.
    targets: Vec<(ViewportTarget, egui::TextureId)>,
}

impl GpuView {
    pub(super) fn new(render_state: &egui_wgpu::RenderState) -> Self {
        let samples = ViewportTarget::supported_samples(&render_state.adapter);
        let target = ViewportTarget::new(&render_state.device, [1, 1], samples);
        let renderer = ViewportRenderer::new(&render_state.device, &target.config());
        let texture = render_state.renderer.write().register_native_texture(
            &render_state.device,
            target.view(),
            wgpu::FilterMode::Linear,
        );
        log::info!("Viewport: offscreen targets, {samples}× MSAA");
        Self {
            render_state: render_state.clone(),
            renderer,
            samples,
            targets: vec![(target, texture)],
        }
    }

    /// Renders `frame` for view `view` at `size` physical pixels and returns the
    /// texture to show. A view's target is made on its first use and recreated
    /// only when its size changes.
    pub(super) fn render(&mut self, view: usize, frame: &FrameData, size: [u32; 2]) -> egui::TextureId {
        let rs = &self.render_state;
        while self.targets.len() <= view {
            let target = ViewportTarget::new(&rs.device, [1, 1], self.samples);
            let texture =
                rs.renderer
                    .write()
                    .register_native_texture(&rs.device, target.view(), wgpu::FilterMode::Linear);
            self.targets.push((target, texture));
        }
        let Some((target, texture)) = self.targets.get_mut(view) else {
            return egui::TextureId::default();
        };
        if target.resize(&rs.device, size) {
            rs.renderer.write().update_egui_texture_from_wgpu_texture(
                &rs.device,
                target.view(),
                wgpu::FilterMode::Linear,
                *texture,
            );
        }
        // One renderer for every view: each render writes its camera and
        // instances, then submits its own pass, so they never mix.
        self.renderer.render(&rs.device, &rs.queue, frame, target);
        *texture
    }
}

impl GpuView {
    /// Hands the renderer the document's texture pictures that arrived or
    /// changed since it last looked (missing and unreadable ones as a
    /// checker), and lets it free the ones the document no longer has.
    /// Nothing is copied while every picture is up to date.
    pub(super) fn sync_textures(&mut self, textures: &Textures, cache: &TextureCache) {
        let rs = &self.render_state;
        for (id, texture) in textures.iter() {
            match cache.status(id, texture) {
                Status::Ready { image, version } => {
                    if self.renderer.has_texture(id, version) {
                        continue;
                    }
                    let format = match image.levels.first().map(|l| &l.pixels) {
                        Some(Pixels::RgbaF16(_)) => TexelFormat::Float16,
                        _ if texture.color_space == ColorSpace::Linear => TexelFormat::Linear8,
                        _ => TexelFormat::Srgb8,
                    };
                    let bytes: Vec<_> = image.levels.iter().map(grid4d_io::Level::bytes).collect();
                    let levels: Vec<TextureLevel<'_>> = image
                        .levels
                        .iter()
                        .zip(&bytes)
                        .map(|(level, bytes)| TextureLevel {
                            width: level.width,
                            height: level.height,
                            bytes,
                        })
                        .collect();
                    self.renderer
                        .upload_texture(&rs.device, &rs.queue, id, version, format, &levels);
                }
                Status::Missing | Status::Failed(_) => {
                    if !self.renderer.has_texture(id, MISSING_VERSION) {
                        self.renderer.missing_texture(&rs.device, &rs.queue, id);
                    }
                }
                Status::Loading => {}
            }
        }
        self.renderer.retain_textures(|id| textures.contains(id));
    }
}

impl Drop for GpuView {
    fn drop(&mut self) {
        let mut renderer = self.render_state.renderer.write();
        for (_, texture) in &self.targets {
            renderer.free_texture(texture);
        }
    }
}

impl fmt::Debug for GpuView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GpuView")
            .field("targets", &self.targets.len())
            .finish_non_exhaustive()
    }
}
