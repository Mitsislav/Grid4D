use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Mat4, Vec3};
use oc4d_scene::{CAMERA_MARKER_SIZE, LIGHT_MARKER_RADIUS, NodeId, ObjectKind, Scene, WalkItem};

use crate::{
    OrbitCamera,
    mesh::{MeshKey, Ratio},
};

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
    instances: Vec<InstanceRaw>,
    last_used: u64,
}

/// Everything the GPU needs to draw one viewport frame — a render-side snapshot
/// of the scene. Owned by the UI, filled every frame, consumed by
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
    walk_stack: Vec<WalkItem>,
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
            walk_stack: Vec::with_capacity(64),
        }
    }

    /// The non-empty batches of this frame: each mesh key with its instances.
    pub fn batches(&self) -> impl Iterator<Item = (MeshKey, &[InstanceRaw])> + '_ {
        self.batches
            .iter()
            .filter(|b| !b.instances.is_empty())
            .map(|b| (b.key, b.instances.as_slice()))
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

    /// Rebuilds the instance batches from `scene`.
    ///
    /// Hot path: allocation-free once every shape in the scene has been seen,
    /// because batches are emptied (keeping capacity), never recreated. A new
    /// shape allocates its batch once. Hidden or disabled objects hide their
    /// whole subtree.
    pub fn extract(&mut self, scene: &Scene, is_selected: impl Fn(NodeId) -> bool) {
        self.frame += 1;
        for batch in &mut self.batches {
            batch.instances.clear();
        }
        let (batches, index, frame) = (&mut self.batches, &mut self.batch_index, self.frame);
        scene.walk(&mut self.walk_stack, |id, node, world, _depth| {
            if !node.data.visible_in_editor || !node.data.enabled {
                return false;
            }
            if let Some(style) = draw_style(&node.data.kind, node.data.display_color) {
                let model = *world * Mat4::from_scale(style.scale);
                let slot = *index.entry(style.key).or_insert_with(|| {
                    batches.push(Batch {
                        key: style.key,
                        instances: Vec::new(),
                        last_used: frame,
                    });
                    batches.len() - 1
                });
                let batch = &mut batches[slot];
                batch.last_used = frame;
                batch.instances.push(InstanceRaw {
                    model: model.to_cols_array_2d(),
                    normal: normal_matrix(&model),
                    color: [style.color[0], style.color[1], style.color[2], 1.0],
                    params: [
                        f32::from(u8::from(is_selected(id))),
                        f32::from(u8::from(style.unlit)),
                        0.0,
                        0.0,
                    ],
                });
            }
            true
        });
        if self.frame.is_multiple_of(BATCH_TTL_FRAMES) {
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

/// How an object is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DrawStyle {
    pub key: MeshKey,
    /// Object-space scale applied to the unit mesh.
    pub scale: Vec3,
    pub color: [f32; 3],
    pub unlit: bool,
}

/// The unit mesh, scale and colour for an object; `None` for objects without
/// geometry (Nulls, shapes with zero size).
pub(crate) fn draw_style(kind: &ObjectKind, color: [f32; 3]) -> Option<DrawStyle> {
    let shaded = |key, scale| {
        Some(DrawStyle {
            key,
            scale,
            color,
            unlit: false,
        })
    };
    match *kind {
        ObjectKind::Null => None,
        ObjectKind::Cube { size, segments } => shaded(MeshKey::Cube { segments }, size),
        ObjectKind::Sphere { radius, segments } => shaded(MeshKey::Sphere { segments }, Vec3::splat(radius)),
        ObjectKind::Cylinder {
            radius,
            height,
            rotation_segments,
            height_segments,
        } => shaded(
            MeshKey::Cylinder {
                rotation: rotation_segments,
                height: height_segments,
            },
            Vec3::new(radius, height, radius),
        ),
        ObjectKind::Plane {
            width,
            depth,
            width_segments,
            depth_segments,
        } => shaded(
            MeshKey::Plane {
                width: width_segments,
                depth: depth_segments,
            },
            Vec3::new(width, 1.0, depth),
        ),
        ObjectKind::Cone {
            bottom_radius,
            top_radius,
            height,
            rotation_segments,
            height_segments,
        } => {
            let (rb, rt) = (bottom_radius.max(0.0), top_radius.max(0.0));
            let r = rb.max(rt);
            if r <= 0.0 {
                return None;
            }
            shaded(
                MeshKey::Cone {
                    bottom: Ratio::new(rb, r),
                    top: Ratio::new(rt, r),
                    rotation: rotation_segments,
                    height: height_segments,
                },
                Vec3::new(r, height, r),
            )
        }
        ObjectKind::Torus {
            ring_radius,
            pipe_radius,
            ring_segments,
            pipe_segments,
        } => {
            let outer = ring_radius.abs() + pipe_radius.abs();
            if outer <= 0.0 {
                return None;
            }
            shaded(
                MeshKey::Torus {
                    ring: Ratio::new(ring_radius.abs(), outer),
                    ring_segments,
                    pipe_segments,
                },
                Vec3::splat(outer),
            )
        }
        ObjectKind::Disc {
            inner_radius,
            outer_radius,
            rotation_segments,
            disc_segments,
        } => {
            let (ri, ro) = (inner_radius.max(0.0), outer_radius.max(0.0));
            let r = ri.max(ro);
            if r <= 0.0 || ro == ri {
                return None; // no area
            }
            shaded(
                MeshKey::Disc {
                    inner: Ratio::new(ri.min(ro), r),
                    outer: Ratio::new(ri.max(ro), r),
                    rotation: rotation_segments,
                    segments: disc_segments,
                },
                Vec3::new(r, 1.0, r),
            )
        }
        ObjectKind::Capsule {
            radius,
            height,
            rotation_segments,
            height_segments,
            cap_segments,
        } => {
            let r = radius.abs();
            if r <= 0.0 {
                return None;
            }
            let total = height.abs().max(2.0 * r);
            shaded(
                MeshKey::Capsule {
                    straight: Ratio::new(total - 2.0 * r, total),
                    rotation: rotation_segments,
                    height: height_segments,
                    cap: cap_segments,
                },
                Vec3::splat(r),
            )
        }
        ObjectKind::Camera { .. } => shaded(MeshKey::Cube { segments: [1, 1, 1] }, Vec3::splat(CAMERA_MARKER_SIZE)),
        ObjectKind::Light { color: light, .. } => Some(DrawStyle {
            key: MeshKey::Sphere { segments: 24 },
            scale: Vec3::splat(LIGHT_MARKER_RADIUS),
            color: light,
            unlit: true,
        }),
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
    use oc4d_scene::{Document, Node, ObjectKindTag};

    use super::*;

    #[test]
    fn instance_layout_is_144_bytes() {
        assert_eq!(size_of::<InstanceRaw>(), 144);
    }

    #[test]
    fn extract_reuses_capacity() {
        let doc = Document::with_demo_scene();
        let mut frame = FrameData::new();
        frame.extract(&doc.scene, |_| false);
        let count = frame.instance_count();
        assert!(count >= 5, "demo scene should produce instances, got {count}");
        let caps = |f: &FrameData| -> Vec<usize> { f.batches.iter().map(|b| b.instances.capacity()).collect() };
        let before = (caps(&frame), frame.batches.capacity(), frame.batch_index.capacity());
        for _ in 0..(2 * BATCH_TTL_FRAMES) {
            frame.extract(&doc.scene, |_| false);
        }
        assert_eq!(frame.instance_count(), count);
        let after = (caps(&frame), frame.batches.capacity(), frame.batch_index.capacity());
        assert_eq!(before, after, "steady-state extraction must not reallocate");
    }

    #[test]
    fn hidden_parent_hides_children() {
        let mut doc = Document::with_demo_scene();
        let group = doc
            .scene
            .iter()
            .find(|(_, n)| n.data.name == "Group")
            .map(|(id, _)| id)
            .unwrap();
        let mut frame = FrameData::new();
        frame.extract(&doc.scene, |_| false);
        let before = frame.instance_count();
        doc.scene.get_mut(group).unwrap().data.visible_in_editor = false;
        frame.extract(&doc.scene, |_| false);
        assert_eq!(frame.instance_count(), before - 2);
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
        frame.extract(&doc.scene, |_| false);
        let batches: Vec<(MeshKey, usize)> = frame.batches().map(|(k, i)| (k, i.len())).collect();
        assert_eq!(
            batches,
            [
                (MeshKey::Cube { segments: [2, 2, 2] }, 3),
                (MeshKey::Cube { segments: [3, 1, 1] }, 1)
            ]
        );
    }

    #[test]
    fn shape_ratios_are_part_of_the_key_and_size_is_the_scale() {
        let torus = |ring, pipe| ObjectKind::Torus {
            ring_radius: ring,
            pipe_radius: pipe,
            ring_segments: 36,
            pipe_segments: 18,
        };
        let small = draw_style(&torus(200.0, 50.0), [1.0; 3]).unwrap();
        let big = draw_style(&torus(400.0, 100.0), [1.0; 3]).unwrap();
        let fat = draw_style(&torus(200.0, 100.0), [1.0; 3]).unwrap();
        assert_eq!(small.key, big.key, "uniformly scaled: same mesh");
        assert_eq!(big.scale, Vec3::splat(500.0));
        assert_ne!(small.key, fat.key, "a thicker pipe is another shape");
    }

    #[test]
    fn every_kind_has_a_draw_style_except_nulls_and_empty_shapes() {
        for tag in ObjectKindTag::ALL {
            let style = draw_style(&tag.default_kind(), [1.0; 3]);
            assert_eq!(style.is_some(), tag != ObjectKindTag::Null, "{tag:?}");
        }
        let flat_cone = ObjectKind::Cone {
            bottom_radius: 0.0,
            top_radius: 0.0,
            height: 10.0,
            rotation_segments: 8,
            height_segments: 1,
        };
        assert_eq!(draw_style(&flat_cone, [1.0; 3]), None);
    }

    #[test]
    fn stale_batches_are_dropped() {
        let mut doc = Document::with_demo_scene();
        let mut frame = FrameData::new();
        frame.extract(&doc.scene, |_| false);
        let shapes = frame.batches.len();
        // New segments on the only cylinder: a new shape appears, the old one goes stale.
        let cylinder = doc.scene.iter().find(|(_, n)| n.data.name == "Cylinder").unwrap().0;
        doc.scene.get_mut(cylinder).unwrap().data.kind = ObjectKind::Cylinder {
            radius: 50.0,
            height: 200.0,
            rotation_segments: 7,
            height_segments: 1,
        };
        for _ in 0..(2 * BATCH_TTL_FRAMES) {
            frame.extract(&doc.scene, |_| false);
        }
        assert_eq!(frame.batches.len(), shapes, "the old cylinder batch was forgotten");
        assert!(
            frame
                .batches()
                .any(|(k, _)| k == MeshKey::Cylinder { rotation: 7, height: 1 })
        );
    }
}
