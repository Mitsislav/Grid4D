use std::{collections::HashMap, ops::Range};

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt as _;

use grid4d_eval::{CurveHandle, MeshKey, Shading};
use grid4d_scene::{MaterialId, MeshHandle, TextureId};

use crate::{
    DisplayStyle, FrameData, InstanceRaw, LightRaw, Vertex, ViewportTarget,
    components::{ComponentOverlay, EdgeInstance, FaceVertex, PointInstance},
    display::{Fill, bounding_box, curve_lines, edge_lines},
    effects::{ShadowMaps, ShadowUniform, Ssao, SsaoUniform, shadow_view},
    frame::UvKey,
    mesh::{TessellationScratch, tessellate},
    shaders,
    textures::{TexelFormat, TextureLevel, Textures},
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
    /// xy: target size in pixels, z: the fill (0 lit, 1 quick, 2 constant), w: 1
    /// for lines in the object's colour.
    viewport: [f32; 4],
    wire: [f32; 4],
    selected: [f32; 4],
    hover: [f32; 4],
    /// Point size, edge width, selected edge width (pixels), pull towards the eye.
    sizes: [f32; 4],
    /// Selected face, hovered face and context edge opacity.
    alphas: [f32; 4],
    /// x: scene lights to use (0: the default light), y: ambient light with
    /// scene lights, z: 1 to darken the ambient light by the view's occlusion.
    lighting: [f32; 4],
}

/// The grid plane and the parallel flag as the shaders read them from `light_dir.w`.
fn grid_flags(plane: crate::GridPlane, parallel: bool) -> f32 {
    let code = plane.code() + if parallel { 8 } else { 0 };
    f32::from(u8::try_from(code).unwrap_or(0))
}

/// Overlay vertices are pulled towards the eye by this share of their distance.
const OVERLAY_PULL: f32 = 0.002;
/// How much of the default light's soft fill remains when scene lights light the
/// view, so sides no light reaches stay readable.
const SCENE_AMBIENT: f32 = 0.35;
const LIGHT_STRIDE: u64 = size_of::<LightRaw>() as u64;
const INITIAL_LIGHT_CAPACITY: usize = 4;

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

/// Vertices and indices on the GPU: a surface's triangles, or a line list.
#[derive(Debug)]
struct GpuGeometry {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
}

/// A part of a cached mesh, built the first time a frame draws it.
#[derive(Debug, Default)]
enum Part {
    #[default]
    NotBuilt,
    /// Built: nothing to draw (no triangles, or no edges).
    Empty,
    Ready(GpuGeometry),
}

impl Part {
    fn get(&self) -> Option<&GpuGeometry> {
        match self {
            Self::Ready(geometry) => Some(geometry),
            Self::NotBuilt | Self::Empty => None,
        }
    }
}

/// The GPU side of one [`DrawKey`]: its surface and its lines.
#[derive(Debug, Default)]
struct GpuMesh {
    faces: Part,
    lines: Part,
    /// Last frame this mesh was drawn, for eviction.
    last_used: u64,
}

/// What a cached mesh is made of: a key's geometry with a Phong angle, as is or
/// as its bounding box, with its own texture coordinates or a projection's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct DrawKey {
    mesh: MeshKey,
    shading: Shading,
    boxed: bool,
    uvs: UvKey,
    /// Its vertices carry tangents (a material with Normal or Bump pictures).
    tangents: bool,
}

/// One instanced draw: a cached mesh and its instances in the instance buffer.
#[derive(Clone, Debug)]
struct Draw {
    key: DrawKey,
    instances: Range<u32>,
    /// A spline's curve: its lines show in every display mode.
    curve: bool,
    /// Its textured material (bind group 2), if any.
    material: Option<MaterialId>,
    /// 0: a surface; 1, 2, …: a Material tag over the ones below, drawn after
    /// them on their depth.
    level: u8,
}

const INSTANCE_STRIDE: u64 = size_of::<InstanceRaw>() as u64;
const INITIAL_INSTANCE_CAPACITY: usize = 256;
/// GPU meshes not drawn for this many frames are freed.
const MESH_TTL_FRAMES: u64 = 600;
/// Polygon meshes (a new key on every edit) go much sooner: after this many
/// frames (each view's render is one) without being drawn.
const POLYGON_TTL_FRAMES: u64 = 8;
/// How often (in frames) the cache looks for meshes to free.
const EVICTION_INTERVAL: u64 = 120;

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 4] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 12 => Float32x2, 15 => Float32x4];
/// Line vertices: a position.
const LINE_ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x3];
/// `PointInstance` and `FaceVertex`: position, flags.
const POSITION_FLAGS_ATTRIBUTES: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Uint32];
/// `EdgeInstance`: a, flags, b.
const EDGE_ATTRIBUTES: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Uint32, 2 => Float32x3];
const INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 12] = wgpu::vertex_attr_array![
    2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4, // model
    6 => Float32x4, 7 => Float32x4, 8 => Float32x4,                 // normal matrix
    9 => Float32x4,                                                 // color
    10 => Float32x4,                                                // params
    11 => Float32x4,                                                // emission
    13 => Float32x4,                                                // uv (2 × 2)
    14 => Float32x4,                                                // uv offset, flags
];
/// The instances ([`InstanceRaw`]) as vertex buffer 1 of every object pass.
pub(crate) const INSTANCE_LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
    array_stride: INSTANCE_STRIDE,
    step_mode: wgpu::VertexStepMode::Instance,
    attributes: &INSTANCE_ATTRIBUTES,
};

/// Owns every GPU resource of the 3D viewport and records its draw commands.
///
/// Usage per frame: [`prepare`](Self::prepare) (uploads) → [`paint`](Self::paint)
/// (inside the host's render pass). Meshes are generated (primitives) or taken
/// from their [`MeshHandle`] (polygon objects) and uploaded the first frame their
/// [`MeshKey`] appears, and freed when unused for a while.
#[derive(Debug)]
pub struct ViewportRenderer {
    mesh_pipeline: wgpu::RenderPipeline,
    /// Material tags over others: on the surfaces' depth, not writing it.
    layer_pipeline: wgpu::RenderPipeline,
    /// The highest layer level this frame draws.
    top_level: u8,
    /// The surfaces of Hidden Line: depth only.
    depth_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    grid_pipeline: wgpu::RenderPipeline,
    point_pipeline: wgpu::RenderPipeline,
    edge_pipeline: wgpu::RenderPipeline,
    face_pipeline: wgpu::RenderPipeline,
    overlay: OverlayBuffers,
    camera_buffer: wgpu::Buffer,
    /// The scene's lights (a storage buffer that only grows).
    light_buffer: wgpu::Buffer,
    light_capacity: usize,
    camera_layout: wgpu::BindGroupLayout,
    camera_bind_group: wgpu::BindGroup,
    /// The key light's shadow map and its pass.
    shadows: ShadowMaps,
    /// The ambient-occlusion passes.
    ssao: Ssao,
    /// Texture pictures and the bind groups of textured materials.
    textures: Textures,
    /// This frame's shadow faces to draw (0: no shadow) and whether it draws SSAO.
    shadow_faces: u32,
    ssao_on: bool,
    meshes: HashMap<DrawKey, GpuMesh>,
    /// Reused while generating meshes for new keys.
    tessellation: TessellationScratch,
    vertex_scratch: Vec<Vertex>,
    index_scratch: Vec<u32>,
    line_scratch: Vec<[f32; 3]>,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    /// This frame's draws; cleared (capacity kept) every frame.
    draws: Vec<Draw>,
    frame: u64,
    encode_srgb: bool,
    draw_grid: bool,
    /// This frame's display mode: how surfaces are filled, whether lines are drawn.
    fill: Fill,
    draw_lines: bool,
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
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(size_of::<CameraUniform>() as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(LIGHT_STRIDE),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(size_of::<ShadowUniform>() as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });
        let light_buffer = create_light_buffer(device, INITIAL_LIGHT_CAPACITY);
        let shadows = ShadowMaps::new(device);
        let ssao = Ssao::new(device);
        let textures = Textures::new(device);
        let camera_bind_group =
            create_camera_bind_group(device, &camera_layout, &camera_buffer, &light_buffer, &shadows);
        // Group 0: camera, lights and shadow; group 1: the view's ambient
        // occlusion; group 2: the object's material (textures).
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("grid4d.viewport.layout"),
            bind_group_layouts: &[Some(&camera_layout), Some(&ssao.layouts.ao), Some(&textures.layout)],
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

        let mesh_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("grid4d.mesh.wgsl"),
            source: wgpu::ShaderSource::Wgsl(shaders::MESH.into()),
        });
        let object = |pass: &ObjectPass| create_object_pipeline(device, &layout, target, &mesh_shader, pass);
        let surfaces = ObjectPass {
            label: "grid4d.mesh.pipeline",
            vertex: "vs_main",
            fragment: "fs_main",
            topology: wgpu::PrimitiveTopology::TriangleList,
            stride: size_of::<Vertex>() as u64,
            attributes: &VERTEX_ATTRIBUTES,
            writes: wgpu::ColorWrites::ALL,
            depth_write: true,
            depth_compare: wgpu::CompareFunction::Less,
        };

        Self {
            mesh_pipeline: object(&surfaces),
            layer_pipeline: object(&ObjectPass {
                label: "grid4d.layer.pipeline",
                depth_write: false,
                depth_compare: wgpu::CompareFunction::LessEqual,
                ..surfaces
            }),
            top_level: 0,
            depth_pipeline: object(&ObjectPass {
                label: "grid4d.depth.pipeline",
                writes: wgpu::ColorWrites::empty(),
                ..surfaces
            }),
            line_pipeline: object(&ObjectPass {
                label: "grid4d.line.pipeline",
                vertex: "vs_line",
                fragment: "fs_line",
                topology: wgpu::PrimitiveTopology::LineList,
                stride: size_of::<[f32; 3]>() as u64,
                attributes: &LINE_ATTRIBUTES,
                writes: wgpu::ColorWrites::ALL,
                depth_write: false,
                depth_compare: wgpu::CompareFunction::LessEqual,
            }),
            grid_pipeline: create_grid_pipeline(device, &layout, target),
            point_pipeline,
            edge_pipeline,
            face_pipeline,
            overlay: OverlayBuffers::new(device),
            camera_buffer,
            light_buffer,
            light_capacity: INITIAL_LIGHT_CAPACITY,
            camera_layout,
            camera_bind_group,
            shadows,
            ssao,
            textures,
            shadow_faces: 0,
            ssao_on: false,
            meshes: HashMap::new(),
            tessellation: TessellationScratch::default(),
            vertex_scratch: Vec::new(),
            index_scratch: Vec::new(),
            line_scratch: Vec::new(),
            instance_buffer: create_instance_buffer(device, INITIAL_INSTANCE_CAPACITY),
            instance_capacity: INITIAL_INSTANCE_CAPACITY,
            draws: Vec::with_capacity(16),
            frame: 0,
            encode_srgb: !target.color_format.is_srgb(),
            draw_grid: true,
            fill: Fill::Lit,
            draw_lines: false,
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
        self.textures.begin_frame(queue);
        let display = frame.display;
        self.fill = display.shading.fill();
        self.draw_lines = display.shading.lines();
        self.ssao_on = frame.effects.ssao && matches!(self.fill, Fill::Lit | Fill::Quick);
        let style = &frame.component_style;
        let px = |n: u32| f32::from(u16::try_from(n).unwrap_or(u16::MAX));
        let uniform = CameraUniform {
            view_proj: frame.view_proj.to_cols_array_2d(),
            eye: frame.eye.extend(if self.encode_srgb { 1.0 } else { 0.0 }).to_array(),
            // w: the grid's plane (0 XZ, 1 XY, 2 ZY) + 8 for a parallel camera.
            light_dir: frame
                .light_dir
                .extend(grid_flags(frame.grid_plane, frame.parallel))
                .to_array(),
            grid: [
                frame.grid_spacing,
                frame.grid_spacing * 10.0,
                frame.grid_extent,
                frame.grid_extent,
            ],
            viewport: [
                px(viewport[0]),
                px(viewport[1]),
                self.fill.code(),
                f32::from(u8::from(display.shading.coloured_lines())),
            ],
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
            // Quick Shading always uses the default light.
            lighting: [
                if self.fill == Fill::Quick {
                    0.0
                } else {
                    to_f32(frame.lights.len())
                },
                SCENE_AMBIENT,
                f32::from(u8::from(self.ssao_on)),
                0.0,
            ],
        };
        queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniform));

        let mut rebind = false;
        if frame.lights.len() > self.light_capacity {
            self.light_capacity = frame.lights.len().next_power_of_two();
            self.light_buffer = create_light_buffer(device, self.light_capacity);
            rebind = true;
        }
        if !frame.lights.is_empty() {
            queue.write_buffer(&self.light_buffer, 0, bytemuck::cast_slice(&frame.lights));
        }
        // The key light's shadow, in the modes lit by the scene's lights.
        self.shadow_faces = 0;
        let mut shadow = ShadowUniform::none();
        if frame.effects.shadows
            && self.fill == Fill::Lit
            && let (Some((index, hard)), Some((min, max))) = (frame.shadow_light, frame.bounds)
            && let Some(light) = frame.lights.get(index)
        {
            let view = shadow_view(light, min, max);
            rebind |= self.shadows.ensure(device, view.count);
            self.shadows.write_faces(queue, &view);
            shadow = ShadowUniform::new(&view, u16::try_from(index).unwrap_or(u16::MAX), hard);
            self.shadow_faces = view.count;
        }
        queue.write_buffer(&self.shadows.uniform, 0, bytemuck::bytes_of(&shadow));
        if rebind {
            self.camera_bind_group = create_camera_bind_group(
                device,
                &self.camera_layout,
                &self.camera_buffer,
                &self.light_buffer,
                &self.shadows,
            );
        }
        if self.ssao_on {
            self.ssao.write(
                queue,
                &SsaoUniform::new(frame.view, frame.proj, viewport, frame.parallel),
            );
        }

        let total = frame.instance_count();
        if total > self.instance_capacity {
            self.instance_capacity = total.next_power_of_two();
            self.instance_buffer = create_instance_buffer(device, self.instance_capacity);
        }

        self.draws.clear();
        self.top_level = 0;
        let mut offset = 0usize;
        let boxed = display.style == DisplayStyle::Box;
        let (faces, lines) = (self.fill != Fill::None, self.draw_lines);
        for (mesh_key, shading, geometry, instances) in frame.draw_batches() {
            let curve = matches!(mesh_key, MeshKey::Curve(_));
            let key = DrawKey {
                mesh: mesh_key,
                shading,
                // A curve is a curve in every style.
                boxed: boxed && !curve,
                uvs: geometry.uvs,
                tangents: geometry.material.is_some_and(crate::frame::MaterialData::bends_normals),
            };
            let ready = if curve {
                self.ensure_curve(device, key, geometry.curve)
            } else {
                self.ensure_mesh(device, key, geometry.mesh, faces, lines)
            };
            if !ready {
                continue; // nothing to draw for this shape
            }
            if let Some(material) = geometry.material {
                self.textures.prepare(device, queue, material, self.frame);
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
                curve,
                material: geometry.material.map(|m| m.id),
                level: geometry.level,
            });
            self.top_level = self.top_level.max(geometry.level);
            offset = end;
        }
        // Polygon meshes get a new key on every edit (each frame of a component
        // drag): free them soon after they are no longer drawn (views with
        // different display styles draw different meshes of the same key).
        // Primitives linger a while.
        let frame_now = self.frame;
        let periodic = self.frame.is_multiple_of(EVICTION_INTERVAL);
        self.meshes.retain(|key, mesh| match key.mesh {
            MeshKey::Polygon(_) | MeshKey::Curve(_) => mesh.last_used + POLYGON_TTL_FRAMES > frame_now,
            // A projection's coordinates follow the object's size.
            _ if key.uvs != UvKey::Own => mesh.last_used + POLYGON_TTL_FRAMES > frame_now,
            _ => !periodic || mesh.last_used + MESH_TTL_FRAMES > frame_now,
        });
        if periodic {
            self.textures.evict(frame_now);
        }
        self.draw_grid = frame.show_grid;
        self.overlay.sync(device, queue, &frame.components);
    }

    /// Makes sure a curve's lines are on the GPU (curves have no surface).
    /// `false` if there is nothing to draw.
    fn ensure_curve(&mut self, device: &wgpu::Device, key: DrawKey, curve: Option<&CurveHandle>) -> bool {
        let mesh = self.meshes.entry(key).or_default();
        mesh.last_used = self.frame;
        if matches!(mesh.lines, Part::NotBuilt) {
            mesh.lines = match curve {
                Some(curve) => {
                    curve_lines(curve.polyline(), &mut self.line_scratch, &mut self.index_scratch);
                    upload(device, "grid4d.curve", &self.line_scratch, &self.index_scratch)
                }
                None => Part::Empty,
            };
            mesh.faces = Part::Empty;
        }
        mesh.lines.get().is_some()
    }

    /// Makes sure the parts of `key`'s mesh this frame draws — its surface
    /// (`faces`), its lines (`lines`) — are on the GPU; `polygon` holds the
    /// geometry of a [`MeshKey::Polygon`]. `false` if there is nothing to draw.
    fn ensure_mesh(
        &mut self,
        device: &wgpu::Device,
        key: DrawKey,
        polygon: Option<&MeshHandle>,
        faces: bool,
        lines: bool,
    ) -> bool {
        let mesh = self.meshes.entry(key).or_default();
        mesh.last_used = self.frame;
        let build_faces = faces && matches!(mesh.faces, Part::NotBuilt);
        let build_lines = lines && matches!(mesh.lines, Part::NotBuilt);
        if build_faces || build_lines {
            let generated;
            let source = match (polygon, key.mesh.build()) {
                (Some(handle), _) => handle.mesh(),
                (None, Some(mesh)) => {
                    generated = mesh;
                    &generated
                }
                (None, None) => return false, // a polygon key without its mesh
            };
            let cuboid = if key.boxed { bounding_box(source) } else { None };
            let geometry = if key.boxed { cuboid.as_ref() } else { Some(source) };
            if build_faces {
                mesh.faces = match geometry {
                    Some(geometry) => {
                        // A projection's coordinates, one per corner.
                        let projected = key
                            .uvs
                            .projection()
                            .filter(|_| !key.boxed)
                            .map(|(projection, matrix)| grid4d_geom::uv::project(projection, geometry, matrix));
                        tessellate(
                            geometry,
                            key.shading.smoothing(),
                            projected.as_deref(),
                            key.tangents,
                            &mut self.tessellation,
                            &mut self.vertex_scratch,
                            &mut self.index_scratch,
                        );
                        upload(device, "grid4d.mesh", &self.vertex_scratch, &self.index_scratch)
                    }
                    None => Part::Empty,
                };
            }
            if build_lines {
                mesh.lines = match geometry {
                    Some(geometry) => {
                        edge_lines(geometry, &mut self.line_scratch, &mut self.index_scratch);
                        upload(device, "grid4d.lines", &self.line_scratch, &self.index_scratch)
                    }
                    None => Part::Empty,
                };
            }
        }
        (faces && mesh.faces.get().is_some()) || (lines && mesh.lines.get().is_some())
    }

    /// Draws `frame` into `target` and submits the work: [`prepare`](Self::prepare),
    /// the effects' passes (the key light's shadow map, the ambient occlusion),
    /// then the view's own pass ([`paint`](Self::paint)). The renderer must have
    /// been created for `target.config()`.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &FrameData,
        target: &mut ViewportTarget,
    ) {
        self.prepare(device, queue, frame, target.size());
        if self.ssao_on {
            target.prepare_ssao(device, &self.ssao.layouts);
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("grid4d.viewport.encoder"),
        });
        for face in 0..self.shadow_faces {
            let mut pass = self.shadows.begin_face(&mut encoder, face);
            pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
            self.draw_parts_where(&mut pass, |d| d.level == 0, false, |mesh| mesh.faces.get());
        }
        let ao = match target.ssao_textures() {
            Some(textures) if self.ssao_on => {
                self.ssao.record(&mut encoder, textures, |pass| {
                    pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
                    self.draw_parts_where(pass, |d| d.level == 0, false, |mesh| mesh.faces.get());
                });
                &textures.ao
            }
            _ => &self.ssao.unused,
        };
        {
            let (color, depth) = target.attachments();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("grid4d.viewport.pass"),
                color_attachments: &[Some(color)],
                depth_stencil_attachment: Some(depth),
                ..Default::default()
            });
            self.paint(&mut pass, ao);
        }
        queue.submit([encoder.finish()]);
    }

    /// Records the view's draw calls into an open render pass; `ao` is the
    /// view's ambient occlusion (bind group 1).
    fn paint(&self, pass: &mut wgpu::RenderPass<'_>, ao: &wgpu::BindGroup) {
        pass.set_bind_group(0, &self.camera_bind_group, &[]);
        pass.set_bind_group(1, ao, &[]);
        pass.set_bind_group(2, self.textures.group(None), &[]);

        let surfaces = match self.fill {
            Fill::Lit | Fill::Quick | Fill::Constant => Some(&self.mesh_pipeline),
            Fill::DepthOnly => Some(&self.depth_pipeline),
            Fill::None => None,
        };
        if let Some(pipeline) = surfaces {
            pass.set_pipeline(pipeline);
            pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
            self.draw_parts_where(pass, |d| d.level == 0, true, |mesh| mesh.faces.get());
            // Material tags over others, lowest first, where their cut-outs
            // leave them.
            if matches!(self.fill, Fill::Lit | Fill::Quick | Fill::Constant) {
                pass.set_pipeline(&self.layer_pipeline);
                for level in 1..=self.top_level {
                    self.draw_parts_where(pass, |d| d.level == level, true, |mesh| mesh.faces.get());
                }
            }
            pass.set_bind_group(2, self.textures.group(None), &[]);
        }

        // The grid after the meshes: depth-tested against them, alpha-blended.
        if self.draw_grid {
            pass.set_pipeline(&self.grid_pipeline);
            pass.draw(0..6, 0..1);
        }

        // Lines over the surfaces (pulled slightly towards the viewer in the
        // shader): every object's in a lines mode, else the splines' alone.
        if self.draws.iter().any(|d| self.draw_lines || d.curve) {
            pass.set_pipeline(&self.line_pipeline);
            pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
            let all = self.draw_lines;
            self.draw_parts_where(
                pass,
                |draw| draw.level == 0 && (all || draw.curve),
                false,
                |mesh| mesh.lines.get(),
            );
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

impl ViewportRenderer {
    /// One instanced draw per batch that `which` picks, of the part of its
    /// mesh that `part` picks; with `materials` each batch's material is
    /// bound first (group 2).
    fn draw_parts_where(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        which: impl Fn(&Draw) -> bool,
        materials: bool,
        part: impl Fn(&GpuMesh) -> Option<&GpuGeometry>,
    ) {
        let mut bound = None;
        for draw in self.draws.iter().filter(|d| which(d)) {
            let Some(geometry) = self.meshes.get(&draw.key).and_then(&part) else {
                continue;
            };
            if materials && bound != Some(draw.material) {
                pass.set_bind_group(2, self.textures.group(draw.material), &[]);
                bound = Some(draw.material);
            }
            pass.set_vertex_buffer(0, geometry.vertices.slice(..));
            pass.set_index_buffer(geometry.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..geometry.count, 0, draw.instances.clone());
        }
    }
}

impl ViewportRenderer {
    /// Whether texture `id`'s picture is on the GPU at `version`.
    #[must_use]
    pub fn has_texture(&self, id: TextureId, version: u64) -> bool {
        self.textures.has(id, version)
    }

    /// Puts texture `id`'s picture (its mip chain, largest first) on the GPU;
    /// materials using it show it from the next frame. Call it when a picture
    /// arrives or changes, never every frame.
    pub fn upload_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: TextureId,
        version: u64,
        format: TexelFormat,
        levels: &[TextureLevel<'_>],
    ) {
        self.textures.upload(device, queue, id, version, format, levels);
    }

    /// Shows texture `id` as missing (a magenta and black checker); its version
    /// becomes [`MISSING_VERSION`](crate::MISSING_VERSION).
    pub fn missing_texture(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, id: TextureId) {
        self.textures.missing(device, queue, id);
    }

    /// Frees the pictures of textures `keep` refuses (no longer in the document).
    pub fn retain_textures(&mut self, keep: impl Fn(TextureId) -> bool) {
        self.textures.retain(keep);
    }
}

/// Uploads vertices and indices (`label`: what they are); [`Part::Empty`] without
/// indices.
fn upload<T: Pod>(device: &wgpu::Device, label: &'static str, vertices: &[T], indices: &[u32]) -> Part {
    if indices.is_empty() {
        return Part::Empty;
    }
    Part::Ready(GpuGeometry {
        vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        }),
        indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(indices),
            usage: wgpu::BufferUsages::INDEX,
        }),
        count: to_u32(indices.len()),
    })
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// A count for the shaders (exact up to 2²⁴).
fn to_f32(n: usize) -> f32 {
    u16::try_from(n).map_or(f32::from(u16::MAX), f32::from)
}

fn create_light_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("grid4d.lights"),
        size: capacity.max(1) as u64 * LIGHT_STRIDE,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn create_camera_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    camera: &wgpu::Buffer,
    lights: &wgpu::Buffer,
    shadows: &ShadowMaps,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("grid4d.camera.bind_group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: camera.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: lights.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: shadows.uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(shadows.array_view()),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(&shadows.sampler),
            },
        ],
    })
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

/// What differs between the pipelines that draw objects (`mesh.wgsl`): their
/// per-vertex data comes with the instances (`InstanceRaw`).
#[derive(Clone, Copy)]
struct ObjectPass {
    label: &'static str,
    vertex: &'static str,
    fragment: &'static str,
    topology: wgpu::PrimitiveTopology,
    stride: u64,
    attributes: &'static [wgpu::VertexAttribute],
    writes: wgpu::ColorWrites,
    depth_write: bool,
    depth_compare: wgpu::CompareFunction,
}

fn create_object_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    target: &GpuTargetConfig,
    shader: &wgpu::ShaderModule,
    pass: &ObjectPass,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(pass.label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(pass.vertex),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[
                Some(wgpu::VertexBufferLayout {
                    array_stride: pass.stride,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: pass.attributes,
                }),
                Some(INSTANCE_LAYOUT),
            ],
        },
        primitive: wgpu::PrimitiveState {
            topology: pass.topology,
            front_face: wgpu::FrontFace::Ccw,
            // Planes are double-sided; closed meshes could use Back culling later.
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: target.depth_format.map(|format| wgpu::DepthStencilState {
            format,
            depth_write_enabled: Some(pass.depth_write),
            depth_compare: Some(pass.depth_compare),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: multisample(target),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(pass.fragment),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target.color_format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: pass.writes,
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
            ("shadow.wgsl", shaders::SHADOW),
            ("ssao.wgsl", shaders::SSAO),
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
