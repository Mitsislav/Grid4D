use std::collections::HashMap;

use glam::{Mat4, Vec3};
use grid4d_geom::{BitSet, spline::Polyline};
use grid4d_scene::{
    Aabb, Components, LIGHT_MARKER_RADIUS, LightDetails, LightShadow, LightType, MaterialId, MeshHandle, MeshId,
    NodeId, ObjectKind, ObjectKindTag, Scene, SplineHandle, TagKind, Tags, WalkItem,
};

use crate::{
    CurveHandle, CurveId, Drawable, MeshKey, Shading, Surface,
    curve::{CachedCurve, CurveSource},
    deform::{self, Active, Base, Deformed, Link},
    drawable,
    generator::{self, Input, Options},
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
    /// A spline's curve: picked a few pixels around its line (see
    /// [`EvaluatedScene::curve`]).
    Curve(CurveId),
}

/// One visible node, as viewers see it this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EvaluatedNode {
    pub id: NodeId,
    /// What kind of object it is (views filter by it).
    pub kind: ObjectKindTag,
    /// Hierarchy depth (0 for top-level objects).
    pub depth: u32,
    pub world: Mat4,
    /// Object-space bounds (framing, picking, selection brackets); `None` for Nulls.
    pub bounds: Option<Aabb>,
    /// What to draw; `None` for objects without geometry.
    pub drawable: Option<Drawable>,
    pub pick: PickShape,
}

/// What an evaluation is for: what is visible, and at what detail.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Purpose {
    /// The viewport: objects visible in the editor, generators at their viewport
    /// levels.
    #[default]
    Editor,
    /// The final render: objects visible in the renderer, generators at their
    /// render levels.
    Render,
}

impl Purpose {
    /// Whether `data`'s object (and so its subtree) is visible for this purpose.
    #[must_use]
    pub const fn shows(self, data: &grid4d_scene::NodeData) -> bool {
        match self {
            Self::Editor => data.visible_in_editor,
            Self::Render => data.visible_in_render,
        }
    }
}

/// A light object as viewers see it this frame (animation and Target tags applied).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EvaluatedLight {
    pub id: NodeId,
    pub light_type: LightType,
    /// World position.
    pub position: Vec3,
    /// The direction it shines in (its −Z axis), unit length.
    pub direction: Vec3,
    /// Colour × intensity, linear RGB.
    pub color: [f32; 3],
    pub details: LightDetails,
    pub shadow: LightShadow,
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
    /// The visible light objects, in pre-order.
    lights: Vec<EvaluatedLight>,
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
    /// The visible splines' sampled curves, by id (drawing and picking).
    curves: HashMap<CurveId, CurveHandle>,
    /// Each spline's curve, sampled again only when the spline changes.
    curve_cache: HashMap<NodeId, CachedCurve>,
    /// The visible editable splines and their components (Points mode).
    splines: HashMap<NodeId, (SplineHandle, Components)>,
    /// Each deformed object's mesh, rebuilt only when its source or chain changes.
    deformed: HashMap<NodeId, Deformed>,
    /// Scratch space for deformers: those reaching the node being visited, and
    /// its chain.
    active: Vec<Active>,
    links: Vec<Link>,
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
    /// Scratch space for materials: the material each current ancestor shows
    /// (its own Material tag's, else inherited).
    material_stack: Vec<(u32, Option<MaterialId>)>,
    /// [`Scene::revision`] of the scene `nodes` were built from.
    revision: Option<u64>,
    /// The viewport, or the final render.
    purpose: Purpose,
}

impl EvaluatedScene {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// An evaluation for the final render: objects visible in the renderer,
    /// generators at their render levels.
    #[must_use]
    pub fn for_render() -> Self {
        Self {
            purpose: Purpose::Render,
            ..Self::default()
        }
    }

    #[must_use]
    pub const fn purpose(&self) -> Purpose {
        self.purpose
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
        self.curves.clear();
        self.splines.clear();
        let revision = scene.revision();
        let (purpose, frame) = (self.purpose, self.frame);
        let (nodes, meshes, polygons, shown) = (&mut self.nodes, &mut self.meshes, &mut self.polygons, &mut self.shown);
        let (curves, curve_cache, splines) = (&mut self.curves, &mut self.curve_cache, &mut self.splines);
        let (generated, inputs, cage_stack) = (&mut self.generated, &mut self.inputs, &mut self.cage_stack);
        let material_stack = &mut self.material_stack;
        material_stack.clear();
        let (deformed, active, links) = (&mut self.deformed, &mut self.active, &mut self.links);
        active.clear();
        let materials = scene.materials();
        // The next cage root to consume, and the depth of the cage being consumed.
        let mut next_cage: Option<NodeId> = None;
        let mut cage_depth: Option<u32> = None;
        scene.walk(&mut self.walk_stack, |id, node, world, depth| {
            if cage_depth.is_some_and(|d| depth <= d) {
                cage_depth = None;
            }
            deform::leave(active, depth);
            let data = &node.data;
            // Its own parameters at the frame (borrowed unless animated).
            let kind = data.kind_at(frame);
            let generator = generator::options(&kind, purpose);
            // A disabled generator is a plain group: its children show as they are.
            if !purpose.shows(data) || (!data.enabled && generator.is_none()) {
                return false;
            }
            if next_cage == Some(id) {
                cage_depth = Some(depth);
                next_cage = None;
            }
            // Its deformer children reach it and everything below it.
            deform::push_children(scene, id, depth, frame, active);
            // Its Material tag's material, else the nearest ancestor's.
            while material_stack.last().is_some_and(|&(d, _)| d >= depth) {
                material_stack.pop();
            }
            let inherited = material_stack.last().and_then(|&(_, m)| m);
            let material = data.tags.material().filter(|&m| materials.contains(m)).or(inherited);
            material_stack.push((depth, material));
            // Markers (cameras, lights) keep their own colour and surface.
            let own_material = material.and_then(|m| materials.get(m));
            let mut drawable = drawable(&kind, data.display_color).map(|d| {
                let material = own_material.filter(|_| !d.marker);
                Drawable {
                    shading: Shading::from_angle(data.tags.smoothing_angle()),
                    color: material.map_or(d.color, |m| m.color),
                    surface: material.map_or(d.surface, Surface::of),
                    ..d
                }
            });
            let mut pick = pick_shape(&kind);
            // The surface deformers would reshape: a polygon object's shown
            // mesh or a generator's result (primitives are made on demand).
            let mut surface: Option<MeshHandle> = None;
            if let ObjectKind::Polygon { mesh, components } = &*kind {
                polygons.insert(id, (mesh.clone(), components.clone()));
                let hidden = &components.hidden().polygons;
                let mesh = if hidden.is_empty() {
                    mesh
                } else {
                    &shown_mesh(shown, id, mesh, hidden).mesh
                };
                meshes.insert(mesh.id(), mesh.clone());
                pick = PickShape::Mesh(mesh.id());
                surface = Some(mesh.clone());
                drawable = drawable.filter(|_| mesh.mesh().face_count() > 0).map(|d| Drawable {
                    key: MeshKey::Polygon(mesh.id()),
                    ..d
                });
            }
            let mut bounds_of_curve = None;
            let source = match &*kind {
                ObjectKind::SplinePrimitive(primitive) => Some(CurveSource::Primitive(*primitive)),
                ObjectKind::Text(text) => Some(CurveSource::Text(text.clone())),
                ObjectKind::Spline {
                    spline,
                    interpolation,
                    components,
                } => {
                    splines.insert(id, (spline.clone(), components.clone()));
                    Some(CurveSource::Editable(spline.id(), *interpolation))
                }
                ObjectKind::Ffd {
                    lattice, components, ..
                } => {
                    // Its lattice is edited in Points mode like a spline's points.
                    splines.insert(id, (lattice.clone(), components.clone()));
                    deform::Deformer::of(&kind).map(CurveSource::Deformer)
                }
                kind if kind.is_deformer() => deform::Deformer::of(kind).map(CurveSource::Deformer),
                _ => None,
            };
            if let Some(source) = source {
                let sample = || curve_of(&kind);
                let cached = match curve_cache.entry(id) {
                    std::collections::hash_map::Entry::Occupied(entry) => {
                        let cached = entry.into_mut();
                        cached.refresh(source, sample, revision);
                        cached
                    }
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert(CachedCurve::new(source, sample(), revision))
                    }
                };
                let handle = cached.handle.clone();
                pick = PickShape::Curve(handle.id());
                // The cached curve's extent (a text's needs its font).
                bounds_of_curve = handle.polyline().bounds().map(|(min, max)| Aabb {
                    min: min - Vec3::splat(0.5),
                    max: max + Vec3::splat(0.5),
                });
                // Lines in the display colour: a curve has no surface or material.
                drawable = (!handle.polyline().points.is_empty()).then_some(Drawable {
                    key: MeshKey::Curve(handle.id()),
                    shading: Shading::default(),
                    scale: Vec3::ONE,
                    color: data.display_color,
                    surface: Surface::default(),
                    unlit: true,
                    marker: false,
                });
                curves.insert(handle.id(), handle);
            }
            let mut bounds = if kind.is_spline() || kind.is_deformer() {
                bounds_of_curve
            } else {
                kind.local_bounds()
            };
            if let Some(options) = generator.filter(|_| data.enabled) {
                if let Some(result) = generated_mesh(
                    generated, inputs, cage_stack, scene, id, options, revision, purpose, frame,
                ) {
                    let mesh = &result.mesh;
                    if mesh.mesh().face_count() > 0 {
                        surface = Some(mesh.clone());
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
                        // Its material (own or inherited), else the cage's, else the cage's colour.
                        let cage_material = cage_tags
                            .and_then(|cage| cage.data.tags.material())
                            .filter(|&m| materials.contains(m));
                        let shown = material.or(cage_material).and_then(|m| materials.get(m));
                        drawable = Some(Drawable {
                            key: MeshKey::Polygon(mesh.id()),
                            shading: Shading::from_angle(tags.smoothing_angle()),
                            scale: Vec3::ONE,
                            color: shown.map_or(result.colour, |m| m.color),
                            surface: shown.map_or_else(Surface::default, Surface::of),
                            unlit: false,
                            marker: false,
                        });
                    }
                }
                // Only a Subdivision Surface hides its cage: the spline
                // generators' inputs are curves, drawn anyway, and anything
                // else under them is not used and stays as it is.
                if matches!(options, Options::Subdivision(_)) {
                    next_cage = scene.children(id).first().copied();
                }
            }
            // Deformers reaching it reshape its surface (not a cage's: the
            // generator takes those, nor a deformer's or a curve's).
            if cage_depth.is_none()
                && !active.is_empty()
                && let Some(d) = drawable.filter(|d| !d.unlit && !d.marker)
            {
                let base = match surface.take() {
                    Some(mesh) => Some(Base::Mesh(mesh)),
                    None if !matches!(d.key, MeshKey::Polygon(_)) => Some(Base::Primitive(kind.clone().into_owned())),
                    None => None,
                };
                if let Some(base) = base {
                    deform::chain(active, scene.world_matrix_at(id, frame), links);
                    if let Some(entry) = deform::deformed(deformed, id, base, links, revision) {
                        let mesh = &entry.mesh;
                        meshes.insert(mesh.id(), mesh.clone());
                        pick = PickShape::Mesh(mesh.id());
                        bounds = mesh.bounds();
                        drawable = Some(Drawable {
                            key: MeshKey::Polygon(mesh.id()),
                            scale: Vec3::ONE,
                            ..d
                        });
                    }
                }
            }
            if cage_depth.is_some() && !kind.is_spline() {
                // Part of a cage: the generator shows it. (Splines stay drawn:
                // curves are guides, and an Extrude's input is edited as one.)
                drawable = None;
                pick = PickShape::None;
            }
            nodes.push(EvaluatedNode {
                id,
                kind: kind.tag(),
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
        // The lights where animation and Target tags put them.
        self.lights.clear();
        for node in &self.nodes {
            let Some(kind) = scene.get(node.id).map(|n| n.data.kind_at(self.frame)) else {
                continue;
            };
            let ObjectKind::Light {
                color,
                intensity,
                light_type,
                details,
                shadow,
            } = *kind
            else {
                continue;
            };
            self.lights.push(EvaluatedLight {
                id: node.id,
                light_type,
                position: node.world.w_axis.truncate(),
                direction: node.world.transform_vector3(Vec3::NEG_Z).normalize_or(Vec3::NEG_Z),
                color: color.map(|c| c * intensity),
                details,
                shadow,
            });
        }
        // Objects gone, hidden or with nothing hidden any more.
        self.shown.retain(|_, entry| self.meshes.contains_key(&entry.mesh.id()));
        self.generated.retain(|_, entry| entry.seen == revision);
        self.deformed.retain(|_, entry| entry.seen == revision);
        self.curve_cache.retain(|_, entry| entry.seen == revision);
        self.revision = Some(scene.revision());
        true
    }

    /// Visible nodes in Object Manager pre-order.
    #[must_use]
    pub fn nodes(&self) -> &[EvaluatedNode] {
        &self.nodes
    }

    /// The visible light objects, in Object Manager pre-order.
    #[must_use]
    pub fn lights(&self) -> &[EvaluatedLight] {
        &self.lights
    }

    /// The mesh behind a [`MeshKey::Polygon`](crate::MeshKey::Polygon) or
    /// [`PickShape::Mesh`], if a visible object uses it.
    #[must_use]
    pub fn mesh(&self, id: MeshId) -> Option<&MeshHandle> {
        self.meshes.get(&id)
    }

    /// A visible editable spline's node, curve and components; `None` for
    /// anything else (hidden, not an editable spline).
    #[must_use]
    pub fn spline(&self, id: NodeId) -> Option<(&EvaluatedNode, &SplineHandle, &Components)> {
        let (spline, components) = self.splines.get(&id)?;
        let node = self.nodes.iter().find(|n| n.id == id)?;
        Some((node, spline, components))
    }

    /// A visible spline's sampled curve, by the id its drawable and pick shape
    /// carry.
    #[must_use]
    pub fn curve(&self, id: CurveId) -> Option<&CurveHandle> {
        self.curves.get(&id)
    }

    /// The mesh deformers made of `id` in the last update (its shape as shown);
    /// `None` when no deformer reaches it, or it is hidden.
    #[must_use]
    pub fn deformed(&self, id: NodeId) -> Option<&MeshHandle> {
        self.deformed
            .get(&id)
            .map(|entry| &entry.mesh)
            .filter(|mesh| self.meshes.contains_key(&mesh.id()))
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
        // Temporary values made at this frame included.
        let local = data.transform_at(frame).matrix();
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
            .map_or(Vec3::Y, |&u| nodes[u].world.w_axis.truncate() - translation);
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
fn look_rotation(forward: Vec3, up: Vec3) -> Option<glam::Quat> {
    let back = (-forward).try_normalize()?;
    // An up direction along the line of sight: fall back to another axis.
    let side = up
        .cross(back)
        .try_normalize()
        .or_else(|| Vec3::Z.cross(back).try_normalize())
        .or_else(|| Vec3::X.cross(back).try_normalize())?;
    let true_up = back.cross(side);
    Some(glam::Quat::from_mat3(&glam::Mat3::from_cols(side, true_up, back)))
}

/// A generator's result and what it was built from.
#[derive(Debug)]
struct Generated {
    options: Options,
    inputs: Vec<Input>,
    colour: [f32; 3],
    mesh: MeshHandle,
    /// The scene revision of the last update that used it.
    seen: u64,
}

/// Generator `id`'s cached result, rebuilt if its options or inputs changed.
/// `None` when it has no cage.
#[expect(clippy::too_many_arguments, reason = "the evaluation's scratch space, passed apart")]
fn generated_mesh<'a>(
    generated: &'a mut HashMap<NodeId, Generated>,
    inputs: &mut Vec<Input>,
    stack: &mut Vec<NodeId>,
    scene: &Scene,
    id: NodeId,
    options: Options,
    revision: u64,
    purpose: Purpose,
    frame: f64,
) -> Option<&'a Generated> {
    let colour = generator::gather(scene, id, &options, purpose, frame, stack, inputs)?;
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

/// A spline's curve cut into pieces; empty for other kinds.
fn curve_of(kind: &ObjectKind) -> Polyline {
    match kind {
        ObjectKind::SplinePrimitive(primitive) => primitive.polyline(),
        ObjectKind::Text(text) => grid4d_text::text_spline(text).sample(text.interpolation),
        kind if kind.is_deformer() => deform::Deformer::of(kind).map_or_else(Polyline::default, |d| d.box_lines()),
        ObjectKind::Spline {
            spline, interpolation, ..
        } => spline.spline().sample(*interpolation),
        _ => Polyline::default(),
    }
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
        // Picked by its result, or its curve (set during evaluation).
        ObjectKind::SubdivisionSurface { .. }
        | ObjectKind::Extrude { .. }
        | ObjectKind::Lathe(_)
        | ObjectKind::Sweep(_)
        | ObjectKind::Loft(_)
        | ObjectKind::Bend(_)
        | ObjectKind::Twist(_)
        | ObjectKind::Taper(_)
        | ObjectKind::Bulge(_)
        | ObjectKind::Ffd { .. }
        | ObjectKind::SplinePrimitive(_)
        | ObjectKind::Text(_)
        | ObjectKind::Spline { .. } => PickShape::None,
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
    fn materials_colour_objects_and_their_children() {
        use grid4d_scene::{Material, Tag};
        let mut doc = Document::with_demo_scene();
        let (group, cylinder, light) = (id_of(&doc, "Group"), id_of(&doc, "Cylinder"), id_of(&doc, "Light"));
        let mut red = Material::new("Red");
        red.color = [1.0, 0.0, 0.0];
        let red = doc.scene.materials_mut().insert(red);
        for id in [group, light] {
            doc.scene
                .get_mut(id)
                .unwrap()
                .data
                .tags
                .push(Tag::Material { material: Some(red) });
        }
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        let colour = |eval: &EvaluatedScene, id| {
            eval.nodes()
                .iter()
                .find(|n| n.id == id)
                .and_then(|n| n.drawable)
                .map(|d| d.color)
        };
        assert_eq!(
            colour(&eval, cylinder),
            Some([1.0, 0.0, 0.0]),
            "inherited from the group"
        );
        assert_ne!(colour(&eval, light), Some([1.0, 0.0, 0.0]), "markers keep their colour");
        let cube = id_of(&doc, "Cube");
        let cube_colour = doc.scene.get(cube).unwrap().data.display_color;
        assert_eq!(
            colour(&eval, cube),
            Some(cube_colour),
            "no material: the display colour"
        );

        // Editing the material re-evaluates.
        doc.scene.materials_mut().get_mut(red).unwrap().color = [0.0, 1.0, 0.0];
        assert!(eval.update(&doc.scene));
        assert_eq!(colour(&eval, cylinder), Some([0.0, 1.0, 0.0]));
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
            Node::new("Cam", ObjectKind::Camera { fov_y_deg: 40.0 }).with_position(Vec3::new(300.0, 200.0, 0.0)),
        );
        let child = doc
            .scene
            .insert_child(
                camera,
                Node::new("Child", ObjectKind::Null).with_position(Vec3::new(0.0, 0.0, -100.0)),
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
        let forward = cam.transform_vector3(-Vec3::Z).normalize();
        let expected = (Vec3::ZERO - Vec3::new(300.0, 200.0, 0.0)).normalize();
        assert!(forward.abs_diff_eq(expected, 1e-5), "−Z faces the target: {forward}");
        assert!(cam.transform_vector3(Vec3::Y).y > 0.0, "and stays upright");
        assert!(
            cam.w_axis.truncate().abs_diff_eq(Vec3::new(300.0, 200.0, 0.0), 1e-4),
            "it only turns"
        );
        // The child sits 100 cm in front of the camera, along its line of sight.
        let ahead = world(&eval, child).w_axis.truncate();
        assert!(
            ahead.abs_diff_eq(Vec3::new(300.0, 200.0, 0.0) + expected * 100.0, 1e-3),
            "{ahead}"
        );
        assert_eq!(
            doc.scene.get(camera).unwrap().data.transform.rotation,
            Vec3::ZERO,
            "the document's rotation is untouched"
        );

        aim(&mut doc, Some(target), false);
        eval.update(&doc.scene);
        let level = world(&eval, camera).transform_vector3(-Vec3::Z).normalize();
        assert!(
            level.abs_diff_eq(Vec3::new(-1.0, 0.0, 0.0), 1e-5),
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
    fn visible_lights_are_listed_where_they_shine() {
        use grid4d_scene::Transform;
        let mut doc = Document::new("lights");
        let target = doc
            .scene
            .insert(Node::new("Target", ObjectKind::Null).with_position(Vec3::new(300.0, 0.0, 0.0)));
        let mut spot = Node::new(
            "Spot",
            ObjectKind::Light {
                color: [1.0, 0.5, 0.0],
                intensity: 2.0,
                light_type: LightType::Spot,
                details: LightDetails::default(),
                shadow: LightShadow::None,
            },
        )
        .with_transform(Transform::from_position(Vec3::new(0.0, 300.0, 0.0)));
        spot.data.tags = vec![Tag::Target {
            target: Some(target),
            up: None,
            pitch: true,
        }];
        let spot = doc.scene.insert(spot);
        let mut off = Node::new("Off", ObjectKind::light([1.0; 3], 1.0));
        off.data.visible_in_editor = false;
        doc.scene.insert(off);
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        let [light] = eval.lights() else {
            panic!("one visible light: {:?}", eval.lights())
        };
        assert_eq!((light.id, light.light_type), (spot, LightType::Spot));
        assert!(light.position.abs_diff_eq(Vec3::new(0.0, 300.0, 0.0), 1e-4));
        let towards_target = (Vec3::new(300.0, 0.0, 0.0) - light.position).normalize();
        assert!(
            light.direction.abs_diff_eq(towards_target, 1e-4),
            "the Target tag aims it"
        );
        assert_eq!(light.color, [2.0, 1.0, 0.0], "colour × intensity");
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
        let mesh = crate::editable_mesh(&ObjectKindTag::Cube.default_kind()).unwrap();
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
            ObjectKindTag::SubdivisionSurface.default_kind(),
        ));
        let cage = doc.scene.insert_child(sds, Node::new("Cage", polygon_cube())).unwrap();
        let sphere = doc
            .scene
            .insert_child(sds, Node::new("Sphere", ObjectKindTag::Sphere.default_kind()))
            .unwrap();
        (doc, sds, cage, sphere)
    }

    #[test]
    fn a_render_evaluation_uses_render_visibility_and_levels() {
        let (mut doc, sds, _, sphere) = generator_scene();
        // Default levels: 2 in the viewport, 3 for the renderer.
        let mut editor = EvaluatedScene::new();
        editor.update(&doc.scene);
        let mut render = EvaluatedScene::for_render();
        render.update(&doc.scene);
        assert_eq!(render.purpose(), Purpose::Render);
        let faces = |eval: &EvaluatedScene| result_of(eval, sds).mesh().face_count();
        assert_eq!(faces(&render), faces(&editor) * 4, "one more level");

        // Hidden in the renderer only: gone from the render, still in the editor.
        doc.scene.get_mut(sphere).unwrap().data.visible_in_render = false;
        let mut cube = Node::new("Cube", ObjectKindTag::Cube.default_kind());
        cube.data.visible_in_render = false;
        let cube = doc.scene.insert(cube);
        editor.update(&doc.scene);
        render.update(&doc.scene);
        for id in [cube, sphere] {
            assert!(editor.nodes().iter().any(|n| n.id == id));
            assert!(!render.nodes().iter().any(|n| n.id == id));
        }
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
        assert_eq!(crate::generator_mesh(&doc.scene, sds, 0.0).unwrap(), *result.mesh());
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

        doc.scene.get_mut(cage).unwrap().data.kind = ObjectKindTag::Cube.default_kind();
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

    #[test]
    fn splines_are_curves_sampled_once_per_change() {
        let mut doc = Document::new("t");
        let circle = doc
            .scene
            .insert(Node::new("Circle", ObjectKindTag::Circle.default_kind()));
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        let node = eval.nodes()[0];
        let Some(Drawable {
            key: MeshKey::Curve(id),
            unlit: true,
            ..
        }) = node.drawable
        else {
            panic!("a curve drawable: {:?}", node.drawable)
        };
        assert_eq!(node.pick, PickShape::Curve(id));
        assert_eq!(node.kind, ObjectKindTag::Circle);
        let points = eval.curve(id).unwrap().polyline().points.len();
        assert!(points >= 72, "adaptive at 5°: {points}");

        // A move keeps the curve; a new radius samples it again.
        doc.scene.get_mut(circle).unwrap().data.transform.position.x = 50.0;
        eval.update(&doc.scene);
        assert_eq!(eval.nodes()[0].pick, PickShape::Curve(id), "the same curve");
        if let ObjectKind::SplinePrimitive(p) = &mut doc.scene.get_mut(circle).unwrap().data.kind {
            p.shape = grid4d_geom::spline::SplineShape::Circle {
                radius: 10.0,
                ellipse: None,
                ring: None,
            };
        }
        eval.update(&doc.scene);
        let PickShape::Curve(new) = eval.nodes()[0].pick else {
            panic!()
        };
        assert_ne!(new, id, "a changed spline is a new curve");
        assert!(eval.curve(id).is_none(), "the old one is gone");
        let (min, max) = eval.curve(new).unwrap().polyline().bounds().unwrap();
        assert!((max.x - 10.0).abs() < 0.01 && (min.x + 10.0).abs() < 0.01);
    }

    /// An Extrude with a Circle under it, and a Rectangle under the Circle.
    /// Returns (doc, extrude, circle, rectangle).
    fn extrude_scene() -> (Document, NodeId, NodeId, NodeId) {
        let mut doc = Document::new("extrude");
        let extrude = doc
            .scene
            .insert(Node::new("Extrude", ObjectKindTag::Extrude.default_kind()));
        let circle = doc
            .scene
            .insert_child(extrude, Node::new("Circle", ObjectKindTag::Circle.default_kind()))
            .unwrap();
        let mut rectangle = Node::new("Rectangle", ObjectKindTag::Rectangle.default_kind());
        rectangle.data.transform.position.x = 1000.0;
        let rectangle = doc.scene.insert_child(circle, rectangle).unwrap();
        (doc, extrude, circle, rectangle)
    }

    #[test]
    fn an_extrude_pushes_its_spline_into_a_solid_and_keeps_the_spline_drawn() {
        let (mut doc, extrude, circle, rectangle) = extrude_scene();
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        let result = result_of(&eval, extrude);
        let bounds = result.bounds().unwrap();
        assert!((bounds.max.x - 200.0).abs() < 1e-3 && (bounds.max.z - 20.0).abs() < 1e-3);
        assert!(bounds.max.x < 500.0, "only the first child without Hierarchical");
        assert_eq!(
            node(&eval, extrude).drawable.map(|d| d.key),
            Some(MeshKey::Polygon(result.id()))
        );
        for id in [circle, rectangle] {
            let shown = node(&eval, id);
            assert!(
                matches!(shown.drawable.map(|d| d.key), Some(MeshKey::Curve(_))),
                "splines under a generator stay drawn"
            );
            assert!(matches!(shown.pick, PickShape::Curve(_)));
        }
        assert_eq!(crate::generator_mesh(&doc.scene, extrude, 0.0).unwrap(), *result.mesh());

        // Moving the generator or selecting it changes nothing; a parameter does.
        doc.scene.get_mut(extrude).unwrap().data.transform.position.y = 70.0;
        eval.update(&doc.scene);
        assert_eq!(result_of(&eval, extrude).id(), result.id());
        let ObjectKind::Extrude { hierarchical, .. } = &mut doc.scene.get_mut(extrude).unwrap().data.kind else {
            unreachable!()
        };
        *hierarchical = true;
        eval.update(&doc.scene);
        let both = result_of(&eval, extrude);
        assert_ne!(both.id(), result.id());
        assert!(both.bounds().unwrap().max.x > 1000.0, "the rectangle too");
        assert!(both.mesh().face_count() > result.mesh().face_count());
        // A disabled Extrude makes nothing.
        doc.scene.get_mut(extrude).unwrap().data.enabled = false;
        eval.update(&doc.scene);
        assert_eq!(node(&eval, extrude).drawable, None);
    }

    #[test]
    fn an_extrude_of_a_spline_moved_under_it_is_built_in_its_space() {
        let (mut doc, extrude, circle, _) = extrude_scene();
        doc.scene.get_mut(circle).unwrap().data.transform.position.y = 300.0;
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        let bounds = result_of(&eval, extrude).bounds().unwrap();
        assert!((bounds.min.y - 100.0).abs() < 1e-3, "{bounds:?}");
        // Without a spline: nothing.
        let empty = doc
            .scene
            .insert(Node::new("Extrude", ObjectKindTag::Extrude.default_kind()));
        doc.scene
            .insert_child(empty, Node::new("Cube", ObjectKindTag::Cube.default_kind()))
            .unwrap();
        eval.update(&doc.scene);
        assert_eq!(node(&eval, empty).drawable, None);
    }

    #[test]
    fn a_lathe_turns_its_spline_around_its_y_axis() {
        let mut doc = Document::new("lathe");
        let lathe = doc
            .scene
            .insert(Node::new("Lathe", ObjectKindTag::Lathe.default_kind()));
        let mut circle = Node::new("Circle", ObjectKindTag::Circle.default_kind());
        let ObjectKind::SplinePrimitive(primitive) = &mut circle.data.kind else {
            unreachable!()
        };
        primitive.shape = grid4d_geom::spline::SplineShape::Circle {
            radius: 50.0,
            ellipse: None,
            ring: None,
        };
        circle.data.transform.position.x = 200.0;
        let circle = doc.scene.insert_child(lathe, circle).unwrap();
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        let torus = result_of(&eval, lathe);
        let bounds = torus.bounds().unwrap();
        assert!(
            (bounds.max.x - 250.0).abs() < 1.0 && (bounds.min.z + 250.0).abs() < 1.0,
            "{bounds:?}"
        );
        assert!((bounds.max.y - 50.0).abs() < 1e-3);
        assert!(
            matches!(node(&eval, circle).pick, PickShape::Curve(_)),
            "the profile stays"
        );
        assert_eq!(crate::generator_mesh(&doc.scene, lathe, 0.0).unwrap(), *torus.mesh());

        let ObjectKind::Lathe(options) = &mut doc.scene.get_mut(lathe).unwrap().data.kind else {
            unreachable!()
        };
        options.movement = 100.0;
        eval.update(&doc.scene);
        let raised = result_of(&eval, lathe);
        assert_ne!(raised.id(), torus.id());
        assert!((raised.bounds().unwrap().max.y - 150.0).abs() < 1e-3);
    }

    #[test]
    fn a_sweep_moves_its_profile_along_its_path() {
        let mut doc = Document::new("sweep");
        let sweep = doc
            .scene
            .insert(Node::new("Sweep", ObjectKindTag::Sweep.default_kind()));
        let mut profile = Node::new("Circle", ObjectKindTag::Circle.default_kind());
        let ObjectKind::SplinePrimitive(primitive) = &mut profile.data.kind else {
            unreachable!()
        };
        primitive.shape = grid4d_geom::spline::SplineShape::Circle {
            radius: 10.0,
            ellipse: None,
            ring: None,
        };
        let profile = doc.scene.insert_child(sweep, profile).unwrap();
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        assert_eq!(node(&eval, sweep).drawable, None, "no path yet");

        let path = doc
            .scene
            .insert_child(sweep, Node::new("Helix", ObjectKindTag::Helix.default_kind()))
            .unwrap();
        eval.update(&doc.scene);
        let tube = result_of(&eval, sweep);
        let bounds = tube.bounds().unwrap();
        // The default helix: radius 200, 200 high along Z; the tube 10 thick.
        assert!((bounds.max.x - 210.0).abs() < 2.0, "{bounds:?}");
        assert!(bounds.min.z < 0.0 && bounds.max.z > 200.0, "{bounds:?}");
        for id in [profile, path] {
            assert!(matches!(node(&eval, id).pick, PickShape::Curve(_)), "inputs stay drawn");
        }
        assert_eq!(crate::generator_mesh(&doc.scene, sweep, 0.0).unwrap(), *tube.mesh());

        // The profile is read in its own space; the path in the generator's.
        doc.scene.get_mut(profile).unwrap().data.transform.position.x = 500.0;
        eval.update(&doc.scene);
        assert_eq!(
            result_of(&eval, sweep).mesh(),
            tube.mesh(),
            "moving the profile changes nothing"
        );
        doc.scene.get_mut(path).unwrap().data.transform.position.y = 300.0;
        eval.update(&doc.scene);
        let moved = result_of(&eval, sweep).bounds().unwrap();
        assert!((moved.min.y - bounds.min.y - 300.0).abs() < 1e-2);
    }

    #[test]
    fn a_loft_skins_its_child_splines_in_order() {
        let mut doc = Document::new("loft");
        let loft = doc.scene.insert(Node::new("Loft", ObjectKindTag::Loft.default_kind()));
        let mut sections = Vec::new();
        for (radius, z) in [(200.0, 0.0), (100.0, 300.0)] {
            let mut circle = Node::new("Circle", ObjectKindTag::Circle.default_kind());
            let ObjectKind::SplinePrimitive(primitive) = &mut circle.data.kind else {
                unreachable!()
            };
            primitive.shape = grid4d_geom::spline::SplineShape::Circle {
                radius,
                ellipse: None,
                ring: None,
            };
            circle.data.transform.position.z = z;
            sections.push(doc.scene.insert_child(loft, circle).unwrap());
        }
        // Not a spline: ignored.
        doc.scene
            .insert_child(loft, Node::new("Cube", ObjectKindTag::Cube.default_kind()))
            .unwrap();
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        let skin = result_of(&eval, loft);
        let bounds = skin.bounds().unwrap();
        assert!((bounds.min.z, bounds.max.z) == (0.0, 300.0), "{bounds:?}");
        assert!((bounds.max.x - 200.0).abs() < 1.0);
        for id in &sections {
            assert!(
                matches!(node(&eval, *id).pick, PickShape::Curve(_)),
                "sections stay drawn"
            );
        }
        assert_eq!(crate::generator_mesh(&doc.scene, loft, 0.0).unwrap(), *skin.mesh());
        // Moving a section reshapes the skin.
        doc.scene.get_mut(sections[1]).unwrap().data.transform.position.z = 500.0;
        eval.update(&doc.scene);
        assert!((result_of(&eval, loft).bounds().unwrap().max.z - 500.0).abs() < 1e-3);
    }

    #[test]
    fn only_a_subdivision_surface_hides_its_first_child() {
        let mut doc = Document::new("extrude");
        let extrude = doc
            .scene
            .insert(Node::new("Extrude", ObjectKindTag::Extrude.default_kind()));
        let cube = doc
            .scene
            .insert_child(extrude, Node::new("Cube", ObjectKindTag::Cube.default_kind()))
            .unwrap();
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        assert!(node(&eval, cube).drawable.is_some(), "not an input: drawn as it is");
        assert_eq!(node(&eval, extrude).drawable, None, "nothing to extrude");
        assert_eq!(
            crate::generator_color(&doc.scene, extrude),
            Some(doc.scene.get(extrude).unwrap().data.display_color)
        );
    }

    #[test]
    fn generators_follow_animated_inputs() {
        use grid4d_scene::{Axis, Curve, Interpolation, Key, Param, Track};
        let mut doc = Document::new("loft");
        let loft = doc.scene.insert(Node::new("Loft", ObjectKindTag::Loft.default_kind()));
        doc.scene
            .insert_child(loft, Node::new("Circle", ObjectKindTag::Circle.default_kind()))
            .unwrap();
        let mut top = Node::new("Circle", ObjectKindTag::Circle.default_kind());
        top.data.tracks = vec![Track {
            param: Param::Position(Axis::Z),
            curve: Curve::from_keys([0.0, 10.0].into_iter().zip([100.0, 500.0]).map(|(f, v)| Key {
                interpolation: Interpolation::Linear,
                ..Key::new(f, v)
            })),
        }];
        doc.scene.insert_child(loft, top).unwrap();
        let mut eval = EvaluatedScene::new();
        let height = |eval: &EvaluatedScene| result_of(eval, loft).bounds().unwrap().max.z;
        eval.update(&doc.scene);
        assert!((height(&eval) - 100.0).abs() < 1e-3);
        eval.set_frame(5.0);
        eval.update(&doc.scene);
        assert!((height(&eval) - 300.0).abs() < 1e-3, "{}", height(&eval));
        assert_eq!(
            crate::generator_mesh(&doc.scene, loft, 10.0)
                .unwrap()
                .bounds()
                .unwrap()
                .1
                .z,
            500.0,
            "Make Editable bakes the frame it is given"
        );
    }

    #[test]
    fn text_is_drawn_framed_and_extruded_with_its_holes() {
        let mut doc = Document::new("text");
        let text = doc.scene.insert(Node::new("Text", ObjectKindTag::Text.default_kind()));
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        let shown = node(&eval, text);
        let Some(PickShape::Curve(curve)) = Some(shown.pick) else {
            unreachable!()
        };
        assert!(matches!(shown.drawable.map(|d| d.key), Some(MeshKey::Curve(id)) if id == curve));
        let bounds = shown.bounds.expect("framed by its curve");
        assert!(bounds.max.x > 200.0 && bounds.max.y > 100.0, "{bounds:?}");
        // Unchanged text: the same curve; another text: a new one.
        eval.update(&doc.scene);
        doc.scene.get_mut(text).unwrap().data.transform.position.x = 5.0;
        eval.update(&doc.scene);
        assert_eq!(
            node(&eval, text).pick,
            PickShape::Curve(curve),
            "moving it keeps the curve"
        );
        let ObjectKind::Text(t) = &mut doc.scene.get_mut(text).unwrap().data.kind else {
            unreachable!()
        };
        t.text = "o".into();
        eval.update(&doc.scene);
        assert_ne!(node(&eval, text).pick, PickShape::Curve(curve));

        // Extruded: "o" is a ring, its hole cut out of both caps.
        let extrude = doc
            .scene
            .insert(Node::new("Extrude", ObjectKindTag::Extrude.default_kind()));
        doc.scene.reparent(text, Some(extrude), true).unwrap();
        eval.update(&doc.scene);
        let ring = result_of(&eval, extrude);
        let topology = grid4d_geom::Topology::build(ring.mesh());
        assert!(topology.is_closed(), "a closed solid");
    }

    #[test]
    fn animated_parameters_change_shapes_and_generators_with_the_frame() {
        use grid4d_scene::{Interpolation, Key, ObjectParam, Param, Track};
        let linear = |keys: [(f64, f32); 2]| {
            grid4d_scene::Curve::from_keys(keys.into_iter().map(|(f, v)| Key {
                interpolation: Interpolation::Linear,
                ..Key::new(f, v)
            }))
        };
        let mut doc = Document::new("params");
        let mut sphere = Node::new("Sphere", ObjectKindTag::Sphere.default_kind());
        sphere.data.tracks.push(Track {
            param: Param::Object(ObjectParam::Radius),
            curve: linear([(0.0, 100.0), (10.0, 200.0)]),
        });
        let sphere = doc.scene.insert(sphere);
        let extrude = doc
            .scene
            .insert(Node::new("Extrude", ObjectKindTag::Extrude.default_kind()));
        doc.scene.get_mut(extrude).unwrap().data.tracks.push(Track {
            param: Param::Object(ObjectParam::Offset),
            curve: linear([(0.0, 20.0), (10.0, 120.0)]),
        });
        doc.scene
            .insert_child(extrude, Node::new("Circle", ObjectKindTag::Circle.default_kind()))
            .unwrap();
        let mut eval = EvaluatedScene::new();
        eval.set_frame(5.0);
        eval.update(&doc.scene);
        assert_eq!(node(&eval, sphere).drawable.unwrap().scale, Vec3::splat(150.0));
        assert!((result_of(&eval, extrude).bounds().unwrap().max.z - 70.0).abs() < 1e-3);
        eval.set_frame(10.0);
        assert!(eval.update(&doc.scene), "a new frame re-evaluates animated parameters");
        assert_eq!(node(&eval, sphere).drawable.unwrap().scale, Vec3::splat(200.0));
        assert!((result_of(&eval, extrude).bounds().unwrap().max.z - 120.0).abs() < 1e-3);
    }

    fn bend_node(strength_degrees: f32) -> Node {
        let mut node = Node::new("Bend", ObjectKindTag::Bend.default_kind());
        let ObjectKind::Bend(bend) = &mut node.data.kind else {
            unreachable!()
        };
        bend.size = Vec3::new(100.0, 200.0, 100.0);
        bend.strength = strength_degrees.to_radians();
        node
    }

    fn tall_cube() -> Node {
        Node::new(
            "Cube",
            ObjectKind::Cube {
                size: Vec3::new(20.0, 200.0, 20.0),
                segments: [1, 20, 1],
            },
        )
    }

    #[test]
    fn a_bend_reshapes_its_parent_and_the_parents_subtree() {
        let mut doc = Document::new("bend");
        let group = doc.scene.insert(Node::new("Group", ObjectKind::Null));
        let a = doc.scene.insert_child(group, tall_cube()).unwrap();
        let b = doc.scene.insert_child(group, tall_cube()).unwrap();
        let bend = doc.scene.insert_child(group, bend_node(90.0)).unwrap();
        let outside = doc.scene.insert(tall_cube());
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        for id in [a, b] {
            let shown = node(&eval, id);
            assert!(
                matches!(shown.drawable.unwrap().key, MeshKey::Polygon(_)),
                "drawn from its deformed mesh"
            );
            assert!(
                shown.bounds.unwrap().max.x > 50.0,
                "bent over towards +X: {:?}",
                shown.bounds
            );
        }
        assert!(
            matches!(node(&eval, outside).drawable.unwrap().key, MeshKey::Cube { .. }),
            "out of reach"
        );
        let box_node = node(&eval, bend);
        assert!(
            matches!(box_node.pick, PickShape::Curve(_)),
            "the box is picked like a spline"
        );
        assert_eq!(box_node.kind, ObjectKindTag::Bend);
        assert_eq!(
            doc.scene.get(a).unwrap().data.kind,
            tall_cube().data.kind,
            "the object's own data is untouched"
        );

        // Nothing changed: the same mesh; moving the deformer: a new one.
        let mesh_of = |eval: &EvaluatedScene| match node(eval, a).drawable.unwrap().key {
            MeshKey::Polygon(id) => id,
            other => panic!("{other:?}"),
        };
        let first = mesh_of(&eval);
        doc.scene.get_mut(outside).unwrap().data.transform.position.x = 900.0;
        eval.update(&doc.scene);
        assert_eq!(mesh_of(&eval), first, "unrelated edits rebuild nothing");
        doc.scene.get_mut(bend).unwrap().data.transform.position.y = 30.0;
        eval.update(&doc.scene);
        assert_ne!(mesh_of(&eval), first);

        // Switched off: no deformation.
        doc.scene.get_mut(bend).unwrap().data.enabled = false;
        eval.update(&doc.scene);
        assert!(matches!(node(&eval, a).drawable.unwrap().key, MeshKey::Cube { .. }));
    }

    #[test]
    fn deformers_in_a_cage_shape_the_cage_and_beside_it_the_result() {
        let mut doc = Document::new("sds");
        let sds = doc.scene.insert(Node::new(
            "Subdivision Surface",
            ObjectKindTag::SubdivisionSurface.default_kind(),
        ));
        let cage = doc.scene.insert_child(sds, tall_cube()).unwrap();
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        let straight = result_of(&eval, sds).bounds().unwrap();

        let inner = doc.scene.insert_child(cage, bend_node(90.0)).unwrap();
        eval.update(&doc.scene);
        let bent = result_of(&eval, sds).bounds().unwrap();
        assert!(bent.max.x > straight.max.x + 30.0, "the cage bent before smoothing");

        doc.scene.get_mut(inner).unwrap().data.enabled = false;
        doc.scene.insert_child(sds, bend_node(-90.0)).unwrap();
        eval.update(&doc.scene);
        let result = node(&eval, sds).bounds.unwrap();
        assert!(result.min.x < straight.min.x - 30.0, "the result bent the other way");
    }

    #[test]
    fn an_animated_bend_deforms_per_frame() {
        use grid4d_scene::{Interpolation, Key, ObjectParam, Param, Track};
        let mut doc = Document::new("bend");
        let cube = doc.scene.insert(tall_cube());
        let mut bend = bend_node(0.0);
        bend.data.tracks.push(Track {
            param: Param::Object(ObjectParam::Strength),
            curve: grid4d_scene::Curve::from_keys([0.0, 10.0].map(|f: f64| Key {
                interpolation: Interpolation::Linear,
                ..Key::new(f, if f == 0.0 { 0.0 } else { std::f32::consts::PI })
            })),
        });
        doc.scene.insert_child(cube, bend).unwrap();
        let mut eval = EvaluatedScene::new();
        let reach = |eval: &EvaluatedScene| node(eval, cube).bounds.unwrap().max.x;
        eval.update(&doc.scene);
        assert!(reach(&eval) < 11.0, "unbent at frame 0");
        eval.set_frame(10.0);
        eval.update(&doc.scene);
        assert!(reach(&eval) > 50.0, "half a turn at frame 10: {}", reach(&eval));
    }

    #[test]
    fn twist_taper_and_bulge_reshape_their_parent() {
        use grid4d_geom::deform::{Bulge, Taper, Twist};
        let box_size = Vec3::new(100.0, 200.0, 100.0);
        let kinds = [
            ObjectKind::Twist(Twist {
                size: box_size,
                angle: std::f32::consts::FRAC_PI_4,
                ..Twist::default()
            }),
            ObjectKind::Taper(Taper {
                size: box_size,
                strength: 1.0,
                ..Taper::default()
            }),
            ObjectKind::Bulge(Bulge {
                size: box_size,
                strength: 1.0,
                ..Bulge::default()
            }),
        ];
        let straight = crate::editable_mesh(&tall_cube().data.kind).unwrap();
        for kind in kinds {
            let mut doc = Document::new("deform");
            let cube = doc.scene.insert(tall_cube());
            let deformer = doc.scene.insert_child(cube, Node::new("D", kind.clone())).unwrap();
            let mut eval = EvaluatedScene::new();
            eval.update(&doc.scene);
            let MeshKey::Polygon(mesh) = node(&eval, cube).drawable.unwrap().key else {
                panic!("not drawn from a deformed mesh")
            };
            let deformed = eval.mesh(mesh).unwrap().mesh();
            assert_eq!(deformed.point_count(), straight.point_count());
            assert_ne!(
                deformed.points(),
                straight.points(),
                "{:?} changed the shape",
                kind.tag()
            );
            assert!(matches!(node(&eval, deformer).pick, PickShape::Curve(_)));
        }
    }

    #[test]
    fn an_ffds_lattice_reshapes_its_parent_and_is_edited_as_points() {
        use grid4d_geom::deform::Ffd;
        let mut doc = Document::new("ffd");
        let cube = doc.scene.insert(Node::new(
            "Cube",
            ObjectKind::Cube {
                size: Vec3::splat(100.0),
                segments: [2, 2, 2],
            },
        ));
        let ffd = doc
            .scene
            .insert_child(
                cube,
                Node::new(
                    "FFD",
                    ObjectKind::ffd(Ffd {
                        size: Vec3::splat(100.0),
                        ..Ffd::default()
                    }),
                ),
            )
            .unwrap();
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        let (_, lattice, _) = eval.spline(ffd).expect("the lattice is edited like a spline's points");
        assert_eq!(lattice.spline().points.len(), 27);
        assert_eq!(
            node(&eval, cube).bounds.unwrap().max,
            Vec3::splat(50.0),
            "unmoved: unchanged"
        );

        let ObjectKind::Ffd { lattice, .. } = &mut doc.scene.get_mut(ffd).unwrap().data.kind else {
            unreachable!()
        };
        *lattice = lattice.edited(|s| s.points[26] += Vec3::Y * 40.0);
        eval.update(&doc.scene);
        assert!(
            (node(&eval, cube).bounds.unwrap().max.y - 90.0).abs() < 1e-3,
            "the corner rose"
        );
    }
}
