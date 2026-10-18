use glam::Mat4;
use grid4d_scene::{Aabb, LIGHT_MARKER_RADIUS, NodeId, ObjectKind, Scene, WalkItem};

use crate::{Drawable, drawable};

/// How picking tests a ray against a node, in its object space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PickShape {
    /// Not pickable in the viewport (a Null): select it in the Object Manager.
    None,
    /// The node's object-space bounds (an oriented box in world space).
    Bounds,
    /// An exact sphere of this radius around the origin.
    Sphere { radius: f32 },
}

/// One visible node, as viewers see it this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EvaluatedNode {
    pub id: NodeId,
    /// Hierarchy depth (0 for top-level objects).
    pub depth: u32,
    pub world: Mat4,
    /// Object-space bounds (framing, picking, selection brackets); `None` for Nulls.
    pub bounds: Option<Aabb>,
    /// What to draw; `None` for objects without geometry.
    pub drawable: Option<Drawable>,
    pub pick: PickShape,
}

/// The document as viewers see it: every node that is visible in the editor and
/// enabled (a hidden or disabled node hides its whole subtree), in Object Manager
/// pre-order, with world matrices.
///
/// Keep one per document view and call [`EvaluatedScene::update`] each frame
/// before drawing or picking.
#[derive(Debug, Default)]
pub struct EvaluatedScene {
    nodes: Vec<EvaluatedNode>,
    walk_stack: Vec<WalkItem>,
    /// [`Scene::revision`] of the scene `nodes` were built from.
    revision: Option<u64>,
}

impl EvaluatedScene {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Brings the evaluation up to date with `scene`. Returns `true` if it was
    /// rebuilt, `false` if the scene had not changed since the last call.
    ///
    /// Hot path: costs nothing for an unchanged scene, and a rebuild reuses every
    /// buffer, so steady state allocates nothing.
    pub fn update(&mut self, scene: &Scene) -> bool {
        if self.revision == Some(scene.revision()) {
            return false;
        }
        self.nodes.clear();
        let nodes = &mut self.nodes;
        scene.walk(&mut self.walk_stack, |id, node, world, depth| {
            let data = &node.data;
            if !data.visible_in_editor || !data.enabled {
                return false;
            }
            nodes.push(EvaluatedNode {
                id,
                depth,
                world: *world,
                bounds: data.kind.local_bounds(),
                drawable: drawable(&data.kind, data.display_color),
                pick: pick_shape(&data.kind),
            });
            true
        });
        self.revision = Some(scene.revision());
        true
    }

    /// Visible nodes in Object Manager pre-order.
    #[must_use]
    pub fn nodes(&self) -> &[EvaluatedNode] {
        &self.nodes
    }
}

fn pick_shape(kind: &ObjectKind) -> PickShape {
    match *kind {
        ObjectKind::Null => PickShape::None,
        ObjectKind::Sphere { radius, .. } => PickShape::Sphere { radius },
        ObjectKind::Light { .. } => PickShape::Sphere {
            radius: LIGHT_MARKER_RADIUS,
        },
        ObjectKind::Cube { .. }
        | ObjectKind::Cylinder { .. }
        | ObjectKind::Plane { .. }
        | ObjectKind::Cone { .. }
        | ObjectKind::Torus { .. }
        | ObjectKind::Disc { .. }
        | ObjectKind::Capsule { .. }
        | ObjectKind::Camera { .. } => PickShape::Bounds,
    }
}

#[cfg(test)]
mod tests {
    use grid4d_scene::Document;

    use super::*;

    fn id_of(doc: &Document, name: &str) -> NodeId {
        doc.scene.iter().find(|(_, n)| n.data.name == name).unwrap().0
    }

    #[test]
    fn an_unchanged_scene_is_not_evaluated_again() {
        let mut doc = Document::with_demo_scene();
        let mut eval = EvaluatedScene::new();
        assert!(eval.update(&doc.scene));
        assert!(!eval.update(&doc.scene), "nothing changed");
        let _ = doc.scene.get(id_of(&doc, "Cube"));
        assert!(!eval.update(&doc.scene), "reading is not changing");

        let cube = id_of(&doc, "Cube");
        doc.scene.get_mut(cube).unwrap().data.transform.position.x = 42.0;
        assert!(eval.update(&doc.scene));
        let node = eval.nodes().iter().find(|n| n.id == cube).unwrap();
        assert_eq!(node.world.w_axis.x, 42.0);

        // A replaced document is always re-evaluated.
        doc = Document::with_demo_scene();
        assert!(eval.update(&doc.scene));
    }

    #[test]
    fn steady_state_updates_reuse_capacity() {
        let mut doc = Document::with_demo_scene();
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        let capacity = (eval.nodes.capacity(), eval.walk_stack.capacity());
        let cube = id_of(&doc, "Cube");
        for i in 0..100 {
            doc.scene.get_mut(cube).unwrap().data.transform.position.y = f32::from(u8::try_from(i).unwrap());
            assert!(eval.update(&doc.scene));
        }
        assert_eq!((eval.nodes.capacity(), eval.walk_stack.capacity()), capacity);
    }

    #[test]
    fn world_matrices_and_order_match_the_scene() {
        let doc = Document::with_demo_scene();
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        let mut expected = Vec::new();
        doc.scene.walk(&mut Vec::new(), |id, _, world, depth| {
            expected.push((id, depth, *world));
            true
        });
        let got: Vec<_> = eval.nodes().iter().map(|n| (n.id, n.depth, n.world)).collect();
        assert_eq!(got, expected);
        for node in eval.nodes() {
            assert!(node.world.abs_diff_eq(doc.scene.world_matrix(node.id), 1e-4));
        }
    }

    #[test]
    fn hidden_or_disabled_nodes_hide_their_subtree() {
        let mut doc = Document::with_demo_scene();
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        let all = eval.nodes().len();

        let group = id_of(&doc, "Group");
        doc.scene.get_mut(group).unwrap().data.visible_in_editor = false;
        eval.update(&doc.scene);
        assert_eq!(eval.nodes().len(), all - 3, "Group, Cylinder and Box");

        doc.scene.get_mut(group).unwrap().data.visible_in_editor = true;
        let light = id_of(&doc, "Light");
        doc.scene.get_mut(light).unwrap().data.enabled = false;
        eval.update(&doc.scene);
        assert_eq!(eval.nodes().len(), all - 1);
    }

    #[test]
    fn nulls_draw_nothing_and_spheres_pick_exactly() {
        let doc = Document::with_demo_scene();
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        let node = |name| *eval.nodes().iter().find(|n| n.id == id_of(&doc, name)).unwrap();
        let group = node("Group");
        assert_eq!(
            (group.drawable, group.pick, group.bounds),
            (None, PickShape::None, None)
        );
        assert_eq!(node("Sphere").pick, PickShape::Sphere { radius: 80.0 });
        assert_eq!(node("Cube").pick, PickShape::Bounds);
        assert!(node("Cube").drawable.is_some());
    }
}
