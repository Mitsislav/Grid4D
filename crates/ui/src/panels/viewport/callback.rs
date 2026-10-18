//! Glue between egui's paint callbacks and `oc4d_render::ViewportRenderer`.
//!
//! The renderer lives in egui-wgpu's `CallbackResources` (created once at start-up);
//! each frame we hand egui a tiny callback that points at the shared `FrameData`.

use std::sync::{Arc, Mutex, PoisonError};

use egui_wgpu::wgpu;
use oc4d_render::{FrameData, ViewportRenderer};

pub(super) struct ViewportCallback {
    frame: Arc<Mutex<FrameData>>,
}

impl ViewportCallback {
    pub(super) fn new(frame: Arc<Mutex<FrameData>>) -> Self {
        Self { frame }
    }
}

impl egui_wgpu::CallbackTrait for ViewportCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(renderer) = resources.get_mut::<ViewportRenderer>() {
            let frame = self.frame.lock().unwrap_or_else(PoisonError::into_inner);
            renderer.prepare(device, queue, &frame);
        }
        // `Vec::new()` does not allocate.
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        if let Some(renderer) = resources.get::<ViewportRenderer>() {
            renderer.paint(pass);
        }
    }
}
