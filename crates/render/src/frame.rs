use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use glam::{Affine2, Mat3, Mat4, Vec3};
use grid4d_eval::{Ambient, CurveHandle, EvaluatedLight, EvaluatedScene, MeshKey, Shading, SkyModel, Sun, Surround};
use grid4d_geom::Component;
use grid4d_scene::{
    AreaShape, Channel, Falloff, LightType, Material, MaterialId, Materials, MeshHandle, NodeId, ObjectKindTag,
    Projection, TextureId, TextureSlot,
};

use crate::{
    ComponentHit, ComponentStyle, DisplayMode, FilterItem, OrbitCamera, ViewEffects, ViewFilter,
    components::ComponentOverlay, sky::SurroundFrame,
};

/// Per-instance GPU data. Layout must match `InstanceIn` in `shaders/mesh.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct InstanceRaw {
    pub model: [[f32; 4]; 4],
    /// Inverse-transpose of the model's upper 3×3 (columns as vec4); the
    /// three `w`s hold the object's `unlit` mask (the linked lights that do
    /// not light it), 24 bits each, as whole numbers.
    pub normal: [[f32; 4]; 3],
    /// Linear RGB + alpha.
    pub color: [f32; 4],
    /// `x`: selected (0/1), `y`: unlit (0/1), `z`: roughness, `w`: metallic.
    pub params: [f32; 4],
    /// Linear RGB the surface gives off (its material's luminance); `w`: 1 for
    /// markers (cameras, lights), which cast no shadow and occlude nothing.
    pub emission: [f32; 4],
    /// The Material tag's change to texture coordinates: the 2 × 2 part
    /// (columns), then `uv_offset.xy` its shift.
    pub uv: [f32; 4],
    /// `xy`: shift of the texture coordinates, `z`: 1 for a Camera projection
    /// (coordinates from the view's picture), `w` unused.
    pub uv_offset: [f32; 4],
}

/// Where a batch's texture coordinates come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum UvKey {
    /// The mesh's own UVs (a Camera projection works in the shader).
    Own,
    /// A Material tag's projection, `matrix` (as bits) taking the drawn mesh's
    /// points into the projection's shape.
    Projected { projection: Projection, matrix: [u32; 16] },
}

impl UvKey {
    fn projected(projection: Projection, matrix: Mat4) -> Self {
        Self::Projected {
            projection,
            matrix: matrix.to_cols_array().map(f32::to_bits),
        }
    }

    /// The projection and its matrix, for a projected key.
    pub(crate) fn projection(self) -> Option<(Projection, Mat4)> {
        match self {
            Self::Own => None,
            Self::Projected { projection, matrix } => {
                Some((projection, Mat4::from_cols_array(&matrix.map(f32::from_bits))))
            }
        }
    }
}

/// What the viewport needs of a material with textures (or a cut-out), per
/// batch: its texture slots and the values the instances do not carry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MaterialData {
    pub id: MaterialId,
    /// In [`Channel::ALL`] order.
    pub maps: [Option<TextureSlot>; 10],
    pub emission_strength: f32,
    pub opacity: f32,
    pub opacity_threshold: f32,
    pub specular: f32,
    pub normal_strength: f32,
    pub bump_height: f32,
}

impl MaterialData {
    /// Whether its pictures bend the shading (Normal, Bump): its meshes need
    /// tangents.
    pub(crate) fn bends_normals(&self) -> bool {
        self.maps[4].is_some() || self.maps[5].is_some()
    }

    /// `None` for a material the shader draws from the instance alone (no
    /// texture, nothing cut out).
    fn of(id: MaterialId, material: &Material) -> Option<Self> {
        let needed = material.maps().next().is_some() || material.opacity < material.opacity_threshold;
        needed.then(|| Self {
            id,
            maps: Channel::ALL.map(|channel| material.map(channel).copied()),
            emission_strength: material.emission_strength,
            opacity: material.opacity,
            opacity_threshold: material.opacity_threshold,
            specular: material.specular,
            normal_strength: material.normal_strength,
            bump_height: material.bump_height,
        })
    }
}

/// How the instance's texture coordinates are changed (its Material tag's
/// turn, tiling and offset; a Camera projection's flag).
fn uv_fields(transform: Affine2, camera: bool) -> ([f32; 4], [f32; 4]) {
    let m = transform.matrix2;
    (
        [m.x_axis.x, m.x_axis.y, m.y_axis.x, m.y_axis.y],
        [
            transform.translation.x,
            transform.translation.y,
            f32::from(u8::from(camera)),
            0.0,
        ],
    )
}

/// One light for the shaders. Layout must match `Light` in `shaders/mesh.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct LightRaw {
    /// `xyz`: world position, `w`: type (0 omni, 1 spot, 2 infinite, 3 area).
    pub position: [f32; 4],
    /// `xyz`: the direction it shines in, `w`: cosine of half the outer angle.
    pub direction: [f32; 4],
    /// `rgb`: colour × intensity, `w`: cosine of half the inner angle (the outer
    /// one's for a hard cone).
    pub color: [f32; 4],
    /// `x`: falloff (0 none, 1 linear, 2 inverse square), `y`: radius. An
    /// area light: `y` the distance at which it matches an Omni light with
    /// Inverse Square falloff, `z` the most it gives (right at its face),
    /// `w` 1 when two-sided.
    pub falloff: [f32; 4],
    /// An area light: `xyz` half of the shape along its first axis (a
    /// Tube's along its length), `w` the shape (0 rectangle, 1 disc, 2
    /// sphere, 3 tube).
    pub area_x: [f32; 4],
    /// An area light: `xyz` half of the shape along its second axis, `w` the
    /// radius of a Sphere or Tube.
    pub area_y: [f32; 4],
    /// `x`: its bit in the objects' `unlit` masks, −1 when it lights
    /// everything.
    pub link: [f32; 4],
}

/// The distance at which an area light of intensity 1 lights a facing
/// surface like an Omni light of intensity 1 with Inverse Square falloff at
/// its default Radius/Decay (the path tracer's `AREA_REFERENCE_DISTANCE`).
const AREA_REFERENCE_DISTANCE: f32 = 500.0;

impl LightRaw {
    #[must_use]
    pub fn new(light: &EvaluatedLight) -> Self {
        let code = |n: u8| f32::from(n);
        let kind = match light.light_type {
            LightType::Omni => 0,
            LightType::Spot => 1,
            LightType::Infinite => 2,
            LightType::Area => 3,
        };
        let details = &light.details;
        let half_cos = |degrees: f32| (degrees.to_radians() * 0.5).cos();
        let cos_outer = half_cos(details.outer_angle_deg);
        let cos_inner = if details.use_inner {
            half_cos(details.inner_angle_deg.min(details.outer_angle_deg))
        } else {
            cos_outer
        };
        let falloff = match details.falloff {
            Falloff::None => 0,
            Falloff::Linear => 1,
            Falloff::InverseSquare => 2,
        };
        let mut raw = Self {
            position: light.position.extend(code(kind)).to_array(),
            direction: light.direction.extend(cos_outer).to_array(),
            color: [light.color[0], light.color[1], light.color[2], cos_inner],
            falloff: [code(falloff), details.radius, 0.0, 0.0],
            area_x: [0.0; 4],
            area_y: [0.0; 4],
            link: [light.link.map_or(-1.0, f32::from), 0.0, 0.0, 0.0],
        };
        if light.light_type == LightType::Area {
            raw.set_area(light);
        }
        raw
    }

    /// A Sky's sun: an Infinite light of its colour.
    pub(crate) fn sun(sun: &Sun) -> Self {
        let colour = sun.irradiance;
        Self {
            position: [0.0, 0.0, 0.0, 2.0],
            direction: (-sun.direction).extend(-1.0).to_array(),
            color: [colour.x, colour.y, colour.z, -1.0],
            falloff: [0.0, 1.0, 0.0, 0.0],
            area_x: [0.0; 4],
            area_y: [0.0; 4],
            link: [-1.0, 0.0, 0.0, 0.0],
        }
    }

    /// An area light's shape, and the most light it gives: what the shader
    /// needs to light from the shape's nearest point (see
    /// `docs/reference/render-lights-area.md`).
    fn set_area(&mut self, light: &EvaluatedLight) {
        let details = &light.details;
        let [size_x, size_y] = details.area_size;
        let (shape, half_x, half_y, radius, projected) = match details.area_shape {
            AreaShape::Rectangle => {
                let (hx, hy) = (light.x_axis * (size_x * 0.5), light.y_axis * (size_y * 0.5));
                (0u8, hx, hy, 0.0, 4.0 * hx.cross(hy).length())
            }
            AreaShape::Disc => {
                let (hx, hy) = (light.x_axis * (size_x * 0.5), light.y_axis * (size_x * 0.5));
                (1, hx, hy, 0.0, std::f32::consts::PI * hx.cross(hy).length())
            }
            AreaShape::Sphere => {
                let r = size_x * 0.5 * light.x_axis.length();
                (2, Vec3::ZERO, Vec3::ZERO, r, std::f32::consts::PI * r * r)
            }
            AreaShape::Tube => {
                let r = size_x * 0.5 * light.x_axis.length();
                let half = -light.direction * (light.z_scale * size_y * 0.5);
                (3, half, Vec3::ZERO, r, 4.0 * r * half.length())
            }
        };
        let reference = AREA_REFERENCE_DISTANCE;
        let most = std::f32::consts::PI * reference * reference / projected.max(1e-6);
        self.falloff = [0.0, reference, most, f32::from(u8::from(details.two_sided))];
        self.area_x = half_x.extend(f32::from(shape)).to_array();
        self.area_y = half_y.extend(radius).to_array();
    }
}

/// Batches unused for this many frames are dropped (their shape is gone).
const BATCH_TTL_FRAMES: u64 = 240;

/// The instances drawn with one mesh this frame.
#[derive(Debug)]
struct Batch {
    key: MeshKey,
    /// Meshes with another Phong angle are other GPU meshes.
    shading: Shading,
    /// Texture coordinates by a projection make other GPU meshes too.
    uvs: UvKey,
    /// The textured material its instances share (refreshed every frame).
    material: Option<MaterialData>,
    /// 0: the surface as usual; 1, 2, …: a Material tag drawn over the ones
    /// below it, in that order.
    level: u8,
    /// The geometry of a [`MeshKey::Polygon`], for the renderer to upload.
    mesh: Option<MeshHandle>,
    /// The curve of a [`MeshKey::Curve`].
    curve: Option<CurveHandle>,
    instances: Vec<InstanceRaw>,
    last_used: u64,
}

/// The stored geometry of a batch: a polygon object's mesh or a spline's curve
/// (primitives' meshes are generated from their key).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Geometry<'a> {
    pub mesh: Option<&'a MeshHandle>,
    pub curve: Option<&'a CurveHandle>,
    pub uvs: UvKey,
    pub material: Option<&'a MaterialData>,
    pub level: u8,
}

/// The plane the grid lies in: the ground in perspective views and looking down
/// or up, the view's own plane in front and side views.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GridPlane {
    #[default]
    XZ,
    XY,
    ZY,
}

impl GridPlane {
    /// The plane a parallel view looking along `direction` (either way) shows its
    /// grid in: the one the direction is (nearly) normal to.
    #[must_use]
    pub fn facing(direction: Vec3) -> Self {
        let d = direction.abs();
        if d.y >= d.x && d.y >= d.z {
            Self::XZ
        } else if d.z >= d.x {
            Self::XY
        } else {
            Self::ZY
        }
    }

    /// The plane's normal, then its two axes (the grid's lines run along them).
    #[must_use]
    pub const fn axes(self) -> (Vec3, Vec3, Vec3) {
        match self {
            Self::XZ => (Vec3::Y, Vec3::X, Vec3::Z),
            Self::XY => (Vec3::Z, Vec3::X, Vec3::Y),
            Self::ZY => (Vec3::X, Vec3::Z, Vec3::Y),
        }
    }

    /// How the shaders number the plane.
    #[must_use]
    pub const fn code(self) -> u32 {
        match self {
            Self::XZ => 0,
            Self::XY => 1,
            Self::ZY => 2,
        }
    }
}

/// Everything the GPU needs to draw one viewport frame. Owned by the UI, filled
/// every frame from an [`EvaluatedScene`], consumed by
/// [`ViewportRenderer::prepare`](crate::ViewportRenderer::prepare).
#[derive(Debug)]
pub struct FrameData {
    pub view_proj: Mat4,
    /// The camera's view and projection (`view_proj` = `proj` × `view`).
    pub view: Mat4,
    pub proj: Mat4,
    pub eye: Vec3,
    /// Direction *towards* the key light, world space.
    pub light_dir: Vec3,
    pub grid_spacing: f32,
    /// Half-size of the grid quad; the grid fades out towards this distance.
    pub grid_extent: f32,
    pub show_grid: bool,
    pub grid_plane: GridPlane,
    /// The camera is a parallel (orthographic) one: no grid fade, overlays pulled
    /// a fixed amount towards the viewer.
    pub parallel: bool,
    /// How objects are drawn: the view's shading and display style.
    pub display: DisplayMode,
    /// The object types the view shows (its Filter menu).
    pub filter: ViewFilter,
    /// Shadows and ambient occlusion (its Options menu).
    pub effects: ViewEffects,
    /// World bounds (min, max) of the drawn objects, markers left out: what the
    /// key light's shadow map covers.
    pub(crate) bounds: Option<(Vec3, Vec3)>,
    /// The key light: the first light that casts a shadow (its index in
    /// `lights`), and whether its shadow is hard.
    pub(crate) shadow_light: Option<(usize, bool)>,
    /// Colours and sizes of the component overlay.
    pub component_style: ComponentStyle,
    /// The scene's visible lights; refilled (capacity kept) every frame.
    pub(crate) lights: Vec<LightRaw>,
    /// A picture Environment's light all round (its harmonics in the
    /// picture's own frame), set by whoever decodes the picture before
    /// [`extract`](Self::extract); `None` while it is not known.
    pub picture_ambient: Option<(TextureId, Ambient)>,
    /// What surrounds the scene this frame.
    pub(crate) surround: Option<SurroundFrame>,
    /// The last Sky's harmonics (made again only when it changes).
    sky_ambient: Option<(SkyModel, Ambient)>,
    /// Points, edges and polygons of the active object in a component mode.
    pub(crate) components: ComponentOverlay,
    /// One batch per mesh key seen recently; emptied (capacity kept) every frame.
    batches: Vec<Batch>,
    batch_index: HashMap<BatchKey, usize>,
    frame: u64,
}

/// What separates batches: the mesh, its smoothing, its texture coordinates,
/// its textured material and the layer level it draws.
type BatchKey = (MeshKey, Shading, UvKey, Option<MaterialId>, u8);

impl Default for FrameData {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameData {
    #[must_use]
    pub fn new() -> Self {
        Self {
            view_proj: Mat4::IDENTITY,
            view: Mat4::IDENTITY,
            proj: Mat4::IDENTITY,
            eye: Vec3::ZERO,
            light_dir: Vec3::new(-0.45, 0.8, 0.4).normalize(),
            grid_spacing: 100.0,
            grid_extent: 20_000.0,
            show_grid: true,
            grid_plane: GridPlane::XZ,
            parallel: false,
            display: DisplayMode::default(),
            filter: ViewFilter::ALL,
            effects: ViewEffects::default(),
            bounds: None,
            shadow_light: None,
            component_style: ComponentStyle::default(),
            lights: Vec::with_capacity(8),
            picture_ambient: None,
            surround: None,
            sky_ambient: None,
            components: ComponentOverlay::default(),
            batches: Vec::with_capacity(16),
            batch_index: HashMap::with_capacity(16),
            frame: 0,
        }
    }

    /// The non-empty batches of this frame: each mesh key, the polygon mesh it
    /// names (for [`MeshKey::Polygon`]), and its instances.
    pub fn batches(&self) -> impl Iterator<Item = (MeshKey, Option<&MeshHandle>, &[InstanceRaw])> + '_ {
        self.batches
            .iter()
            .filter(|b| !b.instances.is_empty())
            .map(|b| (b.key, b.mesh.as_ref(), b.instances.as_slice()))
    }

    /// [`batches`](Self::batches) with each batch's shading (what the renderer uploads by).
    pub(crate) fn draw_batches(&self) -> impl Iterator<Item = (MeshKey, Shading, Geometry<'_>, &[InstanceRaw])> + '_ {
        self.batches.iter().filter(|b| !b.instances.is_empty()).map(|b| {
            let geometry = Geometry {
                mesh: b.mesh.as_ref(),
                curve: b.curve.as_ref(),
                uvs: b.uvs,
                material: b.material.as_ref(),
                level: b.level,
            };
            (b.key, b.shading, geometry, b.instances.as_slice())
        })
    }

    #[must_use]
    pub fn instance_count(&self) -> usize {
        self.batches.iter().map(|b| b.instances.len()).sum()
    }

    /// This frame's scene lights.
    #[must_use]
    pub fn lights(&self) -> &[LightRaw] {
        &self.lights
    }

    pub fn set_camera(&mut self, camera: &OrbitCamera, aspect: f32) {
        self.view = camera.view();
        self.proj = camera.projection(aspect);
        self.view_proj = self.proj * self.view;
        self.eye = camera.eye();
        self.grid_spacing = camera.grid_spacing();
        self.parallel = camera.is_parallel();
        if self.parallel {
            self.grid_plane = GridPlane::facing(camera.back());
            // What is in view, with room to spare.
            self.grid_extent = (camera.half_height() * aspect.max(1.0) * 3.0).max(self.grid_spacing * 100.0);
        } else {
            self.grid_plane = GridPlane::XZ;
            self.grid_extent = (camera.distance * 12.0).max(self.grid_spacing * 100.0);
        }
        // C4D-style default light: comes from over the viewer's left shoulder.
        self.light_dir = (camera.back() * 0.6 + camera.up() * 0.7 - camera.right() * 0.4).normalize_or(Vec3::Y);
    }

    /// Rebuilds the instance batches from the evaluated scene, without
    /// textures (see [`extract_with_materials`](Self::extract_with_materials)).
    pub fn extract(&mut self, scene: &EvaluatedScene, is_selected: impl Fn(NodeId) -> bool) {
        self.extract_with_materials(scene, None, is_selected);
    }

    /// Rebuilds the instance batches from the evaluated scene; with the
    /// document's `materials`, objects whose top Material tag's material has
    /// textures (or a cut-out) are drawn with them, through the tag's
    /// projection.
    ///
    /// Hot path: allocation-free once every shape has been seen, because batches
    /// are emptied (keeping capacity), never recreated; a new shape allocates its
    /// batch once. Selection is editor state, so it is applied here, per frame.
    pub fn extract_with_materials(
        &mut self,
        scene: &EvaluatedScene,
        materials: Option<&Materials>,
        is_selected: impl Fn(NodeId) -> bool,
    ) {
        self.frame += 1;
        let frame = self.frame;
        for batch in &mut self.batches {
            batch.instances.clear();
        }
        self.lights.clear();
        self.lights.extend(scene.lights().iter().map(LightRaw::new));
        self.shadow_light = scene
            .lights()
            .iter()
            .position(|l| l.shadow.casts())
            .map(|index| (index, scene.lights()[index].shadow.is_hard()));
        // What surrounds the scene: its background (unless the view hides
        // it) and its light; a Sky's sun lights like an Infinite light and
        // casts the shadow when no light does.
        self.surround = scene.environment().map(|environment| {
            let shown = environment.visible && self.filter.shows(FilterItem::Sky);
            SurroundFrame::new(environment, self.picture_ambient, &mut self.sky_ambient, shown)
        });
        let sun = scene.environment().and_then(|environment| match environment.surround {
            Surround::Sky(model) => model.sun(),
            Surround::Picture(_) => None,
        });
        if let Some(sun) = sun {
            self.lights.push(LightRaw::sun(&sun));
            if self.shadow_light.is_none() {
                self.shadow_light = Some((self.lights.len() - 1, false));
            }
        }
        let mut bounds: Option<(Vec3, Vec3)> = None;
        // Objects, then clones (drawn, selected and filtered as their Cloner).
        let filter = self.filter;
        let objects = scene
            .nodes()
            .iter()
            .filter(|n| filter.shows_object(n.kind))
            .map(|n| (n.id, n.world, n.bounds, n.drawable, n.materials, n.unlit));
        let clones = scene
            .instances()
            .iter()
            .filter(|c| filter.shows_object(ObjectKindTag::Cloner) && filter.shows_object(c.kind))
            .map(|c| (c.owner, c.world, c.bounds, c.drawable, c.materials, c.unlit));
        for (id, world, object_bounds, drawable, layers, unlit) in objects.chain(clones) {
            let Some(draw) = drawable else {
                continue;
            };
            let model = world * Mat4::from_scale(draw.scale);
            if !draw.marker
                && let Some(b) = object_bounds
            {
                for i in 0..8u8 {
                    let corner = Vec3::new(
                        if i & 1 == 0 { b.min.x } else { b.max.x },
                        if i & 2 == 0 { b.min.y } else { b.max.y },
                        if i & 4 == 0 { b.min.z } else { b.max.z },
                    );
                    let p = world.transform_point3(corner);
                    bounds = Some(bounds.map_or((p, p), |(min, max)| (min.min(p), max.max(p))));
                }
            }
            // Unit marker meshes fit in the unit sphere.
            if draw.marker && model.inverse().transform_point3(self.eye).length() <= 1.0 {
                continue;
            }
            let selected = f32::from(u8::from(is_selected(id)));
            let surface = &draw.surface;
            let plain = InstanceRaw {
                model: model.to_cols_array_2d(),
                normal: normal_matrix(&model, unlit),
                color: [draw.color[0], draw.color[1], draw.color[2], 1.0],
                params: [
                    selected,
                    f32::from(u8::from(draw.unlit)),
                    surface.roughness,
                    surface.metallic,
                ],
                emission: [
                    surface.emission[0],
                    surface.emission[1],
                    surface.emission[2],
                    f32::from(u8::from(draw.marker)),
                ],
                uv: [1.0, 0.0, 0.0, 1.0],
                uv_offset: [0.0; 4],
            };
            // Its Material tags from the lowest one that covers the whole
            // surface up: the first drawn as usual, each later one over it
            // (level 1, 2, …) wherever its own cut-out leaves it.
            let layers = match materials {
                Some(table) if !draw.marker => {
                    let all = scene.material_layers(layers);
                    let base = all
                        .iter()
                        .rposition(|l| table.get(l.material).is_some_and(covers))
                        .unwrap_or(0);
                    all.get(base..).unwrap_or(&[])
                }
                _ => &[],
            };
            if layers.is_empty() {
                let key = (draw.key, draw.shading, UvKey::Own, None, 0);
                self.push(scene, key, None, plain);
            }
            for (level, layer) in (0u8..).zip(layers) {
                let found = materials.and_then(|table| table.get(layer.material));
                let (uvs, material, transform, camera) = texturing(layer, found, object_bounds, draw.scale);
                let (uv, uv_offset) = uv_fields(transform, camera);
                let mut instance = InstanceRaw { uv, uv_offset, ..plain };
                if let Some(m) = found {
                    let [r, g, b] = m.color;
                    instance.color = [r, g, b, 1.0];
                    instance.params[2] = m.roughness;
                    instance.params[3] = m.metallic;
                    let [er, eg, eb] = m.emitted();
                    instance.emission = [er, eg, eb, 0.0];
                }
                let key = (draw.key, draw.shading, uvs, material.map(|m| m.id), level);
                self.push(scene, key, material.as_ref(), instance);
            }
        }
        self.bounds = bounds;
        // A polygon object's mesh changes id on every edit (each frame of a
        // drag): its old batches go at once, so they never pile up.
        let gone = |b: &Batch| b.is_fleeting() && b.last_used != frame;
        if frame.is_multiple_of(BATCH_TTL_FRAMES) || self.batches.iter().any(gone) {
            self.drop_stale_batches();
        }
    }

    /// Updates the component overlay: `target` is the polygon object and the
    /// component kind of the current mode (`None` in Model mode), `hover` what
    /// the cursor would select (one component, or a loop). Hot path: rebuilds only
    /// what changed.
    pub fn extract_components(
        &mut self,
        scene: &EvaluatedScene,
        target: Option<(NodeId, Component)>,
        hover: &[ComponentHit],
    ) {
        self.components.update(scene, target, hover);
    }

    /// Forgets shapes that have not been drawn for a while (edited away, deleted)
    /// and polygon meshes not drawn this frame.
    fn drop_stale_batches(&mut self) {
        let frame = self.frame;
        self.batches.retain(|b| {
            if b.is_fleeting() {
                b.last_used == frame
            } else {
                b.last_used + BATCH_TTL_FRAMES > frame
            }
        });
        self.batch_index.clear();
        for (slot, batch) in self.batches.iter().enumerate() {
            let key = (
                batch.key,
                batch.shading,
                batch.uvs,
                batch.material.map(|m| m.id),
                batch.level,
            );
            self.batch_index.insert(key, slot);
        }
    }
}

impl Batch {
    /// Shapes that change with every edit or move (a polygon object's mesh, a
    /// curve, a projection's coordinates): dropped as soon as a frame does not
    /// draw them.
    fn is_fleeting(&self) -> bool {
        matches!(self.key, MeshKey::Polygon(_) | MeshKey::Curve(_)) || self.uvs != UvKey::Own
    }
}

/// Whether a Material tag of `material` hides everything below it: nothing
/// of it is cut out.
fn covers(material: &Material) -> bool {
    material.map(Channel::Opacity).is_none() && material.opacity >= material.opacity_threshold
}

/// How a Material tag (`layer`, its material `found`) lies on an object with
/// object-space `bounds` drawn at `scale`: its texture coordinates, its
/// material if it has textures or a cut-out, the tag's change to the
/// coordinates and whether they come from the view (Camera projection).
fn texturing(
    layer: &grid4d_eval::MaterialLayer,
    found: Option<&Material>,
    bounds: Option<grid4d_scene::Aabb>,
    scale: Vec3,
) -> (UvKey, Option<MaterialData>, Affine2, bool) {
    let Some(material) = found.and_then(|m| MaterialData::of(layer.material, m)) else {
        return (UvKey::Own, None, Affine2::IDENTITY, false);
    };
    let mut uvs = UvKey::Own;
    let mut camera = false;
    match layer.projection {
        Projection::Uv => {}
        Projection::Camera => camera = true,
        Projection::Box | Projection::Spherical | Projection::Cylindrical | Projection::Flat => {
            // The drawn mesh is the object's at unit size: its scale first.
            if let Some(to_unit) = layer.to_unit(bounds) {
                uvs = UvKey::projected(layer.projection, to_unit * Mat4::from_scale(scale));
            }
        }
    }
    (uvs, Some(material), layer.uv_transform, camera)
}

impl FrameData {
    /// Adds `instance` to the batch of `key` (made the first time it is seen).
    fn push(&mut self, scene: &EvaluatedScene, key: BatchKey, material: Option<&MaterialData>, instance: InstanceRaw) {
        let frame = self.frame;
        let batches = &mut self.batches;
        let slot = *self.batch_index.entry(key).or_insert_with(|| {
            let (mesh_key, shading, uvs, _, level) = key;
            // A key's mesh never changes (an edited mesh has a new id), so the
            // handle is looked up once, when its batch is created.
            let mesh = match mesh_key {
                MeshKey::Polygon(id) => scene.mesh(id).cloned(),
                _ => None,
            };
            let curve = match mesh_key {
                MeshKey::Curve(id) => scene.curve(id).cloned(),
                _ => None,
            };
            batches.push(Batch {
                key: mesh_key,
                shading,
                uvs,
                material: material.copied(),
                level,
                mesh,
                curve,
                instances: Vec::new(),
                last_used: frame,
            });
            batches.len() - 1
        });
        let batch = &mut self.batches[slot];
        batch.last_used = frame;
        batch.material = material.copied();
        batch.instances.push(instance);
    }
}

fn normal_matrix(model: &Mat4, unlit: u64) -> [[f32; 4]; 3] {
    let m = Mat3::from_mat4(*model);
    let n = if m.determinant().abs() > f32::EPSILON {
        m.inverse().transpose()
    } else {
        Mat3::IDENTITY
    };
    let [a, b, c] = unlit_chunks(unlit);
    [
        n.x_axis.extend(a).to_array(),
        n.y_axis.extend(b).to_array(),
        n.z_axis.extend(c).to_array(),
    ]
}

/// A 64-bit mask as three whole numbers of 24 bits (exact in an f32).
fn unlit_chunks(mask: u64) -> [f32; 3] {
    std::array::from_fn(|i| {
        let chunk = (mask >> (24 * i)) & 0xFF_FFFF;
        // Below 2²⁴: exact.
        f32::from(u16::try_from(chunk >> 8).unwrap_or(u16::MAX)) * 256.0
            + f32::from(u8::try_from(chunk & 0xFF).unwrap_or(u8::MAX))
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test code")]

    use grid4d_scene::{Document, Node, ObjectKind, ObjectKindTag};

    use super::*;

    fn evaluated(doc: &Document) -> EvaluatedScene {
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        eval
    }

    #[test]
    fn the_grid_lies_in_the_view_plane_of_parallel_views() {
        let mut frame = FrameData::new();
        let side = |yaw: f32, pitch: f32| OrbitCamera {
            yaw,
            pitch,
            projection: crate::Projection::Parallel,
            ..OrbitCamera::default()
        };
        frame.set_camera(&OrbitCamera::default(), 1.5);
        assert_eq!((frame.grid_plane, frame.parallel), (GridPlane::XZ, false));
        frame.set_camera(&side(0.0, 0.0), 1.5);
        assert_eq!((frame.grid_plane, frame.parallel), (GridPlane::XY, true), "front");
        frame.set_camera(&side(std::f32::consts::FRAC_PI_2, 0.0), 1.5);
        assert_eq!(frame.grid_plane, GridPlane::ZY, "right");
        frame.set_camera(&side(0.0, std::f32::consts::FRAC_PI_2), 1.5);
        assert_eq!(frame.grid_plane, GridPlane::XZ, "top");
    }

    #[test]
    fn filtered_types_are_not_drawn_but_lights_still_light() {
        let doc = Document::with_demo_scene();
        let eval = evaluated(&doc);
        let mut frame = FrameData::new();
        frame.extract(&eval, |_| false);
        let all = frame.instance_count();
        frame.filter.set(FilterItem::Generator, false);
        frame.filter.set(FilterItem::Light, false);
        frame.extract(&eval, |_| false);
        // Left: the camera marker (the demo's objects are all primitives).
        assert_eq!(frame.instance_count(), 1, "of {all}");
        assert_eq!(frame.lights().len(), 1, "a hidden light still lights");
    }

    #[test]
    fn the_key_light_is_the_first_that_casts_a_shadow() {
        use grid4d_scene::LightShadow;
        let mut doc = Document::new("shadows");
        let light = |shadow| ObjectKind::Light {
            color: [1.0; 3],
            intensity: 1.0,
            light_type: LightType::Omni,
            details: grid4d_scene::LightDetails::default(),
            shadow,
            linking: grid4d_scene::Linking::default(),
            group: String::new(),
        };
        for (name, shadow) in [
            ("Fill", LightShadow::None),
            ("Key", LightShadow::Raytraced),
            ("Rim", LightShadow::ShadowMaps),
        ] {
            doc.scene.insert(Node::new(name, light(shadow)));
        }
        let mut frame = FrameData::new();
        frame.extract(&evaluated(&doc), |_| false);
        assert_eq!(frame.shadow_light, Some((1, true)), "the second light, hard");
        assert_eq!(frame.bounds, None, "only markers: nothing to cover");
    }

    #[test]
    fn instance_layout_is_192_bytes() {
        assert_eq!(size_of::<InstanceRaw>(), 192);
    }

    #[test]
    fn material_tags_over_others_draw_as_higher_levels_from_the_lowest_that_covers() {
        use grid4d_scene::{MaterialTag, Tag, Texture};
        let mut doc = Document::new("Layers");
        let mask = doc.scene.textures_mut().insert(Texture::linked("label.png".into()));
        let paint = doc.scene.materials_mut().insert(Material::new("Paint"));
        let mut red = Material::new("Red");
        red.color = [1.0, 0.0, 0.0];
        let red = doc.scene.materials_mut().insert(red);
        let mut label = Material::new("Label");
        label.color = [0.0, 0.0, 1.0];
        *label.map_mut(Channel::Opacity) = Some(TextureSlot::new(mask));
        let label = doc.scene.materials_mut().insert(label);
        let with = |doc: &mut Document, tags: &[MaterialId]| {
            let mut node = Node::new("Cube", ObjectKindTag::Cube.default_kind());
            node.data.tags = tags.iter().map(|&m| Tag::Material(MaterialTag::new(Some(m)))).collect();
            doc.scene.insert(node);
        };
        // Paint with a label over it; a label hidden under red paint.
        with(&mut doc, &[paint, label]);
        with(&mut doc, &[label, red]);
        let eval = evaluated(&doc);
        let mut frame = FrameData::new();
        frame.extract_with_materials(&eval, Some(doc.scene.materials()), |_| false);
        let mut drawn: Vec<(u8, Option<MaterialId>, [f32; 4])> = frame
            .draw_batches()
            .flat_map(|(_, _, g, instances)| {
                instances
                    .iter()
                    .map(move |i| (g.level, g.material.map(|m| m.id), i.color))
            })
            .collect();
        drawn.sort_by(|a, b| a.0.cmp(&b.0).then(a.2[0].total_cmp(&b.2[0])));
        let grey = Material::new("").color;
        assert_eq!(
            drawn,
            [
                (0, None, [grey[0], grey[1], grey[2], 1.0]),
                (0, None, [1.0, 0.0, 0.0, 1.0]),
                (1, Some(label), [0.0, 0.0, 1.0, 1.0]),
            ],
            "the label over the paint, nothing under the red"
        );
    }

    #[test]
    fn textured_materials_get_their_own_batches_and_projections_their_own_coordinates() {
        use grid4d_scene::{MaterialTag, Tag, Texture};
        let mut doc = Document::new("Textured");
        let wood = doc.scene.textures_mut().insert(Texture::linked("wood.png".into()));
        let plain = doc.scene.materials_mut().insert(Material::new("Plain"));
        let mut oak = Material::new("Oak");
        *oak.map_mut(Channel::BaseColor) = Some(TextureSlot::new(wood));
        let oak = doc.scene.materials_mut().insert(oak);
        let cube = || Node::new("Cube", ObjectKindTag::Cube.default_kind());
        let tagged = |tag: MaterialTag| {
            let mut node = cube();
            node.data.tags = vec![Tag::Material(tag)];
            node
        };
        doc.scene.insert(cube());
        doc.scene.insert(tagged(MaterialTag::new(Some(plain))));
        doc.scene.insert(tagged(MaterialTag {
            tiling: [2.0, 3.0],
            ..MaterialTag::new(Some(oak))
        }));
        doc.scene.insert(tagged(MaterialTag {
            projection: Projection::Box,
            ..MaterialTag::new(Some(oak))
        }));
        doc.scene.insert(tagged(MaterialTag {
            projection: Projection::Camera,
            ..MaterialTag::new(Some(oak))
        }));
        let eval = evaluated(&doc);
        let mut frame = FrameData::new();
        let summary = |frame: &FrameData| {
            frame
                .draw_batches()
                .map(|(_, _, g, i)| {
                    (
                        g.material.map(|m| m.id),
                        matches!(g.uvs, UvKey::Projected { .. }),
                        i.len(),
                    )
                })
                .collect::<Vec<_>>()
        };
        // Without materials: one batch, as in 1.0.
        frame.extract(&eval, |_| false);
        assert_eq!(summary(&frame), [(None, false, 5)]);

        let mut frame = FrameData::new();
        frame.extract_with_materials(&eval, Some(doc.scene.materials()), |_| false);
        let mut batches = summary(&frame);
        batches.sort_by_key(|&(m, projected, _)| (m.is_some(), projected));
        assert_eq!(
            batches,
            [(None, false, 2), (Some(oak), false, 2), (Some(oak), true, 1)],
            "untextured objects batch together; a projection has its own coordinates"
        );
        // The tag's tiling and the Camera flag travel with the instances.
        let instances: Vec<InstanceRaw> = frame
            .draw_batches()
            .filter(|(_, _, g, _)| g.material.is_some() && g.uvs == UvKey::Own)
            .flat_map(|(_, _, _, i)| i.iter().copied())
            .collect();
        assert!(instances.iter().any(|i| i.uv == [2.0, 0.0, 0.0, 3.0]));
        assert!(instances.iter().any(|i| i.uv_offset[2] == 1.0), "the Camera projection");
        let (_, _, projected, _) = frame.draw_batches().find(|(_, _, g, _)| g.uvs != UvKey::Own).unwrap();
        // The unit cube's corner (0.5, 0.5, 0.5) is the fitted box's corner.
        let (projection, matrix) = projected.uvs.projection().unwrap();
        assert_eq!(projection, Projection::Box);
        assert!(matrix.transform_point3(Vec3::splat(0.5)).abs_diff_eq(Vec3::ONE, 1e-5));
    }

    #[test]
    fn light_layout_is_112_bytes() {
        assert_eq!(size_of::<LightRaw>(), 112);
        // The unlit mask survives its trip through floats.
        let mask = 0xA5A5_5A5A_DEAD_BEEF_u64;
        let chunks = unlit_chunks(mask);
        let back = chunks
            .iter()
            .enumerate()
            .map(|(i, &c)| u64::from(c as u32) << (24 * i))
            .fold(0, |a, b| a | b);
        assert_eq!(back, mask);
    }

    #[test]
    fn lights_reach_the_frame_with_their_cone_and_falloff() {
        use grid4d_scene::{LightDetails, Material, Tag, Transform};
        let mut doc = Document::new("lights");
        let spot = ObjectKind::Light {
            color: [1.0, 0.5, 0.25],
            intensity: 2.0,
            light_type: LightType::Spot,
            details: LightDetails {
                inner_angle_deg: 20.0,
                outer_angle_deg: 60.0,
                falloff: Falloff::Linear,
                radius: 300.0,
                ..LightDetails::default()
            },
            shadow: grid4d_scene::LightShadow::None,
            linking: grid4d_scene::Linking::default(),
            group: String::new(),
        };
        // Turned 90° about X: its −Z axis points down.
        doc.scene.insert(Node::new("Spot", spot).with_transform(
            Transform::from_position(Vec3::new(0.0, 400.0, 0.0)).with_rotation_deg(Vec3::new(0.0, -90.0, 0.0)),
        ));
        let mut hidden = Node::new("Off", ObjectKind::light([1.0; 3], 1.0));
        hidden.data.visible_in_editor = false;
        doc.scene.insert(hidden);
        // A material's surface reaches the instances.
        let mut shiny = Material::new("Shiny");
        (shiny.roughness, shiny.metallic, shiny.emission) = (0.2, 1.0, [0.5, 0.0, 0.0]);
        let id = doc.scene.materials_mut().insert(shiny);
        let mut cube = Node::new("Cube", ObjectKindTag::Cube.default_kind());
        cube.data.tags = vec![Tag::Material(grid4d_scene::MaterialTag::new(Some(id)))];
        doc.scene.insert(cube);

        let mut frame = FrameData::new();
        frame.extract(&evaluated(&doc), |_| false);
        assert_eq!(frame.lights().len(), 1, "a light hidden in the editor does not light");
        let light = frame.lights()[0];
        assert_eq!(light.position, [0.0, 400.0, 0.0, 1.0]);
        let direction = Vec3::from_slice(&light.direction[..3]);
        assert!(direction.abs_diff_eq(Vec3::NEG_Y, 1e-5), "{direction}");
        assert!(
            (light.direction[3] - 30f32.to_radians().cos()).abs() < 1e-6,
            "half the outer angle"
        );
        assert!(
            (light.color[3] - 10f32.to_radians().cos()).abs() < 1e-6,
            "half the inner angle"
        );
        assert_eq!(light.color[..3], [2.0, 1.0, 0.5], "colour × intensity");
        assert_eq!(light.falloff[..2], [1.0, 300.0]);

        let cube = frame.batches().find(|(key, _, _)| matches!(key, MeshKey::Cube { .. }));
        let instance = cube.unwrap().2[0];
        assert_eq!(instance.params[2..], [0.2, 1.0]);
        assert_eq!(instance.emission, [0.5, 0.0, 0.0, 0.0]);
        let (min, max) = frame.bounds.unwrap();
        assert!(
            min.abs_diff_eq(Vec3::splat(-100.0), 1e-3) && max.abs_diff_eq(Vec3::splat(100.0), 1e-3),
            "the cube only, not the light's marker: {min} {max}"
        );
        assert_eq!(frame.shadow_light, None, "no light casts a shadow");
    }

    #[test]
    fn extract_reuses_capacity() {
        let doc = Document::with_demo_scene();
        let eval = evaluated(&doc);
        let mut frame = FrameData::new();
        frame.extract(&eval, |_| false);
        let count = frame.instance_count();
        assert!(count >= 5, "demo scene should produce instances, got {count}");
        let caps = |f: &FrameData| -> Vec<usize> { f.batches.iter().map(|b| b.instances.capacity()).collect() };
        let before = (
            caps(&frame),
            frame.batches.capacity(),
            frame.batch_index.capacity(),
            frame.lights.capacity(),
        );
        assert_eq!(frame.lights().len(), 1, "the demo scene's light");
        for _ in 0..(2 * BATCH_TTL_FRAMES) {
            frame.extract(&eval, |_| false);
        }
        assert_eq!(frame.instance_count(), count);
        let after = (
            caps(&frame),
            frame.batches.capacity(),
            frame.batch_index.capacity(),
            frame.lights.capacity(),
        );
        assert_eq!(before, after, "steady-state extraction must not reallocate");
    }

    #[test]
    fn same_shape_different_size_shares_one_mesh() {
        let mut doc = Document::new("instancing");
        for size in [10.0, 20.0, 30.0] {
            doc.scene.insert(Node::new(
                "c",
                ObjectKind::Cube {
                    size: Vec3::splat(size),
                    segments: [2, 2, 2],
                },
            ));
        }
        doc.scene.insert(Node::new(
            "d",
            ObjectKind::Cube {
                size: Vec3::ONE,
                segments: [3, 1, 1],
            },
        ));
        let mut frame = FrameData::new();
        frame.extract(&evaluated(&doc), |_| false);
        let batches: Vec<(MeshKey, usize)> = frame.batches().map(|(k, _, i)| (k, i.len())).collect();
        assert_eq!(
            batches,
            [
                (MeshKey::Cube { segments: [2, 2, 2] }, 3),
                (MeshKey::Cube { segments: [3, 1, 1] }, 1)
            ]
        );
    }

    #[test]
    fn selection_and_scale_reach_the_instances() {
        let doc = Document::with_demo_scene();
        let cube = doc.scene.roots()[0];
        let mut frame = FrameData::new();
        frame.extract(&evaluated(&doc), |id| id == cube);
        let selected: Vec<&InstanceRaw> = frame
            .batches()
            .flat_map(|(_, _, i)| i)
            .filter(|i| i.params[0] == 1.0)
            .collect();
        assert_eq!(selected.len(), 1);
        // The 200 cm cube at y = 100: the unit mesh scaled by its size.
        let model = Mat4::from_cols_array_2d(&selected[0].model);
        let expected = Mat4::from_translation(Vec3::new(0.0, 100.0, 0.0)) * Mat4::from_scale(Vec3::splat(200.0));
        assert!(model.abs_diff_eq(expected, 1e-3));
    }

    #[test]
    fn stale_batches_are_dropped() {
        let mut doc = Document::with_demo_scene();
        let mut eval = evaluated(&doc);
        let mut frame = FrameData::new();
        frame.extract(&eval, |_| false);
        let shapes = frame.batches.len();
        // New segments on the only cylinder: a new shape appears, the old one goes stale.
        let cylinder = doc.scene.iter().find(|(_, n)| n.data.name == "Cylinder").unwrap().0;
        doc.scene.get_mut(cylinder).unwrap().data.kind = ObjectKind::Cylinder {
            radius: 50.0,
            height: 200.0,
            rotation_segments: 7,
            height_segments: 1,
        };
        eval.update(&doc.scene);
        for _ in 0..(2 * BATCH_TTL_FRAMES) {
            frame.extract(&eval, |_| false);
        }
        assert_eq!(frame.batches.len(), shapes, "the old cylinder batch was forgotten");
        assert!(
            frame
                .batches()
                .any(|(k, _, _)| k == MeshKey::Cylinder { rotation: 7, height: 1 })
        );
    }

    #[test]
    fn edited_polygon_meshes_do_not_pile_up() {
        let mesh = MeshHandle::new(grid4d_eval::editable_mesh(&ObjectKindTag::Cube.default_kind()).unwrap());
        let mut doc = Document::new("drag");
        let id = doc.scene.insert(Node::new("Mesh", ObjectKind::polygon(mesh.clone())));
        let mut eval = evaluated(&doc);
        let mut frame = FrameData::new();
        let mut points = mesh.mesh().points().to_vec();
        for i in 0..50u8 {
            points[0].x = f32::from(i);
            doc.scene.get_mut(id).unwrap().data.kind = ObjectKind::polygon(mesh.with_points(&points));
            eval.update(&doc.scene);
            frame.extract(&eval, |_| false);
        }
        frame.extract(&eval, |_| false);
        assert_eq!(frame.batches.len(), 1, "only the current mesh is kept");
    }

    #[test]
    fn a_camera_marker_around_the_eye_is_not_drawn() {
        let camera = OrbitCamera::default();
        let mut doc = Document::new("cameras");
        let kind = ObjectKindTag::Camera.default_kind();
        doc.scene
            .insert(Node::new("At the view", kind.clone()).with_position(camera.eye()));
        doc.scene.insert(Node::new("Elsewhere", kind).with_position(Vec3::ZERO));
        let mut frame = FrameData::new();
        frame.set_camera(&camera, 1.5);
        frame.extract(&evaluated(&doc), |_| false);
        assert_eq!(frame.instance_count(), 1, "only the camera away from the eye is drawn");
        let drawn = Mat4::from_cols_array_2d(&frame.batches().next().unwrap().2[0].model);
        assert!(drawn.transform_point3(Vec3::ZERO).abs_diff_eq(Vec3::ZERO, 1e-4));
    }

    #[test]
    fn polygon_objects_bring_their_mesh_and_share_it_when_they_can() {
        let mesh = MeshHandle::new(grid4d_eval::editable_mesh(&ObjectKindTag::Cube.default_kind()).unwrap());
        let mut doc = Document::new("meshes");
        for x in [0.0, 300.0] {
            let kind = ObjectKind::polygon(mesh.clone());
            doc.scene
                .insert(Node::new("Shared", kind).with_position(Vec3::new(x, 0.0, 0.0)));
        }
        let other = MeshHandle::new(grid4d_eval::editable_mesh(&ObjectKindTag::Sphere.default_kind()).unwrap());
        doc.scene.insert(Node::new("Own", ObjectKind::polygon(other.clone())));
        let mut frame = FrameData::new();
        frame.extract(&evaluated(&doc), |_| false);
        let batches: Vec<(MeshKey, Option<&MeshHandle>, usize)> =
            frame.batches().map(|(k, m, i)| (k, m, i.len())).collect();
        assert_eq!(
            batches,
            [
                (MeshKey::Polygon(mesh.id()), Some(&mesh), 2),
                (MeshKey::Polygon(other.id()), Some(&other), 1)
            ]
        );
        let unscaled = frame.batches().next().unwrap().2[1].model;
        assert_eq!(
            Mat4::from_cols_array_2d(&unscaled),
            Mat4::from_translation(Vec3::new(300.0, 0.0, 0.0))
        );
    }

    #[test]
    fn objects_with_other_phong_angles_get_their_own_batches() {
        let mut doc = Document::new("phong");
        for (name, angle) in [("A", None), ("B", None), ("C", Some(0.0f32))] {
            let mut node = Node::new(name, ObjectKindTag::Cube.default_kind());
            if let Some(angle) = angle {
                node.data.tags = vec![grid4d_scene::Tag::Phong {
                    angle_limit: true,
                    angle,
                }];
            }
            doc.scene.insert(node);
        }
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        let mut frame = FrameData::new();
        frame.extract(&eval, |_| false);
        let mut counts: Vec<usize> = frame.draw_batches().map(|(_, _, _, i)| i.len()).collect();
        counts.sort_unstable();
        assert_eq!(counts, [1, 2], "two cubes share the default, the flat one is apart");
        assert!(
            frame
                .draw_batches()
                .any(|(_, shading, _, _)| shading == Shading::from_angle(0.0))
        );
    }

    #[test]
    fn clones_draw_as_one_instanced_batch_selected_with_their_cloner() {
        use grid4d_geom::cloner::{Cloner, GridArray, Layout};
        let mut doc = Document::new("clones");
        let cloner = doc.scene.insert(Node::new(
            "Cloner",
            ObjectKind::cloner(Cloner {
                layout: Layout::Grid(GridArray {
                    count: [10, 10, 10],
                    ..GridArray::default()
                }),
                ..Cloner::default()
            }),
        ));
        doc.scene
            .insert_child(cloner, Node::new("Cube", ObjectKindTag::Cube.default_kind()))
            .unwrap();
        let eval = evaluated(&doc);
        let mut frame = FrameData::new();
        frame.extract(&eval, |id| id == cloner);
        assert_eq!(frame.instance_count(), 1000);
        assert_eq!(
            frame.batches.iter().filter(|b| !b.instances.is_empty()).count(),
            1,
            "one instanced draw"
        );
        assert!(
            frame.batches[0].instances.iter().all(|i| i.params[0] == 1.0),
            "selected with the Cloner"
        );
        let mut hidden = ViewFilter::ALL;
        hidden.set(FilterItem::Generator, false);
        frame.filter = hidden;
        frame.extract(&eval, |_| false);
        assert_eq!(frame.instance_count(), 0, "filtered with the Cloner");
    }

    #[test]
    fn a_sky_lights_with_its_sun_and_shows_unless_filtered() {
        let mut doc = Document::new("sky");
        doc.scene.insert(Node::new("Cube", ObjectKindTag::Cube.default_kind()));
        doc.scene.insert(Node::new("Sky", ObjectKindTag::Sky.default_kind()));
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        let mut frame = FrameData::new();
        frame.extract(&eval, |_| false);
        assert_eq!(frame.lights().len(), 1, "its sun");
        assert_eq!(frame.shadow_light, Some((0, false)), "casting the shadow");
        let surround = frame.surround.unwrap();
        assert!(matches!(surround.background, crate::sky::Background::Sky(_)));
        assert!(surround.ambient.diffuse(Vec3::Y).y > 0.05, "light from above");
        frame.filter.set(FilterItem::Sky, false);
        frame.extract(&eval, |_| false);
        assert_eq!(frame.surround.unwrap().background, crate::sky::Background::None);
        assert_eq!(frame.lights().len(), 1, "hidden, it still lights");
    }

    #[test]
    fn linked_lights_reach_the_frame_with_their_bit_and_objects_with_their_mask() {
        let mut doc = Document::new("links");
        let cube = doc.scene.insert(Node::new("Cube", ObjectKindTag::Cube.default_kind()));
        doc.scene
            .insert(Node::new("Sphere", ObjectKindTag::Sphere.default_kind()));
        let mut light = ObjectKind::light([1.0; 3], 1.0);
        if let ObjectKind::Light { linking, .. } = &mut light {
            linking.objects.push(cube);
        }
        doc.scene.insert(Node::new("Plain", ObjectKind::light([1.0; 3], 1.0)));
        doc.scene.insert(Node::new("Linked", light));
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        let mut frame = FrameData::new();
        frame.extract(&eval, |_| false);
        let links: Vec<f32> = frame.lights().iter().map(|l| l.link[0]).collect();
        assert_eq!(links, [-1.0, 0.0]);
        let masks: Vec<f32> = frame
            .batches()
            .flat_map(|(_, _, instances)| instances.iter().filter(|i| i.emission[3] < 0.5))
            .map(|i| i.normal[0][3])
            .collect();
        assert!(masks.contains(&1.0) && masks.contains(&0.0), "{masks:?}");
    }
}
