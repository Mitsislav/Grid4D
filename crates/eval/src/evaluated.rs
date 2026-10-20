use std::collections::HashMap;

use glam::Mat4;
use grid4d_geom::{BitSet, subdivision::SubdivisionOptions};
use grid4d_scene::{
    Aabb, Components, LIGHT_MARKER_RADIUS, MeshHandle, MeshId, NodeId, ObjectKind, Scene, TagKind, Tags, Tracks,
    WalkItem,
};

use crate::{
    Drawable, MeshKey, Shading, drawable,
    generator::{self, Input, viewport_options},
};

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
/// enabled (a hidden or disabled node hides its whole subtree — except a disabled
/// generator, whose children then show as they are), in Object Manager pre-order,
/// with world matrices.
///
/// A generator draws its result instead of its cage: the cage's nodes stay listed
/// (component modes edit them, framing sees them) but are neither drawn nor picked.
///
/// Animation is applied here, at the frame given to [`EvaluatedScene::set_frame`]:
/// animated objects (and their children) are placed where their curves say. Then
/// a Target tag turns its object's forward axis (−Z) towards the target; the
/// object's children follow. The document's own transforms are left as they are.
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
    /// Each enabled generator's result, rebuilt only when its inputs change.
    generated: HashMap<NodeId, Generated>,
    /// Scratch space for gathering a generator's inputs.
    inputs: Vec<Input>,
    cage_stack: Vec<NodeId>,
    walk_stack: Vec<WalkItem>,
    /// Scratch space for Target tags: node index by id, and the corrections of
    /// turned objects whose subtree is being visited.
    aim_index: HashMap<NodeId, usize>,
    aim_stack: Vec<(u32, Mat4)>,
    /// The frame animation is evaluated at.
    frame: f64,
    /// Whether the last update found animated objects (only then does a new frame
    /// need a new evaluation).
    animated: bool,
    /// Scratch space for animation: the world matrices of the current ancestors.
    world_stack: Vec<(u32, Mat4)>,
    /// [`Scene::revision`] of the scene `nodes` were built from.
    revision: Option<u64>,
}

impl EvaluatedScene {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the frame animation is evaluated at; the next [`update`](Self::update)
    /// re-evaluates if it changed and something is animated.
    pub fn set_frame(&mut self, frame: f64) {
        if self.frame != frame {
            self.frame = frame;
            if self.animated {
                self.revision = None;
            }
        }
    }

    /// The frame animation is evaluated at.
    #[must_use]
    pub const fn frame(&self) -> f64 {
        self.frame
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
        let revision = scene.revision();
        let (nodes, meshes, polygons, shown) = (&mut self.nodes, &mut self.meshes, &mut self.polygons, &mut self.shown);
        let (generated, inputs, cage_stack) = (&mut self.generated, &mut self.inputs, &mut self.cage_stack);
        // The next cage root to consume, and the depth of the cage being consumed.
        let mut next_cage: Option<NodeId> = None;
        let mut cage_depth: Option<u32> = None;
        scene.walk(&mut self.walk_stack, |id, node, world, depth| {
            if cage_depth.is_some_and(|d| depth <= d) {
                cage_depth = None;
            }
            let data = &node.data;
            let generator = viewport_options(&data.kind);
            // A disabled generator is a plain group: its children show as they are.
            if !data.visible_in_editor || (!data.enabled && generator.is_none()) {
                return false;
            }
            if next_cage == Some(id) {
                cage_depth = Some(depth);
                next_cage = None;
            }
            let mut drawable = drawable(&data.kind, data.display_color).map(|d| Drawable {
                shading: Shading::from_angle(data.tags.smoothing_angle()),
                ..d
            });
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
            let mut bounds = data.kind.local_bounds();
            if let Some(options) = generator.filter(|_| data.enabled) {
                if let Some(result) = generated_mesh(generated, inputs, cage_stack, scene, id, options, revision) {
                    let mesh = &result.mesh;
                    if mesh.mesh().face_count() > 0 {
                        meshes.insert(mesh.id(), mesh.clone());
                        bounds = mesh.bounds();
                        pick = PickShape::Mesh(mesh.id());
                        // Its own Phong tag, else the cage's.
                        let cage_tags = scene.children(id).first().and_then(|&c| scene.get(c));
                        let tags = if data.tags.tag_index(TagKind::Phong).is_some() {
                            &data.tags
                        } else {
                            cage_tags.map_or(&data.tags, |cage| &cage.data.tags)
                        };
                        drawable = Some(Drawable {
                            key: MeshKey::Polygon(mesh.id()),
                            shading: Shading::from_angle(tags.smoothing_angle()),
                            scale: glam::Vec3::ONE,
                            color: result.colour,
                            unlit: false,
                            marker: false,
                        });
                    }
                }
                next_cage = scene.children(id).first().copied();
            }
            if cage_depth.is_some() {
                // Part of a cage: the generator shows it.
                drawable = None;
                pick = PickShape::None;
            }
            nodes.push(EvaluatedNode {
                id,
                depth,
                world: *world,
                bounds,
                drawable,
                pick,
            });
            true
        });
        self.animated = self
            .nodes
            .iter()
            .any(|n| scene.get(n.id).is_some_and(|node| !node.data.tracks.is_empty()));
        if self.animated {
            animate(scene, &mut self.nodes, self.frame, &mut self.world_stack);
        }
        if self
            .nodes
            .iter()
            .any(|n| scene.get(n.id).is_some_and(|node| node.data.tags.aim().is_some()))
        {
            apply_aims(scene, &mut self.nodes, &mut self.aim_index, &mut self.aim_stack);
        }
        // Objects gone, hidden or with nothing hidden any more.
        self.shown.retain(|_, entry| self.meshes.contains_key(&entry.mesh.id()));
        self.generated.retain(|_, entry| entry.seen == revision);
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

/// Recomputes the world matrices in pre-order with every object's animated
/// transform at `frame`, so animated objects and everything below them move.
/// (Every listed node's parent is listed before it: a hidden parent hides its
/// subtree.)
fn animate(scene: &Scene, nodes: &mut [EvaluatedNode], frame: f64, stack: &mut Vec<(u32, Mat4)>) {
    stack.clear();
    for node in nodes {
        while stack.last().is_some_and(|&(d, _)| d >= node.depth) {
            stack.pop();
        }
        let Some(data) = scene.get(node.id).map(|n| &n.data) else {
            continue;
        };
        let local = if data.tracks.is_empty() {
            data.transform.matrix()
        } else {
            data.tracks.animate(&data.transform, frame).matrix()
        };
        node.world = stack.last().map_or(local, |&(_, parent)| parent * local);
        stack.push((node.depth, node.world));
    }
}

/// Turns every object with a Target tag towards its target (in pre-order, so a
/// turned parent's children follow it): its forward axis (−Z) faces the target,
/// its up axis leans towards the up object (world up without one); with `pitch`
/// off only the heading changes. Missing or hidden targets, and targets inside the
/// object itself, change nothing.
fn apply_aims(
    scene: &Scene,
    nodes: &mut [EvaluatedNode],
    index: &mut HashMap<NodeId, usize>,
    stack: &mut Vec<(u32, Mat4)>,
) {
    index.clear();
    index.extend(nodes.iter().enumerate().map(|(i, n)| (n.id, i)));
    stack.clear();
    for i in 0..nodes.len() {
        let depth = nodes[i].depth;
        while stack.last().is_some_and(|&(d, _)| d >= depth) {
            stack.pop();
        }
        if let Some(&(_, correction)) = stack.last() {
            nodes[i].world = correction * nodes[i].world;
        }
        let Some(aim) = scene.get(nodes[i].id).and_then(|n| n.data.tags.aim()) else {
            continue;
        };
        let Some(&t) = index.get(&aim.target) else { continue };
        // The object itself, or something inside it, cannot be its target.
        let subtree_end = nodes[i + 1..]
            .iter()
            .position(|n| n.depth <= depth)
            .map_or(nodes.len(), |k| i + 1 + k);
        if (i..subtree_end).contains(&t) {
            continue;
        }
        let old = nodes[i].world;
        let (scale, _, translation) = old.to_scale_rotation_translation();
        let mut forward = nodes[t].world.w_axis.truncate() - translation;
        if !aim.pitch {
            forward.y = 0.0;
        }
        let up = aim
            .up
            .and_then(|u| index.get(&u))
            .map_or(glam::Vec3::Y, |&u| nodes[u].world.w_axis.truncate() - translation);
        let Some(rotation) = look_rotation(forward, up) else {
            continue;
        };
        let new = Mat4::from_scale_rotation_translation(scale, rotation, translation);
        if old.determinant().abs() > f32::EPSILON {
            stack.push((depth, new * old.inverse()));
        }
        nodes[i].world = new;
    }
}

/// The rotation whose −Z axis points along `forward` and whose Y axis leans towards
/// `up`; `None` if `forward` has no length.
fn look_rotation(forward: glam::Vec3, up: glam::Vec3) -> Option<glam::Quat> {
    let back = (-forward).try_normalize()?;
    // An up direction along the line of sight: fall back to another axis.
    let side = up
        .cross(back)
        .try_normalize()
        .or_else(|| glam::Vec3::Z.cross(back).try_normalize())
        .or_else(|| glam::Vec3::X.cross(back).try_normalize())?;
    let true_up = back.cross(side);
    Some(glam::Quat::from_mat3(&glam::Mat3::from_cols(side, true_up, back)))
}

/// A generator's result and what it was built from.
#[derive(Debug)]
struct Generated {
    options: SubdivisionOptions,
    inputs: Vec<Input>,
    colour: [f32; 3],
    mesh: MeshHandle,
    /// The scene revision of the last update that used it.
    seen: u64,
}

/// Generator `id`'s cached result, rebuilt if its options or inputs changed.
/// `None` when it has no cage.
fn generated_mesh<'a>(
    generated: &'a mut HashMap<NodeId, Generated>,
    inputs: &mut Vec<Input>,
    stack: &mut Vec<NodeId>,
    scene: &Scene,
    id: NodeId,
    options: SubdivisionOptions,
    revision: u64,
) -> Option<&'a Generated> {
    let colour = generator::gather(scene, id, stack, inputs)?;
    let fresh = generated
        .get(&id)
        .is_some_and(|entry| entry.options == options && entry.inputs == *inputs);
    if !fresh {
        let mesh = MeshHandle::new(generator::build(inputs, &options));
        let entry = generated.entry(id).or_insert_with(|| Generated {
            options,
            inputs: Vec::new(),
            colour,
            mesh: mesh.clone(),
            seen: revision,
        });
        entry.options = options;
        entry.mesh = mesh;
        // Keep the inputs; the old list (with its capacity) becomes the scratch.
        std::mem::swap(&mut entry.inputs, inputs);
    }
    let entry = generated.get_mut(&id)?;
    entry.colour = colour;
    entry.seen = revision;
    Some(entry)
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
        // Picked by its result (set during evaluation).
        ObjectKind::SubdivisionSurface { .. } => PickShape::None,
    }
}

#[cfg(test)]
mod tests {
    use grid4d_scene::{DEFAULT_PHONG_ANGLE, Document, Node, Tag};

    use super::*;

    fn id_of(doc: &Document, name: &str) -> NodeId {
        doc.scene.iter().find(|(_, n)| n.data.name == name).unwrap().0
    }

    #[test]
    fn animated_objects_and_their_children_move_with_the_frame() {
        use grid4d_scene::{Axis, Curve, Interpolation, Key, Param, Track};
        let mut doc = Document::with_demo_scene();
        let (group, cylinder) = (id_of(&doc, "Group"), id_of(&doc, "Cylinder"));
        let linear = |keys: &[(f64, f32)]| {
            Curve::from_keys(keys.iter().map(|&(f, v)| Key {
                interpolation: Interpolation::Linear,
                ..Key::new(f, v)
            }))
        };
        doc.scene.get_mut(group).unwrap().data.tracks = vec![Track {
            param: Param::Position(Axis::Y),
            curve: linear(&[(0.0, 0.0), (10.0, 300.0)]),
        }];
        let world = |eval: &EvaluatedScene, id| eval.nodes().iter().find(|n| n.id == id).unwrap().world;
        let mut eval = EvaluatedScene::new();
        eval.set_frame(5.0);
        eval.update(&doc.scene);
        assert!(
            (world(&eval, group).w_axis.y - 150.0).abs() < 1e-3,
            "halfway up at frame 5"
        );
        let child = world(&eval, cylinder).w_axis.y;
        assert!((child - (150.0 + 100.0)).abs() < 1e-3, "the child follows: {child}");
        assert_eq!(
            doc.scene.get(group).unwrap().data.transform.position.y,
            0.0,
            "the document is untouched"
        );

        eval.set_frame(10.0);
        assert!(eval.update(&doc.scene), "a new frame re-evaluates an animated scene");
        assert!((world(&eval, group).w_axis.y - 300.0).abs() < 1e-3);
        eval.set_frame(10.0);
        assert!(!eval.update(&doc.scene), "the same frame does not");

        // Without animation a new frame changes nothing and costs nothing.
        doc.scene.get_mut(group).unwrap().data.tracks.clear();
        eval.update(&doc.scene);
        eval.set_frame(42.0);
        assert!(!eval.update(&doc.scene));
    }

    #[test]
    fn a_phong_tag_sets_the_shading_of_its_object_only() {
        let mut doc = Document::with_demo_scene();
        let (cube, sphere) = (id_of(&doc, "Cube"), id_of(&doc, "Sphere"));
        let mut eval = EvaluatedScene::new();
        let shading = |eval: &EvaluatedScene, id| {
            eval.nodes()
                .iter()
                .find(|n| n.id == id)
                .unwrap()
                .drawable
                .unwrap()
                .shading
        };
        eval.update(&doc.scene);
        assert_eq!(
            shading(&eval, cube),
            Shading::from_angle(DEFAULT_PHONG_ANGLE),
            "no tag: the default"
        );

        doc.scene.get_mut(cube).unwrap().data.tags = vec![Tag::Phong {
            angle_limit: true,
            angle: 0.0,
        }];
        doc.scene.get_mut(sphere).unwrap().data.tags = vec![Tag::Phong {
            angle_limit: false,
            angle: 0.2,
        }];
        eval.update(&doc.scene);
        assert_eq!(shading(&eval, cube), Shading::from_angle(0.0), "flat");
        assert_eq!(
            shading(&eval, sphere),
            Shading::from_angle(std::f32::consts::PI),
            "no limit: all smooth"
        );
        assert_eq!(
            shading(&eval, id_of(&doc, "Box")),
            Shading::default(),
            "others keep the default"
        );
    }

    #[test]
    fn a_target_tag_turns_the_object_and_its_children_follow() {
        let mut doc = Document::new("aim");
        let target = doc.scene.insert(Node::new("Target", ObjectKind::Null));
        let camera = doc.scene.insert(
            Node::new("Cam", ObjectKind::Camera { fov_y_deg: 40.0 }).with_position(glam::Vec3::new(300.0, 200.0, 0.0)),
        );
        let child = doc
            .scene
            .insert_child(
                camera,
                Node::new("Child", ObjectKind::Null).with_position(glam::Vec3::new(0.0, 0.0, -100.0)),
            )
            .unwrap();
        let world = |eval: &EvaluatedScene, id| eval.nodes().iter().find(|n| n.id == id).unwrap().world;
        let aim = |doc: &mut Document, target: Option<NodeId>, pitch: bool| {
            doc.scene.get_mut(camera).unwrap().data.tags = vec![Tag::Target {
                target,
                up: None,
                pitch,
            }];
        };
        let mut eval = EvaluatedScene::new();

        aim(&mut doc, Some(target), true);
        eval.update(&doc.scene);
        let cam = world(&eval, camera);
        let forward = cam.transform_vector3(-glam::Vec3::Z).normalize();
        let expected = (glam::Vec3::ZERO - glam::Vec3::new(300.0, 200.0, 0.0)).normalize();
        assert!(forward.abs_diff_eq(expected, 1e-5), "−Z faces the target: {forward}");
        assert!(cam.transform_vector3(glam::Vec3::Y).y > 0.0, "and stays upright");
        assert!(
            cam.w_axis
                .truncate()
                .abs_diff_eq(glam::Vec3::new(300.0, 200.0, 0.0), 1e-4),
            "it only turns"
        );
        // The child sits 100 cm in front of the camera, along its line of sight.
        let ahead = world(&eval, child).w_axis.truncate();
        assert!(
            ahead.abs_diff_eq(glam::Vec3::new(300.0, 200.0, 0.0) + expected * 100.0, 1e-3),
            "{ahead}"
        );
        assert_eq!(
            doc.scene.get(camera).unwrap().data.transform.rotation,
            glam::Vec3::ZERO,
            "the document's rotation is untouched"
        );

        aim(&mut doc, Some(target), false);
        eval.update(&doc.scene);
        let level = world(&eval, camera).transform_vector3(-glam::Vec3::Z).normalize();
        assert!(
            level.abs_diff_eq(glam::Vec3::new(-1.0, 0.0, 0.0), 1e-5),
            "Pitch off: level, {level}"
        );

        for ignored in [Some(camera), Some(child), None] {
            aim(&mut doc, ignored, true);
            eval.update(&doc.scene);
            assert_eq!(
                world(&eval, camera),
                doc.scene.world_matrix(camera),
                "{ignored:?} changes nothing"
            );
        }
        aim(&mut doc, Some(target), true);
        doc.scene.get_mut(target).unwrap().data.visible_in_editor = false;
        eval.update(&doc.scene);
        assert_eq!(
            world(&eval, camera),
            doc.scene.world_matrix(camera),
            "a hidden target changes nothing"
        );
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

    /// A document with a Subdivision Surface whose cage is a polygon cube, plus a
    /// second child (a sphere). Returns (doc, generator, cage, sphere).
    fn generator_scene() -> (Document, NodeId, NodeId, NodeId) {
        let mut doc = Document::new("sds");
        let sds = doc.scene.insert(Node::new(
            "Subdivision Surface",
            grid4d_scene::ObjectKindTag::SubdivisionSurface.default_kind(),
        ));
        let cage = doc.scene.insert_child(sds, Node::new("Cage", polygon_cube())).unwrap();
        let sphere = doc
            .scene
            .insert_child(
                sds,
                Node::new("Sphere", grid4d_scene::ObjectKindTag::Sphere.default_kind()),
            )
            .unwrap();
        (doc, sds, cage, sphere)
    }

    fn node(eval: &EvaluatedScene, id: NodeId) -> &EvaluatedNode {
        eval.nodes().iter().find(|n| n.id == id).unwrap()
    }

    fn result_of(eval: &EvaluatedScene, id: NodeId) -> MeshHandle {
        let PickShape::Mesh(mesh) = node(eval, id).pick else {
            panic!("no result")
        };
        eval.mesh(mesh).unwrap().clone()
    }

    #[test]
    fn a_generator_shows_its_cage_smoothed_and_hides_the_cage() {
        let (doc, sds, cage, sphere) = generator_scene();
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        let result = result_of(&eval, sds);
        assert_eq!(result.mesh().face_count(), 6 * 4 * 4, "two levels on a cube");
        assert_eq!(
            node(&eval, sds).drawable.map(|d| d.key),
            Some(MeshKey::Polygon(result.id()))
        );
        assert_eq!(node(&eval, sds).bounds, result.bounds());
        let cage_node = node(&eval, cage);
        assert_eq!(
            (cage_node.drawable, cage_node.pick),
            (None, PickShape::None),
            "the cage is not drawn"
        );
        assert!(cage_node.bounds.is_some(), "but can still be framed");
        assert!(eval.polygon(cage).is_some(), "and edited in component modes");
        assert!(
            node(&eval, sphere).drawable.is_some(),
            "a second child is drawn as it is"
        );
        assert_eq!(crate::generator_mesh(&doc.scene, sds).unwrap(), *result.mesh());
    }

    #[test]
    fn a_generator_rebuilds_only_when_its_inputs_change() {
        let (mut doc, sds, cage, _) = generator_scene();
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        let first = result_of(&eval, sds).id();

        doc.scene.get_mut(sds).unwrap().data.transform.position.x = 50.0;
        eval.update(&doc.scene);
        assert_eq!(
            result_of(&eval, sds).id(),
            first,
            "moving the generator changes nothing"
        );
        let ObjectKind::Polygon { components, .. } = &mut doc.scene.get_mut(cage).unwrap().data.kind else {
            unreachable!()
        };
        components.make_mut().selected.points.insert(0);
        eval.update(&doc.scene);
        assert_eq!(
            result_of(&eval, sds).id(),
            first,
            "selecting cage points changes nothing"
        );

        doc.scene.get_mut(cage).unwrap().data.transform.position.y = 10.0;
        eval.update(&doc.scene);
        let moved = result_of(&eval, sds);
        assert_ne!(moved.id(), first, "moving the cage does");
        assert!(
            moved.bounds().unwrap().min.y > -100.0 + 9.0,
            "the result follows the cage"
        );

        let ObjectKind::SubdivisionSurface { viewport_levels, .. } = &mut doc.scene.get_mut(sds).unwrap().data.kind
        else {
            unreachable!()
        };
        *viewport_levels = 1;
        eval.update(&doc.scene);
        assert_eq!(result_of(&eval, sds).mesh().face_count(), 24, "fewer levels");
    }

    #[test]
    fn a_disabled_generator_shows_its_cage_and_a_hidden_one_hides_everything() {
        let (mut doc, sds, cage, _) = generator_scene();
        let mut eval = EvaluatedScene::new();
        doc.scene.get_mut(sds).unwrap().data.enabled = false;
        eval.update(&doc.scene);
        assert_eq!(node(&eval, sds).drawable, None);
        assert!(
            node(&eval, cage).drawable.is_some(),
            "the cage is drawn as a normal object"
        );

        doc.scene.get_mut(sds).unwrap().data.enabled = true;
        doc.scene.get_mut(sds).unwrap().data.visible_in_editor = false;
        eval.update(&doc.scene);
        assert!(eval.nodes().is_empty(), "nothing of it is visible");
    }

    #[test]
    fn a_cage_merges_its_own_children_and_primitives_work() {
        let (mut doc, sds, cage, _) = generator_scene();
        let child = doc
            .scene
            .insert_child(cage, Node::new("Child", polygon_cube()))
            .unwrap();
        doc.scene.get_mut(child).unwrap().data.transform.position.x = 500.0;
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        assert_eq!(
            result_of(&eval, sds).mesh().face_count(),
            2 * 96,
            "both cubes, smoothed as one mesh"
        );
        assert_eq!(
            node(&eval, child).drawable,
            None,
            "the cage's child is part of the cage"
        );

        doc.scene.get_mut(cage).unwrap().data.kind = grid4d_scene::ObjectKindTag::Cube.default_kind();
        eval.update(&doc.scene);
        assert_eq!(
            result_of(&eval, sds).mesh().face_count(),
            2 * 96,
            "a primitive cage too"
        );
    }

    #[test]
    fn steady_state_updates_with_a_generator_reuse_capacity() {
        let (mut doc, sds, _, sphere) = generator_scene();
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        let first = result_of(&eval, sds).id();
        let capacity = |e: &EvaluatedScene| (e.generated.capacity(), e.inputs.capacity(), e.cage_stack.capacity());
        // One more update fills the scratch list the first build handed over.
        doc.scene.get_mut(sphere).unwrap().data.transform.position.z = 1.0;
        eval.update(&doc.scene);
        let before = capacity(&eval);
        for i in 0..50 {
            doc.scene.get_mut(sphere).unwrap().data.transform.position.x = f32::from(u8::try_from(i).unwrap());
            eval.update(&doc.scene);
        }
        assert_eq!(capacity(&eval), before);
        assert_eq!(result_of(&eval, sds).id(), first);
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
