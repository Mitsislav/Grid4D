use std::{collections::HashMap, ops::Range};

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt as _;

use oc4d_eval::MeshKey;

use crate::{
    FrameData, InstanceRaw, Vertex,
    mesh::{TessellationScratch, tessellate},
    shaders,
};

/// Describes the render target the viewport draws into. Must match the host's
/// render pass (egui's surface), or wgpu validation will reject the pipelines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuTargetConfig {
    pub color_format: wgpu::TextureFormat,
    pub depth_format: Option<wgpu::TextureFormat>,
    pub msaa_samples: u32,
}

/// Layout must match `Camera` in the WGSL shaders.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    light_dir: [f32; 4],
    grid: [f32; 4],
}

#[derive(Debug)]
struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    /// Last frame this mesh was drawn, for eviction.
    last_used: u64,
}

/// One instanced draw: a cached mesh and its instances in the instance buffer.
#[derive(Clone, Debug)]
struct Draw {
    key: MeshKey,
    instances: Range<u32>,
}

const INSTANCE_STRIDE: u64 = size_of::<InstanceRaw>() as u64;
const INITIAL_INSTANCE_CAPACITY: usize = 256;
/// GPU meshes not drawn for this many frames are freed.
const MESH_TTL_FRAMES: u64 = 600;
/// How often (in frames) the cache looks for meshes to free.
const EVICTION_INTERVAL: u64 = 120;

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
const INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 9] = wgpu::vertex_attr_array![
    2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4, // model
    6 => Float32x4, 7 => Float32x4, 8 => Float32x4,                 // normal matrix
    9 => Float32x4,                                                 // color
    10 => Float32x4,                                                // params
];

/// Owns every GPU resource of the 3D viewport and records its draw commands.
///
/// Usage per frame: [`prepare`](Self::prepare) (uploads) → [`paint`](Self::paint)
/// (inside the host's render pass). Meshes are generated and uploaded the first
/// frame their [`MeshKey`] appears and freed when unused for a while.
#[derive(Debug)]
pub struct ViewportRenderer {
    mesh_pipeline: wgpu::RenderPipeline,
    grid_pipeline: wgpu::RenderPipeline,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    meshes: HashMap<MeshKey, GpuMesh>,
    /// Reused while generating meshes for new keys.
    tessellation: TessellationScratch,
    vertex_scratch: Vec<Vertex>,
    index_scratch: Vec<u32>,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    /// This frame's draws; cleared (capacity kept) every frame.
    draws: Vec<Draw>,
    frame: u64,
    encode_srgb: bool,
    draw_grid: bool,
}

impl ViewportRenderer {
    #[must_use]
    pub fn new(device: &wgpu::Device, target: &GpuTargetConfig) -> Self {
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("oc4d.camera"),
            size: size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("oc4d.camera.layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(size_of::<CameraUniform>() as u64),
                },
                count: None,
            }],
        });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("oc4d.camera.bind_group"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("oc4d.viewport.layout"),
            bind_group_layouts: &[Some(&camera_layout)],
            immediate_size: 0,
        });

        Self {
            mesh_pipeline: create_mesh_pipeline(device, &layout, target),
            grid_pipeline: create_grid_pipeline(device, &layout, target),
            camera_buffer,
            camera_bind_group,
            meshes: HashMap::new(),
            tessellation: TessellationScratch::default(),
            vertex_scratch: Vec::new(),
            index_scratch: Vec::new(),
            instance_buffer: create_instance_buffer(device, INITIAL_INSTANCE_CAPACITY),
            instance_capacity: INITIAL_INSTANCE_CAPACITY,
            draws: Vec::with_capacity(16),
            frame: 0,
            encode_srgb: !target.color_format.is_srgb(),
            draw_grid: true,
        }
    }

    /// Uploads the frame's camera and instances, generating meshes for shapes
    /// seen for the first time. Called before the render pass.
    ///
    /// Allocation-free in steady state: meshes are only created for new keys, and
    /// the instance buffer is only recreated (with doubled capacity) when the
    /// scene outgrows it.
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &FrameData) {
        self.frame += 1;
        let uniform = CameraUniform {
            view_proj: frame.view_proj.to_cols_array_2d(),
            eye: frame.eye.extend(if self.encode_srgb { 1.0 } else { 0.0 }).to_array(),
            light_dir: frame.light_dir.extend(0.0).to_array(),
            grid: [
                frame.grid_spacing,
                frame.grid_spacing * 10.0,
                frame.grid_extent,
                frame.grid_extent,
            ],
        };
        queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniform));

        let total = frame.instance_count();
        if total > self.instance_capacity {
            self.instance_capacity = total.next_power_of_two();
            self.instance_buffer = create_instance_buffer(device, self.instance_capacity);
        }

        self.draws.clear();
        let mut offset = 0usize;
        for (key, instances) in frame.batches() {
            if !self.ensure_mesh(device, key) {
                continue; // nothing to draw for this shape
            }
            queue.write_buffer(
                &self.instance_buffer,
                offset as u64 * INSTANCE_STRIDE,
                bytemuck::cast_slice(instances),
            );
            let end = offset + instances.len();
            self.draws.push(Draw {
                key,
                instances: to_u32(offset)..to_u32(end),
            });
            offset = end;
        }
        if self.frame.is_multiple_of(EVICTION_INTERVAL) {
            let frame_now = self.frame;
            self.meshes
                .retain(|_, mesh| mesh.last_used + MESH_TTL_FRAMES > frame_now);
        }
        self.draw_grid = frame.show_grid;
    }

    /// Makes sure the mesh for `key` is on the GPU; `false` if it has no triangles.
    fn ensure_mesh(&mut self, device: &wgpu::Device, key: MeshKey) -> bool {
        if let Some(mesh) = self.meshes.get_mut(&key) {
            mesh.last_used = self.frame;
            return true;
        }
        tessellate(
            key,
            &mut self.tessellation,
            &mut self.vertex_scratch,
            &mut self.index_scratch,
        );
        if self.index_scratch.is_empty() {
            return false;
        }
        let mesh = GpuMesh {
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("oc4d.mesh.vertices"),
                contents: bytemuck::cast_slice(&self.vertex_scratch),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("oc4d.mesh.indices"),
                contents: bytemuck::cast_slice(&self.index_scratch),
                usage: wgpu::BufferUsages::INDEX,
            }),
            index_count: to_u32(self.index_scratch.len()),
            last_used: self.frame,
        };
        self.meshes.insert(key, mesh);
        true
    }

    /// Records the viewport draw calls into the host's render pass.
    pub fn paint(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_bind_group(0, &self.camera_bind_group, &[]);

        pass.set_pipeline(&self.mesh_pipeline);
        pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        for draw in &self.draws {
            let Some(mesh) = self.meshes.get(&draw.key) else {
                continue;
            };
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.index_count, 0, draw.instances.clone());
        }

        // The grid is drawn last: depth-tested against the meshes, alpha-blended.
        if self.draw_grid {
            pass.set_pipeline(&self.grid_pipeline);
            pass.draw(0..6, 0..1);
        }
    }
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn create_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("oc4d.instances"),
        size: capacity as u64 * INSTANCE_STRIDE,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn multisample(target: &GpuTargetConfig) -> wgpu::MultisampleState {
    wgpu::MultisampleState {
        count: target.msaa_samples.max(1),
        mask: !0,
        alpha_to_coverage_enabled: false,
    }
}

fn create_mesh_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    target: &GpuTargetConfig,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("oc4d.mesh.wgsl"),
        source: wgpu::ShaderSource::Wgsl(shaders::MESH.into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("oc4d.mesh.pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[
                Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &VERTEX_ATTRIBUTES,
                }),
                Some(wgpu::VertexBufferLayout {
                    array_stride: INSTANCE_STRIDE,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &INSTANCE_ATTRIBUTES,
                }),
            ],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            // Planes are double-sided; closed meshes could use Back culling later.
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: target.depth_format.map(|format| wgpu::DepthStencilState {
            format,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: multisample(target),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target.color_format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn create_grid_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    target: &GpuTargetConfig,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("oc4d.grid.wgsl"),
        source: wgpu::ShaderSource::Wgsl(shaders::GRID.into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("oc4d.grid.pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: target.depth_format.map(|format| wgpu::DepthStencilState {
            format,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: multisample(target),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target.color_format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parses and validates both WGSL shaders offline (no GPU needed), so shader
    /// errors fail `cargo test` instead of crashing the app at start-up.
    #[test]
    fn shaders_validate() {
        for (name, source) in [("mesh.wgsl", shaders::MESH), ("grid.wgsl", shaders::GRID)] {
            let module =
                naga::front::wgsl::parse_str(source).unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(source)));
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
                .validate(&module)
                .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        }
    }

    #[test]
    fn uniform_is_16_byte_aligned() {
        assert_eq!(size_of::<CameraUniform>() % 16, 0);
    }
}
