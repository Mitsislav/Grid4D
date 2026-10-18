//! The viewport's own render target: a multisampled colour + depth pair that is
//! resolved into a single-sample texture the UI displays.
//!
//! Owning the target (instead of drawing inside the UI toolkit's pass) gives the
//! viewport its own MSAA and depth, independent of the window, and is what later
//! passes need — wireframe overlays with their own depth, several viewports with
//! different settings, post effects.

use crate::GpuTargetConfig;

/// Textures of one viewport, sized in physical pixels.
#[derive(Debug)]
pub struct ViewportTarget {
    size: [u32; 2],
    samples: u32,
    /// Single-sample colour the UI samples; the MSAA colour resolves into it.
    resolve_view: wgpu::TextureView,
    /// Multisampled colour, `None` without MSAA (then the pass draws into `resolve_view`).
    msaa_view: Option<wgpu::TextureView>,
    depth_view: wgpu::TextureView,
}

impl ViewportTarget {
    /// Holds gamma-encoded colour: the shaders encode to sRGB themselves, and UI
    /// toolkits (egui) sample textures as gamma values.
    pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
    pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
    /// Upper limit per side; also keeps a corrupt size from asking for gigabytes.
    pub const MAX_SIZE: u32 = 8192;

    /// The MSAA sample count to use on `adapter`: 4 if both formats support it.
    #[must_use]
    pub fn supported_samples(adapter: &wgpu::Adapter) -> u32 {
        let four = [Self::COLOR_FORMAT, Self::DEPTH_FORMAT].into_iter().all(|format| {
            adapter
                .get_texture_format_features(format)
                .flags
                .sample_count_supported(4)
        });
        if four { 4 } else { 1 }
    }

    /// A target of `size` pixels (clamped to `1..=MAX_SIZE`) with `samples` MSAA samples.
    #[must_use]
    pub fn new(device: &wgpu::Device, size: [u32; 2], samples: u32) -> Self {
        let size = clamp_size(size);
        let samples = samples.max(1);
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let texture = |label, format, sample_count, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: extent,
                    mip_level_count: 1,
                    sample_count,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let resolve_view = texture(
            "oc4d.viewport.color",
            Self::COLOR_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let msaa_view = (samples > 1).then(|| {
            texture(
                "oc4d.viewport.color.msaa",
                Self::COLOR_FORMAT,
                samples,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            )
        });
        let depth_view = texture(
            "oc4d.viewport.depth",
            Self::DEPTH_FORMAT,
            samples,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        Self {
            size,
            samples,
            resolve_view,
            msaa_view,
            depth_view,
        }
    }

    /// Recreates the textures if `size` (clamped) differs from the current one.
    /// Returns `true` if they were recreated — the displayed view then changed.
    pub fn resize(&mut self, device: &wgpu::Device, size: [u32; 2]) -> bool {
        if clamp_size(size) == self.size {
            return false;
        }
        *self = Self::new(device, size, self.samples);
        true
    }

    #[must_use]
    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    /// The resolved colour, for display.
    #[must_use]
    pub fn view(&self) -> &wgpu::TextureView {
        &self.resolve_view
    }

    /// What pipelines drawing into this target must be built for.
    #[must_use]
    pub fn config(&self) -> GpuTargetConfig {
        GpuTargetConfig {
            color_format: Self::COLOR_FORMAT,
            depth_format: Some(Self::DEPTH_FORMAT),
            msaa_samples: self.samples,
        }
    }

    /// The pass attachments: clears colour to transparent (the UI draws the
    /// background underneath) and depth to far; nothing but the resolve is kept.
    pub(crate) fn attachments(
        &self,
    ) -> (
        wgpu::RenderPassColorAttachment<'_>,
        wgpu::RenderPassDepthStencilAttachment<'_>,
    ) {
        let (view, resolve_target, store) = match &self.msaa_view {
            Some(msaa) => (msaa, Some(&self.resolve_view), wgpu::StoreOp::Discard),
            None => (&self.resolve_view, None, wgpu::StoreOp::Store),
        };
        let color = wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store,
            },
        };
        let depth = wgpu::RenderPassDepthStencilAttachment {
            view: &self.depth_view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(1.0),
                store: wgpu::StoreOp::Discard,
            }),
            stencil_ops: None,
        };
        (color, depth)
    }
}

fn clamp_size(size: [u32; 2]) -> [u32; 2] {
    size.map(|s| s.clamp(1, ViewportTarget::MAX_SIZE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_clamped() {
        assert_eq!(clamp_size([0, 50_000]), [1, ViewportTarget::MAX_SIZE]);
        assert_eq!(clamp_size([640, 480]), [640, 480]);
    }

    #[test]
    fn the_display_format_is_not_srgb_aware() {
        // egui samples textures as gamma values; an *Srgb format would decode them.
        assert!(!ViewportTarget::COLOR_FORMAT.is_srgb());
    }
}
