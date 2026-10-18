use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Mat4, Vec3};
use grid4d_eval::{EvaluatedScene, MeshKey};
use grid4d_scene::{MeshHandle, NodeId};

use crate::OrbitCamera;

/// Per-instance GPU data. Layout must match `InstanceIn` in `shaders/mesh.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct InstanceRaw {
    pub model: [[f32; 4]; 4],
    /// Inverse-transpose of the model's upper 3×3 (columns padded to vec4).
    pub normal: [[f32; 4]; 3],
    /// Linear RGB + alpha.
    pub color: [f32; 4],
    /// `x`: selected (0/1), `y`: unlit (0/1), `z`/`w`: reserved.
    pub params: [f32; 4],
}

/// Batches unused for this many frames are dropped (their shape is gone).
const BATCH_TTL_FRAMES: u64 = 240;

/// The instances drawn with one mesh this frame.
#[derive(Debug)]
struct Batch {
    key: MeshKey,
    /// The geometry of a [`MeshKey::Polygon`], for the renderer to upload.
    mesh: Option<MeshHandle>,
    instances: Vec<InstanceRaw>,
    last_used: u64,
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
    /// One batch per mesh key seen recently; emptied (capacity kept) every frame.
    batches: Vec<Batch>,
    batch_index: HashMap<MeshKey, usize>,
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

    #[must_use]
    pub fn instance_count(&self) -> usize {
        self.batches.iter().map(|b| b.instances.len()).sum()
    }

    pub fn set_camera(&mut self, camera: &OrbitCamera, aspect: f32) {
        self.view_proj = camera.view_projection(aspect);
        self.eye = camera.eye();
        self.grid_spacing = camera.grid_spacing();
        self.grid_extent = (camera.distance * 12.0).max(self.grid_spacing * 100.0);
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
        for node in scene.nodes() {
            let Some(draw) = node.drawable else { continue };
            let model = node.world * Mat4::from_scale(draw.scale);
            let batches = &mut self.batches;
            let slot = *self.batch_index.entry(draw.key).or_insert_with(|| {
                // A key's mesh never changes (an edited mesh has a new id), so the
                // handle is looked up once, when its batch is created.
                let mesh = match draw.key {
                    MeshKey::Polygon(id) => scene.mesh(id).cloned(),
                    _ => None,
                };
                batches.push(Batch {
                    key: draw.key,
                    mesh,
                    instances: Vec::new(),
                    last_used: frame,
                });
                batches.len() - 1
            });
            let batch = &mut self.batches[slot];
            batch.last_used = frame;
            batch.instances.push(InstanceRaw {
                model: model.to_cols_array_2d(),
                normal: normal_matrix(&model),
                color: [draw.color[0], draw.color[1], draw.color[2], 1.0],
                params: [
                    f32::from(u8::from(is_selected(node.id))),
                    f32::from(u8::from(draw.unlit)),
                    0.0,
                    0.0,
                ],
            });
        }
        if frame.is_multiple_of(BATCH_TTL_FRAMES) {
            self.drop_stale_batches();
        }
    }

    /// Forgets shapes that have not been drawn for a while (edited away, deleted).
    fn drop_stale_batches(&mut self) {
        let frame = self.frame;
        self.batches.retain(|b| b.last_used + BATCH_TTL_FRAMES > frame);
        self.batch_index.clear();
        for (slot, batch) in self.batches.iter().enumerate() {
            self.batch_index.insert(batch.key, slot);
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
    fn instance_layout_is_144_bytes() {
        assert_eq!(size_of::<InstanceRaw>(), 144);
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
        let before = (caps(&frame), frame.batches.capacity(), frame.batch_index.capacity());
        for _ in 0..(2 * BATCH_TTL_FRAMES) {
            frame.extract(&eval, |_| false);
        }
        assert_eq!(frame.instance_count(), count);
        let after = (caps(&frame), frame.batches.capacity(), frame.batch_index.capacity());
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
    fn polygon_objects_bring_their_mesh_and_share_it_when_they_can() {
        let mesh = MeshHandle::new(grid4d_eval::editable_mesh(&ObjectKindTag::Cube.default_kind()).unwrap());
        let mut doc = Document::new("meshes");
        for x in [0.0, 300.0] {
            let kind = ObjectKind::Polygon { mesh: mesh.clone() };
            doc.scene
                .insert(Node::new("Shared", kind).with_position(Vec3::new(x, 0.0, 0.0)));
        }
        let other = MeshHandle::new(grid4d_eval::editable_mesh(&ObjectKindTag::Sphere.default_kind()).unwrap());
        doc.scene
            .insert(Node::new("Own", ObjectKind::Polygon { mesh: other.clone() }));
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
}
