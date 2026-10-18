use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Mat4, Vec3};
use oc4d_scene::{CAMERA_MARKER_SIZE, LIGHT_MARKER_RADIUS, NodeId, ObjectKind, Scene, WalkItem};

use crate::OrbitCamera;

/// GPU mesh used to draw an object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MeshKind {
    Cube,
    Sphere,
    Cylinder,
    Plane,
}

impl MeshKind {
    pub const COUNT: usize = 4;
    pub const ALL: [Self; Self::COUNT] = [Self::Cube, Self::Sphere, Self::Cylinder, Self::Plane];

    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }
}

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
    batches: [Vec<InstanceRaw>; MeshKind::COUNT],
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
            batches: std::array::from_fn(|_| Vec::with_capacity(256)),
            walk_stack: Vec::with_capacity(64),
        }
    }

    #[must_use]
    pub fn batch(&self, kind: MeshKind) -> &[InstanceRaw] {
        &self.batches[kind.index()]
    }

    #[must_use]
    pub fn instance_count(&self) -> usize {
        self.batches.iter().map(Vec::len).sum()
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
    /// Hot path: allocation-free once the batches have reached the scene's size,
    /// because the vectors are cleared (keeping capacity), never recreated.
    /// Hidden or disabled objects hide their whole subtree.
    pub fn extract(&mut self, scene: &Scene, is_selected: impl Fn(NodeId) -> bool) {
        for batch in &mut self.batches {
            batch.clear();
        }
        let batches = &mut self.batches;
        scene.walk(&mut self.walk_stack, |id, node, world, _depth| {
            if !node.data.visible_in_editor || !node.data.enabled {
                return false;
            }
            if let Some((mesh, scale, color, unlit)) = draw_style(&node.data.kind, node.data.display_color) {
                let model = *world * Mat4::from_scale(scale);
                batches[mesh.index()].push(InstanceRaw {
                    model: model.to_cols_array_2d(),
                    normal: normal_matrix(&model),
                    color: [color[0], color[1], color[2], 1.0],
                    params: [
                        f32::from(u8::from(is_selected(id))),
                        f32::from(u8::from(unlit)),
                        0.0,
                        0.0,
                    ],
                });
            }
            true
        });
    }
}

/// How an object kind is drawn: mesh, object-space scale, colour, unlit flag.
fn draw_style(kind: &ObjectKind, color: [f32; 3]) -> Option<(MeshKind, Vec3, [f32; 3], bool)> {
    Some(match *kind {
        ObjectKind::Null => return None,
        ObjectKind::Cube { size } => (MeshKind::Cube, size, color, false),
        ObjectKind::Sphere { radius } => (MeshKind::Sphere, Vec3::splat(radius), color, false),
        ObjectKind::Cylinder { radius, height } => {
            (MeshKind::Cylinder, Vec3::new(radius, height, radius), color, false)
        }
        ObjectKind::Plane { width, depth } => (MeshKind::Plane, Vec3::new(width, 1.0, depth), color, false),
        ObjectKind::Camera { .. } => (MeshKind::Cube, Vec3::splat(CAMERA_MARKER_SIZE), color, false),
        ObjectKind::Light { color: light, .. } => (MeshKind::Sphere, Vec3::splat(LIGHT_MARKER_RADIUS), light, true),
    })
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
    use oc4d_scene::Document;

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
        let caps: Vec<_> = frame.batches.iter().map(Vec::capacity).collect();
        for _ in 0..10 {
            frame.extract(&doc.scene, |_| false);
        }
        assert_eq!(frame.instance_count(), count);
        let caps_after: Vec<_> = frame.batches.iter().map(Vec::capacity).collect();
        assert_eq!(caps, caps_after, "steady-state extraction must not reallocate");
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
}
