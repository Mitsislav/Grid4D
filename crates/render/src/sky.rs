//! What surrounds the scene in the viewport (see
//! `docs/reference/render-environment.md`): the background a Sky or an
//! Environment shows behind everything (`shaders/sky.wgsl`), and the soft
//! light they give surfaces — nine spherical-harmonic numbers per colour,
//! made once per change (a Sky's here, a picture's by whoever decodes it),
//! that the mesh shader evaluates per pixel.

use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Mat4, Vec3};
use grid4d_eval::{Ambient, EvaluatedEnvironment, SkyModel, Surround};
use grid4d_scene::TextureId;

use crate::{GpuTargetConfig, shaders};

/// What the background shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Background {
    /// Nothing: the view's gradient (not visible to the camera, or filtered
    /// out).
    None,
    Sky(SkyModel),
    Picture {
        texture: TextureId,
        to_picture: Mat3,
        intensity: f32,
    },
    /// An Environment without a picture (yet).
    Plain(Vec3),
}

/// The surround as one frame draws it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SurroundFrame {
    pub background: Background,
    /// The light all round, its harmonics in the frame `to_frame` turns world
    /// directions into.
    pub ambient: Ambient,
    pub to_frame: Mat3,
}

impl SurroundFrame {
    /// The surround `environment` describes; `picture_ambient` the harmonics
    /// of a picture (in its own frame) when they are known; `sky_ambient` a
    /// cache of the last Sky's, made again only when the Sky changes;
    /// `shown`: its background is drawn (visible to the camera, not
    /// filtered out).
    pub fn new(
        environment: &EvaluatedEnvironment,
        picture_ambient: Option<(TextureId, Ambient)>,
        sky_ambient: &mut Option<(SkyModel, Ambient)>,
        shown: bool,
    ) -> Self {
        match environment.surround {
            Surround::Sky(model) => {
                let ambient = match sky_ambient {
                    Some((cached, ambient)) if *cached == model => *ambient,
                    _ => {
                        let ambient = Ambient::project(16, |d| model.radiance(d));
                        *sky_ambient = Some((model, ambient));
                        ambient
                    }
                };
                Self {
                    background: if shown {
                        Background::Sky(model)
                    } else {
                        Background::None
                    },
                    ambient,
                    to_frame: Mat3::IDENTITY,
                }
            }
            Surround::Picture(surround) => {
                let known = surround
                    .texture
                    .zip(picture_ambient)
                    .filter(|(texture, (id, _))| texture == id)
                    .map(|(texture, (_, ambient))| (texture, ambient));
                match known {
                    Some((texture, ambient)) => Self {
                        background: if shown {
                            Background::Picture {
                                texture,
                                to_picture: surround.to_picture,
                                intensity: surround.intensity,
                            }
                        } else {
                            Background::None
                        },
                        ambient: ambient.scaled(surround.intensity),
                        to_frame: surround.to_picture,
                    },
                    // No picture, or not read yet: the plain grey.
                    None => Self {
                        background: if shown {
                            Background::Plain(surround.plain())
                        } else {
                            Background::None
                        },
                        ambient: Ambient::uniform(surround.plain()),
                        to_frame: Mat3::IDENTITY,
                    },
                }
            }
        }
    }
}

/// Layout must match `Sky` in `shaders/sky.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct SkyUniform {
    inverse_view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    mode: [f32; 4],
    perez: [[f32; 4]; 5],
    scale: [f32; 4],
    model_sun: [f32; 4],
    ground: [f32; 4],
    sun: [f32; 4],
    sun_color: [f32; 4],
    to_picture: [[f32; 4]; 3],
    plain: [f32; 4],
}

impl SkyUniform {
    /// The uniform for `background` seen through `view_proj` from `eye`;
    /// `mip` the picture's level to read (one texel per pixel or so).
    pub fn new(background: &Background, view_proj: Mat4, eye: Vec3, encode_srgb: bool, mip: f32) -> Self {
        let mut uniform = Self {
            inverse_view_proj: view_proj.inverse().to_cols_array_2d(),
            eye: eye.extend(f32::from(u8::from(encode_srgb))).to_array(),
            mode: [0.0; 4],
            perez: [[0.0; 4]; 5],
            scale: [0.0; 4],
            model_sun: [0.0, 1.0, 0.0, 0.0],
            ground: [0.0; 4],
            sun: [0.0, 1.0, 0.0, 2.0],
            sun_color: [0.0; 4],
            to_picture: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
            plain: [0.0; 4],
        };
        match *background {
            Background::None => {}
            Background::Sky(model) => {
                let (coefficients, scale, model_sun, ground, gain) = model.shader_values();
                uniform.mode[0] = 1.0;
                for (i, row) in uniform.perez.iter_mut().enumerate() {
                    *row = [coefficients[0][i], coefficients[1][i], coefficients[2][i], 0.0];
                }
                uniform.scale = [scale[0], scale[1], scale[2], gain];
                uniform.model_sun = model_sun.extend(0.0).to_array();
                uniform.ground = ground.extend(1.0).to_array();
                if let Some(sun) = model.sun() {
                    // A disc of its colour, as bright as the view shows.
                    let colour = sun.radiance / sun.radiance.max_element().max(f32::MIN_POSITIVE);
                    uniform.sun = sun.direction.extend(sun.cos_radius).to_array();
                    uniform.sun_color = colour.extend(1.0).to_array();
                }
            }
            Background::Picture {
                to_picture, intensity, ..
            } => {
                uniform.mode = [2.0, intensity, mip, 0.0];
                let rows = to_picture.transpose();
                uniform.to_picture = [
                    rows.x_axis.extend(0.0).to_array(),
                    rows.y_axis.extend(0.0).to_array(),
                    rows.z_axis.extend(0.0).to_array(),
                ];
            }
            Background::Plain(colour) => {
                uniform.mode[0] = 3.0;
                uniform.plain = colour.extend(1.0).to_array();
            }
        }
        uniform
    }
}

/// The background's pipeline, its uniform and the bind group of the picture
/// it shows (made again only when the picture changes).
#[derive(Debug)]
pub(crate) struct SkyPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    sampler: wgpu::Sampler,
    group: Option<wgpu::BindGroup>,
    /// The picture (id and version) `group` binds; `None`: the white one.
    bound: Option<Option<(TextureId, u64)>>,
    /// This frame draws the background.
    pub on: bool,
}

impl SkyPass {
    pub fn new(device: &wgpu::Device, target: &GpuTargetConfig) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grid4d.sky.layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(size_of::<SkyUniform>() as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("grid4d.sky.pipeline.layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("grid4d.sky.wgsl"),
            source: wgpu::ShaderSource::Wgsl(shaders::SKY.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("grid4d.sky.pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            // Behind everything: neither tested nor written.
            depth_stencil: target.depth_format.map(|format| wgpu::DepthStencilState {
                format,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: target.msaa_samples.max(1),
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target.color_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grid4d.sky.uniform"),
            size: size_of::<SkyUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Round the picture's seam, clamped at the poles.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("grid4d.sky.sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            layout,
            uniform,
            sampler,
            group: None,
            bound: None,
            on: false,
        }
    }

    /// Writes this frame's uniform and binds `picture` (id, version and
    /// view; `None`: `white`), making the bind group again only when the
    /// picture changed.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        uniform: &SkyUniform,
        picture: Option<(TextureId, u64, &wgpu::TextureView)>,
        white: &wgpu::TextureView,
    ) {
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(uniform));
        let wanted = picture.map(|(id, version, _)| (id, version));
        if self.group.is_some() && self.bound == Some(wanted) {
            return;
        }
        let view = picture.map_or(white, |(_, _, view)| view);
        self.group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grid4d.sky.group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        }));
        self.bound = Some(wanted);
    }

    /// Draws the background, when this frame has one.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let (true, Some(group)) = (self.on, &self.group) {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sky_uniform_is_16_byte_aligned() {
        assert_eq!(size_of::<SkyUniform>() % 16, 0);
    }
}
