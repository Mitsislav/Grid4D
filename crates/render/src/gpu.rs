use std::{collections::HashMap, ops::Range};

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt as _;

use grid4d_eval::{MeshKey, Shading};
use grid4d_scene::MeshHandle;

use crate::{
    FrameData, InstanceRaw, Vertex, ViewportTarget,
    components::{ComponentOverlay, EdgeInstance, FaceVertex, PointInstance},
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

/// Layout must match `Camera` in the WGSL shaders (the mesh and grid shaders
/// declare only its first fields).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    light_dir: [f32; 4],
    grid: [f32; 4],
    /// xy: target size in pixels.
    viewport: [f32; 4],
    wire: [f32; 4],
    selected: [f32; 4],
    hover: [f32; 4],
    /// Point size, edge width, selected edge width (pixels), pull towards the eye.
    sizes: [f32; 4],
    /// Selected face, hovered face and context edge opacity.
    alphas: [f32; 4],
}

/// Overlay vertices are pulled towards the eye by this share of their distance.
const OVERLAY_PULL: f32 = 0.002;

/// A vertex buffer refilled from a CPU array when it changes; grows to the next
/// power of two, never shrinks.
#[derive(Debug)]
struct GrowingBuffer {
    buffer: wgpu::Buffer,
    capacity: u64,
    len: u32,
    label: &'static str,
}

impl GrowingBuffer {
    const INITIAL_BYTES: u64 = 4096;

    fn new(device: &wgpu::Device, label: &'static str) -> Self {
        Self {
            buffer: create_vertex_buffer(device, label, Self::INITIAL_BYTES),
            capacity: Self::INITIAL_BYTES,
            len: 0,
            label,
        }
    }

    fn upload<T: Pod>(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[T]) {
        let bytes: &[u8] = bytemuck::cast_slice(data);
        let size = bytes.len() as u64;
        if size > self.capacity {
            self.capacity = size.next_power_of_two();
            self.buffer = create_vertex_buffer(device, self.label, self.capacity);
        }
        if !bytes.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytes);
        }
        self.len = to_u32(data.len());
    }
}

/// GPU copies of the component overlay's arrays.
#[derive(Debug)]
struct OverlayBuffers {
    points: GrowingBuffer,
    edges: GrowingBuffer,
    faces: GrowingBuffer,
    hover_points: GrowingBuffer,
    hover_edges: GrowingBuffer,
    hover_faces: GrowingBuffer,
    /// The overlay generations last uploaded.
    generation: Option<u64>,
    hover_generation: Option<u64>,
}

impl OverlayBuffers {
    fn new(device: &wgpu::Device) -> Self {
        Self {
            points: GrowingBuffer::new(device, "grid4d.overlay.points"),
            edges: GrowingBuffer::new(device, "grid4d.overlay.edges"),
            faces: GrowingBuffer::new(device, "grid4d.overlay.faces"),
            hover_points: GrowingBuffer::new(device, "grid4d.overlay.hover_points"),
            hover_edges: GrowingBuffer::new(device, "grid4d.overlay.hover_edges"),
            hover_faces: GrowingBuffer::new(device, "grid4d.overlay.hover_faces"),
            generation: None,
            hover_generation: None,
        }
    }

    /// Uploads whichever arrays changed since the last frame.
    fn sync(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, overlay: &ComponentOverlay) {
        if self.generation != Some(overlay.generation) {
            self.points.upload(device, queue, &overlay.points);
            self.edges.upload(device, queue, &overlay.edges);
            self.faces.upload(device, queue, &overlay.faces);
            self.generation = Some(overlay.generation);
        }
        if self.hover_generation != Some(overlay.hover_generation) {
            self.hover_points.upload(device, queue, &overlay.hover_points);
            self.hover_edges.upload(device, queue, &overlay.hover_edges);
            self.hover_faces.upload(device, queue, &overlay.hover_faces);
            self.hover_generation = Some(overlay.hover_generation);
        }
    }
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
    key: (MeshKey, Shading),
    instances: Range<u32>,
}

const INSTANCE_STRIDE: u64 = size_of::<InstanceRaw>() as u64;
const INITIAL_INSTANCE_CAPACITY: usize = 256;
/// GPU meshes not drawn for this many frames are freed.
const MESH_TTL_FRAMES: u64 = 600;
/// How often (in frames) the cache looks for meshes to free.
const EVICTION_INTERVAL: u64 = 120;

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
/// `PointInstance` and `FaceVertex`: position, flags.
const POSITION_FLAGS_ATTRIBUTES: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Uint32];
/// `EdgeInstance`: a, flags, b.
const EDGE_ATTRIBUTES: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Uint32, 2 => Float32x3];
const INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 9] = wgpu::vertex_attr_array![
    2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4, // model
    6 => Float32x4, 7 => Float32x4, 8 => Float32x4,                 // normal matrix
    9 => Float32x4,                                                 // color
    10 => Float32x4,                                                // params
];

/// Owns every GPU resource of the 3D viewport and records its draw commands.
///
/// Usage per frame: [`prepare`](Self::prepare) (uploads) → [`paint`](Self::paint)
/// (inside the host's render pass). Meshes are generated (primitives) or taken
/// from their [`MeshHandle`] (polygon objects) and uploaded the first frame their
/// [`MeshKey`] appears, and freed when unused for a while.
#[derive(Debug)]
pub struct ViewportRenderer {
    mesh_pipeline: wgpu::RenderPipeline,
    grid_pipeline: wgpu::RenderPipeline,
    point_pipeline: wgpu::RenderPipeline,
    edge_pipeline: wgpu::RenderPipeline,
    face_pipeline: wgpu::RenderPipeline,
    overlay: OverlayBuffers,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    meshes: HashMap<(MeshKey, Shading), GpuMesh>,
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
            label: Some("grid4d.camera"),
            size: size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grid4d.camera.layout"),
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
            label: Some("grid4d.camera.bind_group"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("grid4d.viewport.layout"),
            bind_group_layouts: &[Some(&camera_layout)],
            immediate_size: 0,
        });

        let overlay_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("grid4d.overlay.wgsl"),
            source: wgpu::ShaderSource::Wgsl(shaders::OVERLAY.into()),
        });
        let overlay = |entry: &'static str, step_mode, stride, attributes: &'static [wgpu::VertexAttribute]| {
            create_overlay_pipeline(
                device,
                &layout,
                target,
                &overlay_shader,
                entry,
                wgpu::VertexBufferLayout {
                    array_stride: stride,
                    step_mode,
                    attributes,
                },
            )
        };
        let point_pipeline = overlay(
            "vs_point",
            wgpu::VertexStepMode::Instance,
            size_of::<PointInstance>() as u64,
            &POSITION_FLAGS_ATTRIBUTES,
        );
        let edge_pipeline = overlay(
            "vs_edge",
            wgpu::VertexStepMode::Instance,
            size_of::<EdgeInstance>() as u64,
            &EDGE_ATTRIBUTES,
        );
        let face_pipeline = overlay(
            "vs_face",
            wgpu::VertexStepMode::Vertex,
            size_of::<FaceVertex>() as u64,
            &POSITION_FLAGS_ATTRIBUTES,
        );

        Self {
            mesh_pipeline: create_mesh_pipeline(device, &layout, target),
            grid_pipeline: create_grid_pipeline(device, &layout, target),
            point_pipeline,
            edge_pipeline,
            face_pipeline,
            overlay: OverlayBuffers::new(device),
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

    /// Uploads the frame's camera, instances and component overlay, generating
    /// meshes for shapes seen for the first time. Called before the render pass;
    /// `viewport` is the target's size in pixels.
    ///
    /// Allocation-free in steady state: meshes are only created for new keys, and
    /// buffers are only recreated (with doubled capacity) when the scene outgrows
    /// them; the overlay is uploaded only when it changed.
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &FrameData, viewport: [u32; 2]) {
        self.frame += 1;
        let style = &frame.component_style;
        let px = |n: u32| f32::from(u16::try_from(n).unwrap_or(u16::MAX));
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
            viewport: [px(viewport[0]), px(viewport[1]), 0.0, 0.0],
            wire: style.wire,
            selected: style.selected,
            hover: style.hover,
            sizes: [
                style.point_size,
                style.edge_width,
                style.selected_edge_width,
                OVERLAY_PULL,
            ],
            alphas: [style.face_alpha, style.hover_face_alpha, style.context_alpha, 0.0],
        };
        queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniform));

        let total = frame.instance_count();
        if total > self.instance_capacity {
            self.instance_capacity = total.next_power_of_two();
            self.instance_buffer = create_instance_buffer(device, self.instance_capacity);
        }

        self.draws.clear();
        let mut offset = 0usize;
        for (key, shading, mesh, instances) in frame.draw_batches() {
            if !self.ensure_mesh(device, (key, shading), mesh) {
                continue; // nothing to draw for this shape
            }
            queue.write_buffer(
                &self.instance_buffer,
                offset as u64 * INSTANCE_STRIDE,
                bytemuck::cast_slice(instances),
            );
            let end = offset + instances.len();
            self.draws.push(Draw {
                key: (key, shading),
                instances: to_u32(offset)..to_u32(end),
            });
            offset = end;
        }
        // Polygon meshes get a new key on every edit (each frame of a component
        // drag): free them as soon as they are not drawn. Primitives linger a while.
        let frame_now = self.frame;
        let periodic = self.frame.is_multiple_of(EVICTION_INTERVAL);
        self.meshes.retain(|(key, _), mesh| match key {
            MeshKey::Polygon(_) => mesh.last_used == frame_now,
            _ => !periodic || mesh.last_used + MESH_TTL_FRAMES > frame_now,
        });
        self.draw_grid = frame.show_grid;
        self.overlay.sync(device, queue, &frame.components);
    }

    /// Makes sure the mesh for `key` is on the GPU (`polygon` holds the geometry of
    /// a [`MeshKey::Polygon`]); `false` if it has no triangles.
    fn ensure_mesh(&mut self, device: &wgpu::Device, key: (MeshKey, Shading), polygon: Option<&MeshHandle>) -> bool {
        if let Some(mesh) = self.meshes.get_mut(&key) {
            mesh.last_used = self.frame;
            return true;
        }
        let (mesh_key, shading) = key;
        let generated;
        let source = match (polygon, mesh_key.build()) {
            (Some(handle), _) => handle.mesh(),
            (None, Some(mesh)) => {
                generated = mesh;
                &generated
            }
            (None, None) => return false, // a polygon key without its mesh
        };
        tessellate(
            source,
            shading.smoothing(),
            &mut self.tessellation,
            &mut self.vertex_scratch,
            &mut self.index_scratch,
        );
        if self.index_scratch.is_empty() {
            return false;
        }
        let mesh = GpuMesh {
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("grid4d.mesh.vertices"),
                contents: bytemuck::cast_slice(&self.vertex_scratch),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("grid4d.mesh.indices"),
                contents: bytemuck::cast_slice(&self.index_scratch),
                usage: wgpu::BufferUsages::INDEX,
            }),
            index_count: to_u32(self.index_scratch.len()),
            last_used: self.frame,
        };
        self.meshes.insert(key, mesh);
        true
    }

    /// Draws `frame` into `target` and submits the work: [`prepare`](Self::prepare),
    /// then one render pass with [`paint`](Self::paint). The renderer must have been
    /// created for `target.config()`.
    pub fn render(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &FrameData, target: &ViewportTarget) {
        self.prepare(device, queue, frame, target.size());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("grid4d.viewport.encoder"),
        });
        {
            let (color, depth) = target.attachments();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("grid4d.viewport.pass"),
                color_attachments: &[Some(color)],
                depth_stencil_attachment: Some(depth),
                ..Default::default()
            });
            self.paint(&mut pass);
        }
        queue.submit([encoder.finish()]);
    }

    /// Records the viewport draw calls into an open render pass.
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

        // The grid after the meshes: depth-tested against them, alpha-blended.
        if self.draw_grid {
            pass.set_pipeline(&self.grid_pipeline);
            pass.draw(0..6, 0..1);
        }

        // Component overlay on top: tinted polygons, edges, points, then the
        // component under the cursor over everything else.
        let o = &self.overlay;
        for (faces, edges, points) in [
            (&o.faces, &o.edges, &o.points),
            (&o.hover_faces, &o.hover_edges, &o.hover_points),
        ] {
            if faces.len > 0 {
                pass.set_pipeline(&self.face_pipeline);
                pass.set_vertex_buffer(0, faces.buffer.slice(..));
                pass.draw(0..faces.len, 0..1);
            }
            if edges.len > 0 {
                pass.set_pipeline(&self.edge_pipeline);
                pass.set_vertex_buffer(0, edges.buffer.slice(..));
                pass.draw(0..6, 0..edges.len);
            }
            if points.len > 0 {
                pass.set_pipeline(&self.point_pipeline);
                pass.set_vertex_buffer(0, points.buffer.slice(..));
                pass.draw(0..6, 0..points.len);
            }
        }
    }
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn create_vertex_buffer(device: &wgpu::Device, label: &'static str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn create_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("grid4d.instances"),
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
        label: Some("grid4d.mesh.wgsl"),
        source: wgpu::ShaderSource::Wgsl(shaders::MESH.into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("grid4d.mesh.pipeline"),
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
        label: Some("grid4d.grid.wgsl"),
        source: wgpu::ShaderSource::Wgsl(shaders::GRID.into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("grid4d.grid.pipeline"),
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

/// An overlay pipeline (`entry` in `overlay.wgsl`): depth-tested against the
/// scene without writing depth, premultiplied-alpha blended.
fn create_overlay_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    target: &GpuTargetConfig,
    shader: &wgpu::ShaderModule,
    entry: &'static str,
    buffer: wgpu::VertexBufferLayout<'static>,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(buffer)],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: target.depth_format.map(|format| wgpu::DepthStencilState {
            format,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: multisample(target),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_overlay"),
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
        for (name, source) in [
            ("mesh.wgsl", shaders::MESH),
            ("grid.wgsl", shaders::GRID),
            ("overlay.wgsl", shaders::OVERLAY),
        ] {
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
