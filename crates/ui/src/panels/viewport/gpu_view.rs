//! Glue between egui and `oc4d_render`: the viewport renders into its own
//! [`ViewportTarget`], and the resolved texture is shown as an egui image.
//!
//! This is the only place that talks to `egui_wgpu` (texture registration); the
//! renderer itself knows nothing about egui.

use std::fmt;

use egui_wgpu::wgpu;
use oc4d_render::{FrameData, ViewportRenderer, ViewportTarget};

/// The GPU side of one viewport.
pub(super) struct GpuView {
    render_state: egui_wgpu::RenderState,
    renderer: ViewportRenderer,
    target: ViewportTarget,
    /// The target's resolved colour, registered with egui.
    texture: egui::TextureId,
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
        log::info!("Viewport: offscreen target, {samples}× MSAA");
        Self {
            render_state: render_state.clone(),
            renderer,
            target,
            texture,
        }
    }

    /// Renders `frame` at `size` physical pixels and returns the texture to show.
    /// The textures are recreated only when the size changes.
    pub(super) fn render(&mut self, frame: &FrameData, size: [u32; 2]) -> egui::TextureId {
        let rs = &self.render_state;
        if self.target.resize(&rs.device, size) {
            rs.renderer.write().update_egui_texture_from_wgpu_texture(
                &rs.device,
                self.target.view(),
                wgpu::FilterMode::Linear,
                self.texture,
            );
        }
        self.renderer.render(&rs.device, &rs.queue, frame, &self.target);
        self.texture
    }
}

impl Drop for GpuView {
    fn drop(&mut self) {
        self.render_state.renderer.write().free_texture(&self.texture);
    }
}

impl fmt::Debug for GpuView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GpuView")
            .field("target", &self.target)
            .field("texture", &self.texture)
            .finish_non_exhaustive()
    }
}
