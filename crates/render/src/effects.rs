//! Viewport effects (see `docs/reference/viewport-effects.md`): the key light's
//! shadow map and screen-space ambient occlusion, each switched on per view.
//!
//! Both run as passes before the view's main pass: the shadow map is drawn from
//! the light ([`ShadowMaps`]), the occlusion from a normal-and-depth pre-pass of
//! the view ([`Ssao`], with textures per view target in [`SsaoTextures`]). The
//! main pass samples both.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, camera::rh};

use crate::{LightRaw, Vertex, gpu::INSTANCE_LAYOUT, shaders};

/// Which effects a view draws (its Options menu); both off by default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ViewEffects {
    /// The key light's shadow (Gouraud Shading).
    pub shadows: bool,
    /// Screen-space ambient occlusion (Gouraud and Quick Shading).
    pub ssao: bool,
}

/// Pixels per side of each shadow-map face.
pub(crate) const SHADOW_MAP_SIZE: u32 = 2048;
const SHADOW_MAP_PIXELS: f32 = 2048.0;
/// An Omni light's six views are a little wider than 90°, so filtering near a
/// face's edge stays on that face.
const OMNI_FOV_DEG: f32 = 95.0;
/// Uniform offsets must be multiples of this.
const FACE_STRIDE: u64 = 256;
const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Where the key light's shadow map looks from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ShadowView {
    /// One view-projection per face: one for Spot and Infinite lights, six for
    /// an Omni light (+X, −X, +Y, −Y, +Z, −Z).
    pub faces: [Mat4; 6],
    pub count: u32,
    /// A shadow-map texel's size at the scene's centre, in world units.
    pub texel: f32,
}

/// The views of `light`'s shadow map that cover the scene between `min` and
/// `max` (world bounds).
pub(crate) fn shadow_view(light: &LightRaw, min: Vec3, max: Vec3) -> ShadowView {
    let centre = (min + max) * 0.5;
    let radius = ((max - min).length() * 0.5).max(1.0);
    let position = Vec3::from_slice(&light.position[..3]);
    let direction = Vec3::from_slice(&light.direction[..3]).normalize_or(Vec3::NEG_Z);
    let up = |d: Vec3| if d.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
    let mut faces = [Mat4::IDENTITY; 6];
    // `position.w`: 0 omni, 1 spot, 2 infinite.
    if light.position[3] > 1.5 {
        // Parallel light over the whole scene.
        let eye = centre - direction * radius * 2.0;
        let view = rh::view::look_at_mat4(eye, centre, up(direction));
        let proj = rh::proj::directx::orthographic(-radius, radius, -radius, radius, radius * 0.01, radius * 4.0);
        faces[0] = proj * view;
        return ShadowView {
            faces,
            count: 1,
            texel: 2.0 * radius / SHADOW_MAP_PIXELS,
        };
    }
    let distance = position.distance(centre);
    let far = (distance + radius).max(1.0);
    let near = (far * 0.0005).max(0.05);
    let texel = |fov: f32| 2.0 * distance.max(near) * (fov * 0.5).tan() / SHADOW_MAP_PIXELS;
    if light.position[3] > 0.5 {
        // Through the spot's cone (`direction.w`: cosine of half its opening).
        let opening = 2.0 * light.direction[3].clamp(-1.0, 1.0).acos();
        let fov = (opening + 0.1).clamp(0.1, 170f32.to_radians());
        let view = rh::view::look_at_mat4(position, position + direction, up(direction));
        faces[0] = rh::proj::directx::perspective(fov, 1.0, near, far) * view;
        return ShadowView {
            faces,
            count: 1,
            texel: texel(fov),
        };
    }
    let fov = OMNI_FOV_DEG.to_radians();
    let proj = rh::proj::directx::perspective(fov, 1.0, near, far);
    for (face, axis) in [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z]
        .into_iter()
        .enumerate()
    {
        faces[face] = proj * rh::view::look_at_mat4(position, position + axis, up(axis));
    }
    ShadowView {
        faces,
        count: 6,
        texel: texel(fov),
    }
}

/// What the main pass reads about the shadow. Layout must match `Shadow` in
/// `shaders/mesh.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct ShadowUniform {
    faces: [[[f32; 4]; 4]; 6],
    /// x: the shadow-casting light's index in the light list, y: faces (0: no
    /// shadow), z: map size in pixels, w: offset along the normal (world units).
    params: [f32; 4],
    /// x: filter radius in texels, y: depth bias.
    blur: [f32; 4],
}

impl ShadowUniform {
    /// No shadow.
    pub(crate) fn none() -> Self {
        Self::zeroed()
    }

    /// The shadow of light `light` (its index), seen through `view`; `hard`:
    /// sharp-edged.
    pub(crate) fn new(view: &ShadowView, light: u16, hard: bool) -> Self {
        Self {
            faces: view.faces.map(|m| m.to_cols_array_2d()),
            params: [
                f32::from(light),
                f32::from(u8::try_from(view.count).unwrap_or(0)),
                SHADOW_MAP_PIXELS,
                view.texel * 1.5,
            ],
            blur: [if hard { 1.0 } else { 4.0 }, 0.0002, 0.0, 0.0],
        }
    }
}

/// The shadow map (a depth texture with a layer per face), the pass that draws
/// it, and what the main pass samples it with.
#[derive(Debug)]
pub(crate) struct ShadowMaps {
    pipeline: wgpu::RenderPipeline,
    /// One view-projection per face, at `FACE_STRIDE` offsets.
    face_buffer: wgpu::Buffer,
    face_bind_group: wgpu::BindGroup,
    size: u32,
    layers: u32,
    array_view: wgpu::TextureView,
    layer_views: Vec<wgpu::TextureView>,
    /// The main pass's [`ShadowUniform`].
    pub(crate) uniform: wgpu::Buffer,
    pub(crate) sampler: wgpu::Sampler,
}

impl ShadowMaps {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let face_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grid4d.shadow.face.layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(size_of::<[[f32; 4]; 4]>() as u64),
                },
                count: None,
            }],
        });
        let face_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grid4d.shadow.faces"),
            size: FACE_STRIDE * 6,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let face_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grid4d.shadow.face.bind_group"),
            layout: &face_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &face_buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(size_of::<[[f32; 4]; 4]>() as u64),
                }),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("grid4d.shadow.layout"),
            bind_group_layouts: &[Some(&face_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("grid4d.shadow.wgsl"),
            source: wgpu::ShaderSource::Wgsl(shaders::SHADOW.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("grid4d.shadow.pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_shadow"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(vertex_layout()), Some(INSTANCE_LAYOUT)],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SHADOW_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                // Against "shadow acne": surfaces shadowing themselves.
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.5,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grid4d.shadow.uniform"),
            size: size_of::<ShadowUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("grid4d.shadow.sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        // A 1 × 1 placeholder until a view first draws shadows.
        let (array_view, layer_views) = shadow_texture(device, 1, 1);
        Self {
            pipeline,
            face_buffer,
            face_bind_group,
            size: 1,
            layers: 1,
            array_view,
            layer_views,
            uniform,
            sampler,
        }
    }

    /// The shadow map as the main pass samples it.
    pub(crate) fn array_view(&self) -> &wgpu::TextureView {
        &self.array_view
    }

    /// Makes the shadow map full size with at least `layers` faces. `true` if it
    /// was recreated: bind groups that use it must be recreated too.
    pub(crate) fn ensure(&mut self, device: &wgpu::Device, layers: u32) -> bool {
        if self.size == SHADOW_MAP_SIZE && self.layers >= layers {
            return false;
        }
        let layers = layers.max(self.layers);
        (self.array_view, self.layer_views) = shadow_texture(device, SHADOW_MAP_SIZE, layers);
        (self.size, self.layers) = (SHADOW_MAP_SIZE, layers);
        true
    }

    /// Uploads the faces' view-projections for the shadow pass.
    pub(crate) fn write_faces(&self, queue: &wgpu::Queue, view: &ShadowView) {
        for (face, matrix) in view.faces.iter().take(6).enumerate() {
            queue.write_buffer(
                &self.face_buffer,
                FACE_STRIDE * face as u64,
                bytemuck::cast_slice(&matrix.to_cols_array()),
            );
        }
    }

    /// Opens the pass that draws face `face` of the shadow map, its pipeline and
    /// face bound; the caller binds the instances and draws.
    pub(crate) fn begin_face<'a>(&'a self, encoder: &'a mut wgpu::CommandEncoder, face: u32) -> wgpu::RenderPass<'a> {
        let index = usize::try_from(face)
            .unwrap_or(0)
            .min(self.layer_views.len().saturating_sub(1));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("grid4d.shadow.pass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.layer_views[index],
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        let offset = u32::try_from(FACE_STRIDE).unwrap_or(256) * face;
        pass.set_bind_group(0, &self.face_bind_group, &[offset]);
        pass
    }
}

/// A shadow-map texture of `layers` faces, `size` pixels square: the view the
/// main pass samples and one view per face to draw into.
fn shadow_texture(device: &wgpu::Device, size: u32, layers: u32) -> (wgpu::TextureView, Vec<wgpu::TextureView>) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("grid4d.shadow.map"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: layers,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SHADOW_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let array_view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("grid4d.shadow.map.array"),
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    let layer_views = (0..layers)
        .map(|layer| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("grid4d.shadow.map.face"),
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: layer,
                array_layer_count: Some(1),
                ..Default::default()
            })
        })
        .collect();
    (array_view, layer_views)
}

/// What the occlusion passes read. Layout must match `Ssao` in `shaders/ssao.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct SsaoUniform {
    view: [[f32; 4]; 4],
    proj: [[f32; 4]; 4],
    /// xy: target size in pixels, z: radius (world units), w: power.
    params: [f32; 4],
    /// x: depth bias (world units), y: 1 for a parallel camera.
    extra: [f32; 4],
}

/// How far around a point the occlusion looks, in cm.
const SSAO_RADIUS: f32 = 30.0;
/// The occlusion's contrast.
const SSAO_POWER: f32 = 2.0;

impl SsaoUniform {
    pub(crate) fn new(view: Mat4, proj: Mat4, size: [u32; 2], parallel: bool) -> Self {
        let px = |n: u32| f32::from(u16::try_from(n).unwrap_or(u16::MAX));
        Self {
            view: view.to_cols_array_2d(),
            proj: proj.to_cols_array_2d(),
            params: [px(size[0]), px(size[1]), SSAO_RADIUS, SSAO_POWER],
            extra: [SSAO_RADIUS * 0.03, f32::from(u8::from(parallel)), 0.0, 0.0],
        }
    }
}

const NORMAL_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const OCCLUSION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// The layouts per-view occlusion textures are bound with.
#[derive(Debug)]
pub(crate) struct SsaoLayouts {
    /// One texture the occlusion and blur passes read.
    input: wgpu::BindGroupLayout,
    /// The occlusion as the main pass samples it (its group 1).
    pub(crate) ao: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

/// The occlusion passes: their pipelines and uniform. Textures are per view
/// ([`SsaoTextures`]).
#[derive(Debug)]
pub(crate) struct Ssao {
    uniform: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    pub(crate) layouts: SsaoLayouts,
    prepass: wgpu::RenderPipeline,
    occlusion: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    /// The main pass's group 1 without SSAO (a 1 × 1 texture it does not read).
    pub(crate) unused: wgpu::BindGroup,
}

impl Ssao {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grid4d.ssao.uniform.layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(size_of::<SsaoUniform>() as u64),
                },
                count: None,
            }],
        });
        let texture_entry = |binding, filterable| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let input = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grid4d.ssao.input.layout"),
            entries: &[texture_entry(0, false)],
        });
        let ao = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grid4d.ssao.ao.layout"),
            entries: &[
                texture_entry(0, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("grid4d.ssao.sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grid4d.ssao.uniform"),
            size: size_of::<SsaoUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grid4d.ssao.uniform.bind_group"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("grid4d.ssao.wgsl"),
            source: wgpu::ShaderSource::Wgsl(shaders::SSAO.into()),
        });
        let prepass_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("grid4d.ssao.prepass.layout"),
            bind_group_layouts: &[Some(&uniform_layout)],
            immediate_size: 0,
        });
        let prepass = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("grid4d.ssao.prepass"),
            layout: Some(&prepass_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_prepass"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(vertex_layout()), Some(INSTANCE_LAYOUT)],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_prepass"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(NORMAL_DEPTH_FORMAT.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let screen_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("grid4d.ssao.screen.layout"),
            bind_group_layouts: &[Some(&uniform_layout), Some(&input)],
            immediate_size: 0,
        });
        let screen = |label, entry| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&screen_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_fullscreen"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(OCCLUSION_FORMAT.into())],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let occlusion = screen("grid4d.ssao.occlusion", "fs_ssao");
        let blur = screen("grid4d.ssao.blur", "fs_blur");
        let placeholder = texture_view(device, "grid4d.ssao.unused", [1, 1], OCCLUSION_FORMAT, true);
        let unused = ao_bind_group(device, &ao, &placeholder, &sampler);
        Self {
            uniform,
            uniform_bind_group,
            layouts: SsaoLayouts { input, ao, sampler },
            prepass,
            occlusion,
            blur,
            unused,
        }
    }

    pub(crate) fn write(&self, queue: &wgpu::Queue, uniform: &SsaoUniform) {
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(uniform));
    }

    /// Records the three occlusion passes into `textures`; `draw` draws the
    /// view's surfaces (vertex buffer 0: [`Vertex`], 1: the instances).
    pub(crate) fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        textures: &SsaoTextures,
        draw: impl FnOnce(&mut wgpu::RenderPass<'_>),
    ) {
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("grid4d.ssao.prepass"),
                color_attachments: &[Some(colour_attachment(
                    &textures.normal_depth,
                    wgpu::Color::TRANSPARENT,
                ))],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &textures.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.prepass);
            pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            draw(&mut pass);
        }
        for (pipeline, input, output, label) in [
            (
                &self.occlusion,
                &textures.normals_input,
                &textures.occlusion,
                "grid4d.ssao.occlusion",
            ),
            (
                &self.blur,
                &textures.occlusion_input,
                &textures.blurred,
                "grid4d.ssao.blur",
            ),
        ] {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(colour_attachment(output, wgpu::Color::WHITE))],
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            pass.set_bind_group(1, input, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

/// One view's occlusion textures, made when the view first draws SSAO and
/// again when its size changes.
#[derive(Debug)]
pub(crate) struct SsaoTextures {
    normal_depth: wgpu::TextureView,
    depth: wgpu::TextureView,
    occlusion: wgpu::TextureView,
    blurred: wgpu::TextureView,
    normals_input: wgpu::BindGroup,
    occlusion_input: wgpu::BindGroup,
    /// The blurred occlusion for the main pass (its group 1).
    pub(crate) ao: wgpu::BindGroup,
}

impl SsaoTextures {
    pub(crate) fn new(device: &wgpu::Device, size: [u32; 2], layouts: &SsaoLayouts) -> Self {
        let normal_depth = texture_view(device, "grid4d.ssao.normal_depth", size, NORMAL_DEPTH_FORMAT, true);
        let depth = texture_view(
            device,
            "grid4d.ssao.depth",
            size,
            wgpu::TextureFormat::Depth32Float,
            false,
        );
        let occlusion = texture_view(device, "grid4d.ssao.occlusion", size, OCCLUSION_FORMAT, true);
        let blurred = texture_view(device, "grid4d.ssao.blurred", size, OCCLUSION_FORMAT, true);
        let input = |view: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("grid4d.ssao.input"),
                layout: &layouts.input,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                }],
            })
        };
        Self {
            normals_input: input(&normal_depth),
            occlusion_input: input(&occlusion),
            ao: ao_bind_group(device, &layouts.ao, &blurred, &layouts.sampler),
            normal_depth,
            depth,
            occlusion,
            blurred,
        }
    }
}

fn ao_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("grid4d.ssao.ao"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

/// A 2D texture drawn into (and read, when `sampled`).
fn texture_view(
    device: &wgpu::Device,
    label: &'static str,
    size: [u32; 2],
    format: wgpu::TextureFormat,
    sampled: bool,
) -> wgpu::TextureView {
    let mut usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
    if sampled {
        usage |= wgpu::TextureUsages::TEXTURE_BINDING;
    }
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size[0].max(1),
                height: size[1].max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

fn colour_attachment(view: &wgpu::TextureView, clear: wgpu::Color) -> wgpu::RenderPassColorAttachment<'_> {
    wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(clear),
            store: wgpu::StoreOp::Store,
        },
    }
}

/// Surface vertices: position (location 0) and normal (location 1).
fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
    wgpu::VertexBufferLayout {
        array_stride: size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRIBUTES,
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec4Swizzles as _;

    use super::*;

    fn light(kind: f32, position: Vec3, direction: Vec3, cos_outer: f32) -> LightRaw {
        LightRaw {
            position: position.extend(kind).to_array(),
            direction: direction.extend(cos_outer).to_array(),
            color: [1.0, 1.0, 1.0, cos_outer],
            falloff: [0.0; 4],
        }
    }

    fn ndc(m: &Mat4, p: Vec3) -> Vec3 {
        let clip = *m * p.extend(1.0);
        clip.xyz() / clip.w
    }

    fn inside(p: Vec3) -> bool {
        p.x.abs() <= 1.0 + 1e-4 && p.y.abs() <= 1.0 + 1e-4 && (-1e-4..=1.0 + 1e-4).contains(&p.z)
    }

    const MIN: Vec3 = Vec3::new(-300.0, 0.0, -200.0);
    const MAX: Vec3 = Vec3::new(300.0, 200.0, 200.0);

    fn corners() -> impl Iterator<Item = Vec3> {
        (0..8u8).map(|i| {
            Vec3::new(
                if i & 1 == 0 { MIN.x } else { MAX.x },
                if i & 2 == 0 { MIN.y } else { MAX.y },
                if i & 4 == 0 { MIN.z } else { MAX.z },
            )
        })
    }

    #[test]
    fn an_infinite_light_sees_the_whole_scene_along_its_direction() {
        let down = Vec3::new(0.3, -1.0, 0.2).normalize();
        let view = shadow_view(&light(2.0, Vec3::new(5000.0, 0.0, 0.0), down, 1.0), MIN, MAX);
        assert_eq!(view.count, 1);
        assert!(
            corners().all(|c| inside(ndc(&view.faces[0], c))),
            "every corner in view"
        );
        // Farther along the light means deeper.
        let centre = (MIN + MAX) * 0.5;
        assert!(ndc(&view.faces[0], centre + down * 50.0).z > ndc(&view.faces[0], centre).z);
        assert!(view.texel > 0.0);
    }

    #[test]
    fn a_spot_looks_through_its_cone_and_an_omni_light_every_way() {
        let position = Vec3::new(0.0, 600.0, 300.0);
        let towards = (Vec3::ZERO - position).normalize();
        let spot = shadow_view(&light(1.0, position, towards, 30f32.to_radians().cos()), MIN, MAX);
        assert_eq!(spot.count, 1);
        let centre = ndc(&spot.faces[0], Vec3::ZERO);
        assert!(
            centre.x.abs() < 1e-4 && centre.y.abs() < 1e-4 && inside(centre),
            "{centre}"
        );

        let omni = shadow_view(&light(0.0, Vec3::new(0.0, 100.0, 0.0), Vec3::NEG_Z, 1.0), MIN, MAX);
        assert_eq!(omni.count, 6);
        for (face, axis) in [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z]
            .into_iter()
            .enumerate()
        {
            let p = ndc(&omni.faces[face], Vec3::new(0.0, 100.0, 0.0) + axis * 150.0);
            assert!(p.x.abs() < 1e-4 && p.y.abs() < 1e-4 && inside(p), "face {face}: {p}");
        }
    }

    #[test]
    fn uniform_layouts_are_16_byte_aligned() {
        assert_eq!(size_of::<ShadowUniform>() % 16, 0);
        assert_eq!(size_of::<SsaoUniform>() % 16, 0);
        assert_eq!(size_of::<ShadowUniform>(), 416);
    }
}
