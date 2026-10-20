use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Mat4, Vec3};
use grid4d_eval::{EvaluatedLight, EvaluatedScene, MeshKey, Shading};
use grid4d_geom::Component;
use grid4d_scene::{Falloff, LightType, MeshHandle, NodeId};

use crate::{ComponentHit, ComponentStyle, DisplayMode, OrbitCamera, components::ComponentOverlay};

/// Per-instance GPU data. Layout must match `InstanceIn` in `shaders/mesh.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct InstanceRaw {
    pub model: [[f32; 4]; 4],
    /// Inverse-transpose of the model's upper 3×3 (columns padded to vec4).
    pub normal: [[f32; 4]; 3],
    /// Linear RGB + alpha.
    pub color: [f32; 4],
    /// `x`: selected (0/1), `y`: unlit (0/1), `z`: roughness, `w`: metallic.
    pub params: [f32; 4],
    /// Linear RGB the surface gives off (its material's luminance); `w` unused.
    pub emission: [f32; 4],
}

/// One light for the shaders. Layout must match `Light` in `shaders/mesh.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct LightRaw {
    /// `xyz`: world position, `w`: type (0 omni, 1 spot, 2 infinite).
    pub position: [f32; 4],
    /// `xyz`: the direction it shines in, `w`: cosine of half the outer angle.
    pub direction: [f32; 4],
    /// `rgb`: colour × intensity, `w`: cosine of half the inner angle (the outer
    /// one's for a hard cone).
    pub color: [f32; 4],
    /// `x`: falloff (0 none, 1 linear, 2 inverse square), `y`: radius; `zw` unused.
    pub falloff: [f32; 4],
}

impl LightRaw {
    #[must_use]
    pub fn new(light: &EvaluatedLight) -> Self {
        let code = |n: u8| f32::from(n);
        let kind = match light.light_type {
            LightType::Omni => 0,
            LightType::Spot => 1,
            LightType::Infinite => 2,
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
        Self {
            position: light.position.extend(code(kind)).to_array(),
            direction: light.direction.extend(cos_outer).to_array(),
            color: [light.color[0], light.color[1], light.color[2], cos_inner],
            falloff: [code(falloff), details.radius, 0.0, 0.0],
        }
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
    /// The geometry of a [`MeshKey::Polygon`], for the renderer to upload.
    mesh: Option<MeshHandle>,
    instances: Vec<InstanceRaw>,
    last_used: u64,
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
    /// Colours and sizes of the component overlay.
    pub component_style: ComponentStyle,
    /// The scene's visible lights; refilled (capacity kept) every frame.
    pub(crate) lights: Vec<LightRaw>,
    /// Points, edges and polygons of the active object in a component mode.
    pub(crate) components: ComponentOverlay,
    /// One batch per mesh key seen recently; emptied (capacity kept) every frame.
    batches: Vec<Batch>,
    batch_index: HashMap<(MeshKey, Shading), usize>,
    frame: u64,
}

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
            eye: Vec3::ZERO,
            light_dir: Vec3::new(-0.45, 0.8, 0.4).normalize(),
            grid_spacing: 100.0,
            grid_extent: 20_000.0,
            show_grid: true,
            grid_plane: GridPlane::XZ,
            parallel: false,
            display: DisplayMode::default(),
            component_style: ComponentStyle::default(),
            lights: Vec::with_capacity(8),
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
    pub(crate) fn draw_batches(
        &self,
    ) -> impl Iterator<Item = (MeshKey, Shading, Option<&MeshHandle>, &[InstanceRaw])> + '_ {
        self.batches
            .iter()
            .filter(|b| !b.instances.is_empty())
            .map(|b| (b.key, b.shading, b.mesh.as_ref(), b.instances.as_slice()))
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
        self.view_proj = camera.view_projection(aspect);
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

    /// Rebuilds the instance batches from the evaluated scene.
    ///
    /// Hot path: allocation-free once every shape has been seen, because batches
    /// are emptied (keeping capacity), never recreated; a new shape allocates its
    /// batch once. Selection is editor state, so it is applied here, per frame.
    pub fn extract(&mut self, scene: &EvaluatedScene, is_selected: impl Fn(NodeId) -> bool) {
        self.frame += 1;
        let frame = self.frame;
        for batch in &mut self.batches {
            batch.instances.clear();
        }
        self.lights.clear();
        self.lights.extend(scene.lights().iter().map(LightRaw::new));
        for node in scene.nodes() {
            let Some(draw) = node.drawable else { continue };
            let model = node.world * Mat4::from_scale(draw.scale);
            // Unit marker meshes fit in the unit sphere.
            if draw.marker && model.inverse().transform_point3(self.eye).length() <= 1.0 {
                continue;
            }
            let batches = &mut self.batches;
            let slot = *self.batch_index.entry((draw.key, draw.shading)).or_insert_with(|| {
                // A key's mesh never changes (an edited mesh has a new id), so the
                // handle is looked up once, when its batch is created.
                let mesh = match draw.key {
                    MeshKey::Polygon(id) => scene.mesh(id).cloned(),
                    _ => None,
                };
                batches.push(Batch {
                    key: draw.key,
                    shading: draw.shading,
                    mesh,
                    instances: Vec::new(),
                    last_used: frame,
                });
                batches.len() - 1
            });
            let batch = &mut self.batches[slot];
            batch.last_used = frame;
            let surface = &draw.surface;
            batch.instances.push(InstanceRaw {
                model: model.to_cols_array_2d(),
                normal: normal_matrix(&model),
                color: [draw.color[0], draw.color[1], draw.color[2], 1.0],
                params: [
                    f32::from(u8::from(is_selected(node.id))),
                    f32::from(u8::from(draw.unlit)),
                    surface.roughness,
                    surface.metallic,
                ],
                emission: [surface.emission[0], surface.emission[1], surface.emission[2], 0.0],
            });
        }
        // A polygon object's mesh changes id on every edit (each frame of a
        // drag): its old batches go at once, so they never pile up.
        let gone = |b: &Batch| matches!(b.key, MeshKey::Polygon(_)) && b.last_used != frame;
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
        self.batches.retain(|b| match b.key {
            MeshKey::Polygon(_) => b.last_used == frame,
            _ => b.last_used + BATCH_TTL_FRAMES > frame,
        });
        self.batch_index.clear();
        for (slot, batch) in self.batches.iter().enumerate() {
            self.batch_index.insert((batch.key, batch.shading), slot);
        }
    }
}

fn normal_matrix(model: &Mat4) -> [[f32; 4]; 3] {
    let m = Mat3::from_mat4(*model);
    let n = if m.determinant().abs() > f32::EPSILON {
        m.inverse().transpose()
    } else {
        Mat3::IDENTITY
    };
    [
        n.x_axis.extend(0.0).to_array(),
        n.y_axis.extend(0.0).to_array(),
        n.z_axis.extend(0.0).to_array(),
    ]
}

#[cfg(test)]
mod tests {
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
    fn instance_layout_is_160_bytes() {
        assert_eq!(size_of::<InstanceRaw>(), 160);
    }

    #[test]
    fn light_layout_is_64_bytes() {
        assert_eq!(size_of::<LightRaw>(), 64);
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
        cube.data.tags = vec![Tag::Material { material: Some(id) }];
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
}
