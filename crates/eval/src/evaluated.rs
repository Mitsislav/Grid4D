use std::collections::HashMap;

use glam::Mat4;
use grid4d_geom::BitSet;
use grid4d_scene::{Aabb, Components, LIGHT_MARKER_RADIUS, MeshHandle, MeshId, NodeId, ObjectKind, Scene, WalkItem};

use crate::{Drawable, MeshKey, drawable};

/// How picking tests a ray against a node, in its object space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PickShape {
    /// Not pickable in the viewport (a Null): select it in the Object Manager.
    None,
    /// The node's object-space bounds (an oriented box in world space).
    Bounds,
    /// An exact sphere of this radius around the origin.
    Sphere { radius: f32 },
    /// A polygon object's shown mesh, exactly (see [`EvaluatedScene::mesh`]).
    Mesh(MeshId),
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
    /// The meshes of the visible polygon objects as shown (hidden polygons left
    /// out), for drawing and picking.
    meshes: HashMap<MeshId, MeshHandle>,
    /// The visible polygon objects' own meshes and components (component modes).
    polygons: HashMap<NodeId, (MeshHandle, Components)>,
    /// Shown meshes of polygon objects with hidden polygons, kept across updates
    /// so one is built only when the mesh or its hidden polygons change.
    shown: HashMap<NodeId, Shown>,
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
        self.meshes.clear();
        self.polygons.clear();
        let (nodes, meshes, polygons, shown) = (&mut self.nodes, &mut self.meshes, &mut self.polygons, &mut self.shown);
        scene.walk(&mut self.walk_stack, |id, node, world, depth| {
            let data = &node.data;
            if !data.visible_in_editor || !data.enabled {
                return false;
            }
            let mut drawable = drawable(&data.kind, data.display_color);
            let mut pick = pick_shape(&data.kind);
            if let ObjectKind::Polygon { mesh, components } = &data.kind {
                polygons.insert(id, (mesh.clone(), components.clone()));
                let hidden = &components.hidden().polygons;
                let mesh = if hidden.is_empty() {
                    mesh
                } else {
                    &shown_mesh(shown, id, mesh, hidden).mesh
                };
                meshes.insert(mesh.id(), mesh.clone());
                pick = PickShape::Mesh(mesh.id());
                drawable = drawable.filter(|_| mesh.mesh().face_count() > 0).map(|d| Drawable {
                    key: MeshKey::Polygon(mesh.id()),
                    ..d
                });
            }
            nodes.push(EvaluatedNode {
                id,
                depth,
                world: *world,
                bounds: data.kind.local_bounds(),
                drawable,
                pick,
            });
            true
        });
        // Objects gone, hidden or with nothing hidden any more.
        self.shown.retain(|_, entry| self.meshes.contains_key(&entry.mesh.id()));
        self.revision = Some(scene.revision());
        true
    }

    /// Visible nodes in Object Manager pre-order.
    #[must_use]
    pub fn nodes(&self) -> &[EvaluatedNode] {
        &self.nodes
    }

    /// The mesh behind a [`MeshKey::Polygon`](crate::MeshKey::Polygon) or
    /// [`PickShape::Mesh`], if a visible object uses it.
    #[must_use]
    pub fn mesh(&self, id: MeshId) -> Option<&MeshHandle> {
        self.meshes.get(&id)
    }

    /// A visible polygon object's node, own mesh (hidden polygons included: the
    /// indices components refer to) and components; `None` for anything else
    /// (hidden, not a polygon object).
    #[must_use]
    pub fn polygon(&self, id: NodeId) -> Option<(&EvaluatedNode, &MeshHandle, &Components)> {
        let (mesh, components) = self.polygons.get(&id)?;
        let node = self.nodes.iter().find(|n| n.id == id)?;
        Some((node, mesh, components))
    }
}

/// A polygon object's mesh without its hidden polygons.
#[derive(Debug)]
struct Shown {
    source: MeshId,
    hidden: BitSet,
    mesh: MeshHandle,
}

/// The cached shown mesh of `id`, rebuilt if its mesh or hidden polygons changed.
fn shown_mesh<'a>(shown: &'a mut HashMap<NodeId, Shown>, id: NodeId, mesh: &MeshHandle, hidden: &BitSet) -> &'a Shown {
    let build = || Shown {
        source: mesh.id(),
        hidden: hidden.clone(),
        mesh: MeshHandle::new(mesh.mesh().with_faces_where(|f| !hidden.contains(f))),
    };
    shown
        .entry(id)
        .and_modify(|entry| {
            if entry.source != mesh.id() || entry.hidden != *hidden {
                *entry = build();
            }
        })
        .or_insert_with(build)
}

fn pick_shape(kind: &ObjectKind) -> PickShape {
    match *kind {
        ObjectKind::Null => PickShape::None,
        ObjectKind::Polygon { ref mesh, .. } => PickShape::Mesh(mesh.id()),
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
    use grid4d_scene::{Document, Node};

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
        doc.scene.insert(Node::new("Mesh", polygon_cube()));
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        let capacity = |e: &EvaluatedScene| {
            (
                e.nodes.capacity(),
                e.meshes.capacity(),
                e.polygons.capacity(),
                e.shown.capacity(),
                e.walk_stack.capacity(),
            )
        };
        let before = capacity(&eval);
        let cube = id_of(&doc, "Cube");
        for i in 0..100 {
            doc.scene.get_mut(cube).unwrap().data.transform.position.y = f32::from(u8::try_from(i).unwrap());
            assert!(eval.update(&doc.scene));
        }
        assert_eq!(capacity(&eval), before);
    }

    fn polygon_cube() -> ObjectKind {
        let mesh = crate::editable_mesh(&grid4d_scene::ObjectKindTag::Cube.default_kind()).unwrap();
        ObjectKind::polygon(MeshHandle::new(mesh))
    }

    #[test]
    fn polygon_objects_expose_their_mesh_while_visible() {
        let mut doc = Document::new("meshes");
        let kind = polygon_cube();
        let ObjectKind::Polygon { mesh, .. } = &kind else {
            unreachable!()
        };
        let mesh = mesh.clone();
        let id = doc.scene.insert(Node::new("Mesh", kind));
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        let node = eval.nodes()[0];
        assert_eq!(node.pick, PickShape::Mesh(mesh.id()));
        assert_eq!(node.bounds, mesh.bounds());
        assert_eq!(eval.mesh(mesh.id()), Some(&mesh));
        let (found, found_mesh, components) = eval.polygon(id).unwrap();
        assert_eq!((found.id, found_mesh.id()), (id, mesh.id()));
        assert_eq!(*components, Components::default());

        doc.scene.get_mut(id).unwrap().data.visible_in_editor = false;
        eval.update(&doc.scene);
        assert_eq!(eval.mesh(mesh.id()), None, "hidden objects are not drawn or picked");
        assert!(eval.polygon(id).is_none());
    }

    #[test]
    fn hidden_polygons_are_neither_drawn_nor_picked() {
        let mut doc = Document::new("hidden");
        let kind = polygon_cube();
        let ObjectKind::Polygon { mesh, .. } = &kind else {
            unreachable!()
        };
        let own = mesh.clone();
        let id = doc.scene.insert(Node::new("Mesh", kind));
        let hide = |doc: &mut Document, faces: &[usize]| {
            let ObjectKind::Polygon { components, .. } = &mut doc.scene.get_mut(id).unwrap().data.kind else {
                unreachable!()
            };
            let hidden = &mut components.make_mut().hidden.polygons;
            hidden.clear();
            for &f in faces {
                hidden.insert(f);
            }
        };
        let shown = |eval: &EvaluatedScene| {
            let node = eval.nodes()[0];
            let PickShape::Mesh(pick) = node.pick else {
                panic!("{:?}", node.pick)
            };
            assert_eq!(
                node.drawable.map(|d| d.key),
                Some(MeshKey::Polygon(pick)),
                "drawn as picked"
            );
            eval.mesh(pick).unwrap().clone()
        };
        let mut eval = EvaluatedScene::new();

        hide(&mut doc, &[0, 2]);
        eval.update(&doc.scene);
        let first = shown(&eval);
        assert_ne!(first.id(), own.id());
        assert_eq!(first.mesh().face_count(), 4);
        assert_eq!(first.mesh().face(0), own.mesh().face(1), "the other faces, in order");
        let (_, mesh, _) = eval.polygon(id).unwrap();
        assert_eq!(mesh.id(), own.id(), "components keep the object's own mesh");

        doc.scene.get_mut(id).unwrap().data.transform.position.x = 5.0;
        eval.update(&doc.scene);
        assert_eq!(
            shown(&eval).id(),
            first.id(),
            "built again only when the hidden set changes"
        );

        hide(&mut doc, &[0, 1, 2, 3, 4, 5]);
        eval.update(&doc.scene);
        assert_eq!(eval.nodes()[0].drawable, None, "nothing left to draw");

        hide(&mut doc, &[]);
        eval.update(&doc.scene);
        assert_eq!(shown(&eval).id(), own.id());
        assert!(eval.shown.is_empty(), "the shown mesh is dropped");
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
