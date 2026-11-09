//! Texture pictures on the GPU and the materials that use them (see
//! `docs/reference/material-channels.md`).
//!
//! The interface hands decoded pictures over with
//! [`ViewportRenderer::upload_texture`](crate::ViewportRenderer::upload_texture)
//! when one arrives or changes (never per frame); a material with textures
//! gets one bind group (group 2 of the mesh shader: its channels' settings,
//! the sampler and a picture per channel), made again only when its slots or
//! one of its pictures change. Everything else draws with a default group
//! whose channels are all empty.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use grid4d_scene::{Channel, MaterialId, TextureId};

use crate::frame::MaterialData;

/// How a picture's texels are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TexelFormat {
    /// 8-bit RGBA, sRGB-encoded colour.
    Srgb8,
    /// 8-bit RGBA data (roughness, normals, …), read as is.
    Linear8,
    /// Half-float RGBA, linear (HDR, OpenEXR, 16-bit pictures).
    Float16,
}

impl TexelFormat {
    const fn wgpu(self) -> wgpu::TextureFormat {
        match self {
            Self::Srgb8 => wgpu::TextureFormat::Rgba8UnormSrgb,
            Self::Linear8 => wgpu::TextureFormat::Rgba8Unorm,
            Self::Float16 => wgpu::TextureFormat::Rgba16Float,
        }
    }

    const fn texel_bytes(self) -> u32 {
        match self {
            Self::Srgb8 | Self::Linear8 => 4,
            Self::Float16 => 8,
        }
    }
}

/// One level of a picture's mip chain: `bytes` holds `width × height` texels,
/// rows from the top.
#[derive(Clone, Copy, Debug)]
pub struct TextureLevel<'a> {
    pub width: u32,
    pub height: u32,
    pub bytes: &'a [u8],
}

/// The version the renderer records for a missing picture (a checker).
pub const MISSING_VERSION: u64 = u64::MAX;

/// The GPU memory all pictures together may take; a picture that would pass
/// it goes up from a smaller mip level.
pub const TEXTURE_BUDGET: u64 = 1 << 30;

#[derive(Debug)]
struct GpuTexture {
    /// Kept alive with its view.
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    version: u64,
    /// Every texel grey: colour channels multiply their value by it.
    grey: bool,
    /// The GPU memory it takes.
    bytes: u64,
}

/// Layout must match `MaterialUniform` in `shaders/mesh.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub(crate) struct MaterialUniform {
    /// Emission strength, opacity, opacity threshold, specular.
    header: [f32; 4],
    /// Normal strength, bump height (cm), how far the matte lighting wraps
    /// round (subsurface), unused.
    shape: [f32; 4],
    /// Per channel: tiling (u, v), offset (u, v); range (from, to),
    /// strength, flags (1: a picture, 2: a grey one).
    channels: [[f32; 8]; 10],
}

impl MaterialUniform {
    /// Nothing textured, nothing cut out.
    fn empty() -> Self {
        Self {
            header: [1.0, 1.0, 0.0, 0.5],
            shape: [1.0, 0.1, 0.0, 0.0],
            channels: [[1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0]; 10],
        }
    }
}

/// What a material's bind group was made from: when it differs, it is made
/// again.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Signature {
    material: MaterialData,
    /// The picture version bound on each channel (`None`: no picture yet).
    pictures: [Option<u64>; 10],
}

#[derive(Debug)]
struct GpuMaterial {
    signature: Signature,
    buffer: wgpu::Buffer,
    group: wgpu::BindGroup,
    last_used: u64,
}

/// Materials not drawn for this many frames lose their bind group.
const MATERIAL_TTL_FRAMES: u64 = 600;

/// The pictures and material bind groups of the viewport.
#[derive(Debug)]
pub(crate) struct Textures {
    pub layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// A 1 × 1 white picture for channels without one (written on the first
    /// frame: making the renderer needs no queue).
    white: GpuTexture,
    white_written: bool,
    pictures: HashMap<TextureId, GpuTexture>,
    materials: HashMap<MaterialId, GpuMaterial>,
    /// The bind group of untextured objects.
    default_group: wgpu::BindGroup,
    _default_buffer: wgpu::Buffer,
}

impl Textures {
    pub fn new(device: &wgpu::Device) -> Self {
        let mut entries = vec![
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(size_of::<MaterialUniform>() as u64),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ];
        for binding in 2..12 {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
        }
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grid4d.material.layout"),
            entries: &entries,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("grid4d.material.sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        let white_texture = create_texture(device, "grid4d.texture.white", TexelFormat::Linear8, 1, 1, 1);
        let white = GpuTexture {
            view: white_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            texture: white_texture,
            version: 0,
            grey: true,
            bytes: 4,
        };
        let default_buffer = uniform_buffer(device, &MaterialUniform::empty());
        let views = [&white.view; 10];
        let default_group = bind_group(device, &layout, &default_buffer, &sampler, views);
        Self {
            layout,
            sampler,
            white,
            white_written: false,
            pictures: HashMap::new(),
            materials: HashMap::new(),
            default_group,
            _default_buffer: default_buffer,
        }
    }

    /// Writes the white picture once.
    pub fn begin_frame(&mut self, queue: &wgpu::Queue) {
        if !self.white_written {
            write_levels(
                queue,
                &self.white.texture,
                TexelFormat::Linear8,
                &[TextureLevel {
                    width: 1,
                    height: 1,
                    bytes: &[255; 4],
                }],
            );
            self.white_written = true;
        }
    }

    /// Picture `id` on the GPU: its version, its view and its width.
    pub fn view(&self, id: TextureId) -> Option<(u64, &wgpu::TextureView, u32)> {
        self.pictures.get(&id).map(|t| (t.version, &t.view, t.texture.width()))
    }

    /// The 1 × 1 white picture.
    pub const fn white_view(&self) -> &wgpu::TextureView {
        &self.white.view
    }

    /// Whether picture `id` is on the GPU at `version`.
    pub fn has(&self, id: TextureId, version: u64) -> bool {
        self.pictures.get(&id).is_some_and(|t| t.version == version)
    }

    /// Puts picture `id` (its mip chain, largest first) on the GPU, replacing
    /// an older version.
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: TextureId,
        version: u64,
        format: TexelFormat,
        levels: &[TextureLevel<'_>],
    ) {
        if levels.is_empty() {
            return;
        }
        // Within the device's largest side and what the budget leaves.
        let others: u64 = self
            .pictures
            .iter()
            .filter(|&(&other, _)| other != id)
            .map(|(_, t)| t.bytes)
            .sum();
        let max_side = device.limits().max_texture_dimension_2d;
        let first = first_fitting(format, levels, max_side, TEXTURE_BUDGET.saturating_sub(others));
        let levels = levels.get(first..).unwrap_or(levels);
        let texture = make_texture(device, queue, "grid4d.texture", format, levels, version);
        self.pictures.insert(id, texture);
    }

    /// Shows picture `id` as missing: a magenta and black checker.
    pub fn missing(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, id: TextureId) {
        const SIDE: u32 = 64;
        let mut bytes = Vec::with_capacity(64 * 64 * 4);
        for y in 0..SIDE {
            for x in 0..SIDE {
                let on = (x / 8 + y / 8) % 2 == 0;
                bytes.extend(if on { [255, 0, 255, 255] } else { [0, 0, 0, 255] });
            }
        }
        let level = TextureLevel {
            width: SIDE,
            height: SIDE,
            bytes: &bytes,
        };
        let texture = make_texture(
            device,
            queue,
            "grid4d.texture.missing",
            TexelFormat::Srgb8,
            &[level],
            MISSING_VERSION,
        );
        self.pictures.insert(id, texture);
    }

    /// Frees the pictures `keep` refuses.
    pub fn retain(&mut self, keep: impl Fn(TextureId) -> bool) {
        self.pictures.retain(|&id, _| keep(id));
    }

    /// Makes sure `material`'s bind group is up to date (its slots, its
    /// pictures' versions); `frame` marks it used.
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, material: &MaterialData, frame: u64) {
        let pictures = material
            .maps
            .map(|slot| slot.and_then(|s| self.pictures.get(&s.texture)).map(|t| t.version));
        let signature = Signature {
            material: *material,
            pictures,
        };
        if let Some(gpu) = self.materials.get_mut(&material.id) {
            gpu.last_used = frame;
            if gpu.signature == signature {
                return;
            }
        }
        let mut uniform = MaterialUniform::empty();
        uniform.header = [
            material.emission_strength,
            material.opacity,
            material.opacity_threshold,
            material.specular,
        ];
        // Subsurface softens the matte part: lighting wraps up to half way
        // round to the shadow side.
        uniform.shape = [
            material.normal_strength,
            material.bump_height,
            material.subsurface.clamp(0.0, 1.0) * 0.5,
            0.0,
        ];
        let mut views = [&self.white.view; 10];
        for (index, channel) in Channel::ALL.into_iter().enumerate() {
            let Some(slot) = material.maps[index] else { continue };
            let Some(picture) = self.pictures.get(&slot.texture) else {
                continue;
            };
            views[index] = &picture.view;
            // 1: a picture; 2: a grey one on a colour channel; 4 × the
            // picture's channel a number channel reads (0 – 3).
            let grey = if picture.grey && channel.is_colour() { 2.0 } else { 0.0 };
            let source = f32::from(u8::try_from(slot.source.index()).unwrap_or(0)) * 4.0;
            let flags = 1.0 + grey + source;
            uniform.channels[index] = [
                slot.tiling[0],
                slot.tiling[1],
                slot.offset[0],
                slot.offset[1],
                slot.range[0],
                slot.range[1],
                slot.strength,
                flags,
            ];
        }
        match self.materials.get_mut(&material.id) {
            Some(gpu) if gpu.signature.pictures == signature.pictures => {
                // Same pictures: only the settings changed.
                queue.write_buffer(&gpu.buffer, 0, bytemuck::bytes_of(&uniform));
                gpu.signature = signature;
            }
            _ => {
                let buffer = uniform_buffer(device, &uniform);
                let group = bind_group(device, &self.layout, &buffer, &self.sampler, views);
                self.materials.insert(
                    material.id,
                    GpuMaterial {
                        signature,
                        buffer,
                        group,
                        last_used: frame,
                    },
                );
            }
        }
    }

    /// The bind group for objects of `material` (the default one without).
    pub fn group(&self, material: Option<MaterialId>) -> &wgpu::BindGroup {
        material
            .and_then(|id| self.materials.get(&id))
            .map_or(&self.default_group, |gpu| &gpu.group)
    }

    /// Frees bind groups of materials not drawn for a while.
    pub fn evict(&mut self, frame: u64) {
        self.materials
            .retain(|_, gpu| gpu.last_used + MATERIAL_TTL_FRAMES > frame);
    }
}

fn uniform_buffer(device: &wgpu::Device, uniform: &MaterialUniform) -> wgpu::Buffer {
    use wgpu::util::DeviceExt as _;
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("grid4d.material.uniform"),
        contents: bytemuck::bytes_of(uniform),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    })
}

fn bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffer: &wgpu::Buffer,
    sampler: &wgpu::Sampler,
    views: [&wgpu::TextureView; 10],
) -> wgpu::BindGroup {
    let mut entries = vec![
        wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
            binding: 1,
            resource: wgpu::BindingResource::Sampler(sampler),
        },
    ];
    for (binding, view) in (2..).zip(views) {
        entries.push(wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        });
    }
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("grid4d.material.bind_group"),
        layout,
        entries: &entries,
    })
}

/// The bytes the levels `levels` take on the GPU.
fn chain_bytes(format: TexelFormat, levels: &[TextureLevel<'_>]) -> u64 {
    levels
        .iter()
        .map(|l| u64::from(l.width) * u64::from(l.height) * u64::from(format.texel_bytes()))
        .sum()
}

/// The first level of `levels` whose sides are at most `max_side` and whose
/// chain from there takes at most `budget` bytes (the last level if none).
fn first_fitting(format: TexelFormat, levels: &[TextureLevel<'_>], max_side: u32, budget: u64) -> usize {
    let last = levels.len().saturating_sub(1);
    (0..levels.len())
        .find(|&i| {
            let fits = |level: &TextureLevel<'_>| level.width.max(level.height) <= max_side;
            levels.get(i).is_some_and(fits) && chain_bytes(format, levels.get(i..).unwrap_or(&[])) <= budget
        })
        .unwrap_or(last)
}

/// The levels of `levels` that make a valid mip chain: each half the one
/// before (rounded down, at least 1) and holding all its texels.
fn valid_levels(format: TexelFormat, levels: &[TextureLevel<'_>]) -> usize {
    let Some(first) = levels.first() else { return 0 };
    let (mut width, mut height) = (first.width.max(1), first.height.max(1));
    let mut count = 0;
    for level in levels {
        let needed = u64::from(level.width) * u64::from(level.height) * u64::from(format.texel_bytes());
        if level.width != width || level.height != height || (level.bytes.len() as u64) < needed {
            break;
        }
        count += 1;
        width = (width / 2).max(1);
        height = (height / 2).max(1);
    }
    count
}

/// Whether every texel of `level` has equal red, green and blue.
fn is_grey(format: TexelFormat, level: &TextureLevel<'_>) -> bool {
    match format {
        TexelFormat::Srgb8 | TexelFormat::Linear8 => level
            .bytes
            .as_chunks::<4>()
            .0
            .iter()
            .all(|[r, g, b, _]| r == g && g == b),
        TexelFormat::Float16 => level
            .bytes
            .as_chunks::<8>()
            .0
            .iter()
            .all(|t| t[0..2] == t[2..4] && t[2..4] == t[4..6]),
    }
}

fn make_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &'static str,
    format: TexelFormat,
    levels: &[TextureLevel<'_>],
    version: u64,
) -> GpuTexture {
    let count = valid_levels(format, levels).max(1);
    let fallback = TextureLevel {
        width: 1,
        height: 1,
        bytes: &[255, 0, 255, 255, 0, 0, 0, 0],
    };
    let levels = if valid_levels(format, levels) == 0 {
        std::slice::from_ref(&fallback)
    } else {
        &levels[..count]
    };
    let first = levels[0];
    let texture = create_texture(
        device,
        label,
        format,
        first.width,
        first.height,
        u32::try_from(levels.len()).unwrap_or(1),
    );
    write_levels(queue, &texture, format, levels);
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    GpuTexture {
        grey: is_grey(format, &first),
        bytes: chain_bytes(format, levels),
        texture,
        view,
        version,
    }
}

fn create_texture(
    device: &wgpu::Device,
    label: &'static str,
    format: TexelFormat,
    width: u32,
    height: u32,
    mips: u32,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: format.wgpu(),
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

/// Writes `levels` into `texture`'s mip levels, largest first.
fn write_levels(queue: &wgpu::Queue, texture: &wgpu::Texture, format: TexelFormat, levels: &[TextureLevel<'_>]) {
    for (mip, level) in (0u32..).zip(levels) {
        let needed =
            usize::try_from(u64::from(level.width) * u64::from(level.height) * u64::from(format.texel_bytes()))
                .unwrap_or(usize::MAX);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: mip,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            level.bytes.get(..needed).unwrap_or(level.bytes),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(level.width * format.texel_bytes()),
                rows_per_image: Some(level.height),
            },
            wgpu::Extent3d {
                width: level.width,
                height: level.height,
                depth_or_array_layers: 1,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_uniform_is_16_byte_aligned() {
        assert_eq!(size_of::<MaterialUniform>(), 352);
        assert_eq!(size_of::<MaterialUniform>() % 16, 0);
    }

    #[test]
    fn pictures_too_large_go_up_from_a_smaller_level() {
        let bytes = [0u8; 256];
        let level = |side| TextureLevel {
            width: side,
            height: side,
            bytes: &bytes,
        };
        let chain = [level(8), level(4), level(2), level(1)];
        // 8×8 + 4×4 + 2×2 + 1 texels of 4 bytes.
        assert_eq!(chain_bytes(TexelFormat::Srgb8, &chain), 340);
        assert_eq!(first_fitting(TexelFormat::Srgb8, &chain, 16, 1000), 0);
        assert_eq!(
            first_fitting(TexelFormat::Srgb8, &chain, 4, 1000),
            1,
            "the device's largest side"
        );
        assert_eq!(first_fitting(TexelFormat::Srgb8, &chain, 16, 84), 1, "the budget");
        assert_eq!(
            first_fitting(TexelFormat::Srgb8, &chain, 16, 0),
            3,
            "at least the smallest"
        );
    }

    #[test]
    fn a_mip_chain_is_cut_where_it_stops_halving() {
        let bytes = [0u8; 64];
        let level = |width, height| TextureLevel {
            width,
            height,
            bytes: &bytes,
        };
        let chain = [level(4, 2), level(2, 1), level(1, 1)];
        assert_eq!(valid_levels(TexelFormat::Linear8, &chain), 3);
        assert_eq!(valid_levels(TexelFormat::Linear8, &[level(4, 2), level(3, 1)]), 1);
        assert_eq!(valid_levels(TexelFormat::Float16, &[level(4, 4)]), 0, "too few bytes");
        let grey = [9u8, 9, 9, 255, 3, 3, 3, 0];
        assert!(is_grey(
            TexelFormat::Srgb8,
            &TextureLevel {
                width: 2,
                height: 1,
                bytes: &grey
            }
        ));
        let red = [9u8, 0, 0, 255];
        assert!(!is_grey(
            TexelFormat::Srgb8,
            &TextureLevel {
                width: 1,
                height: 1,
                bytes: &red
            }
        ));
    }
}
