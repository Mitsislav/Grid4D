//! Deformers in evaluation (see `docs/reference/deformer-basics.md`): which
//! deformers reach an object, the chain of them it goes through, and the
//! deformed mesh, cached per node and rebuilt only when the chain or the
//! source changes. Deformer plugins (`grid4d-plugin`) take their place in the
//! chain like the built-in ones.

use std::{collections::HashMap, sync::Arc};

use glam::{Mat3, Mat4, Vec3};
use grid4d_geom::{
    PolyMesh,
    deform::{Bend, Bulge, Ffd, Taper, Twist},
    deformers::{Displace, Explode, FollowSpline, GreyPicture, ProjectToSurface, Smooth, Surface, Wrap, marker_guide},
    spline::Polyline,
};
use grid4d_plugin::DeformerPlugin;
use grid4d_scene::{MeshHandle, NodeId, ObjectKind, PluginObject, Scene, SplineHandle, TextureId};

/// The pictures Displace deformers read (by texture), as the editor hands
/// them to evaluation (see [`crate::EvaluatedScene::set_pictures`]): grey
/// values, shared. A texture missing here displaces nothing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pictures(HashMap<TextureId, SharedPicture>);

impl Pictures {
    /// Gives `texture` its picture (replacing one it had).
    pub fn insert(&mut self, texture: TextureId, picture: Arc<GreyPicture>) {
        self.0.insert(texture, SharedPicture(picture));
    }

    /// `texture`'s picture, if it has one.
    #[must_use]
    pub fn get(&self, texture: TextureId) -> Option<&Arc<GreyPicture>> {
        self.0.get(&texture).map(|p| &p.0)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A picture compared by identity (the same decoded picture), so comparing
/// chains stays cheap.
#[derive(Clone, Debug)]
pub struct SharedPicture(Arc<GreyPicture>);

impl PartialEq for SharedPicture {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// What a Project to Surface deformer pulls points onto: a polygon
/// object's mesh (compared by id) or a primitive's (made when applied),
/// with the matrix from the deformer's space into the target's.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Target {
    shape: TargetShape,
    into: Mat4,
}

#[derive(Clone, Debug, PartialEq)]
enum TargetShape {
    Mesh(MeshHandle),
    Primitive(ObjectKind),
}

/// A target placed in the deformer's space, for [`ProjectToSurface::apply`].
struct Placed<'a> {
    mesh: &'a MeshHandle,
    into: Mat4,
    back: Mat4,
    normals: Mat3,
}

impl Placed<'_> {
    fn normal(&self, face: u32) -> Vec3 {
        let face = usize::try_from(face).unwrap_or(usize::MAX);
        if face >= self.mesh.mesh().face_count() {
            return Vec3::ZERO;
        }
        (self.normals * self.mesh.mesh().face_normal(face)).normalize_or_zero()
    }
}

impl Surface for Placed<'_> {
    fn nearest(&self, point: Vec3) -> Option<(Vec3, Vec3)> {
        let (at, face) = self.mesh.nearest(self.into.transform_point3(point))?;
        Some((self.back.transform_point3(at), self.normal(face)))
    }

    fn cast(&self, origin: Vec3, dir: Vec3) -> Option<(Vec3, Vec3)> {
        let local_origin = self.into.transform_point3(origin);
        let local_dir = self.into.transform_vector3(dir);
        let hit = self.mesh.raycast(local_origin, local_dir)?;
        let at = local_origin + local_dir * hit.t;
        Some((self.back.transform_point3(at), self.normal(hit.face)))
    }
}

/// `true` if `kind` deforms: a built-in deformer, or an object of an installed
/// deformer plugin.
#[must_use]
pub fn is_deformer(kind: &ObjectKind) -> bool {
    kind.is_deformer()
        || matches!(kind, ObjectKind::Plugin(o) if grid4d_plugin::registry().deformer(&o.type_id).is_some())
}

/// A deformer plugin's object, with its plugin (shared: cloning a chain does
/// not copy its values). Two are equal when their values are.
#[derive(Clone)]
pub(crate) struct PluginDeformer {
    plugin: &'static dyn DeformerPlugin,
    object: Arc<PluginObject>,
}

impl PartialEq for PluginDeformer {
    fn eq(&self, other: &Self) -> bool {
        self.object == other.object
    }
}

impl std::fmt::Debug for PluginDeformer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.object.fmt(f)
    }
}

/// A deformer's parameters, as a chain applies them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Deformer {
    Bend(Bend),
    Twist(Twist),
    Taper(Taper),
    Bulge(Bulge),
    /// With its lattice (shared: cloning does not copy it).
    Ffd(Ffd, SplineHandle),
    Wrap(Wrap),
    /// With its path in the deformer's space (none: nothing moves).
    FollowSpline(FollowSpline, Option<Arc<Polyline>>),
    /// With its picture (none: a Texture source moves nothing).
    Displace(Displace, Option<SharedPicture>),
    /// With its target (none: nothing moves).
    Project(ProjectToSurface, Option<Target>),
    Smooth(Smooth),
    Explode(Explode),
    Plugin(PluginDeformer),
}

impl Deformer {
    /// The deformer `kind` is, if it is one.
    pub(crate) fn of(kind: &ObjectKind) -> Option<Self> {
        match kind {
            ObjectKind::Bend(d) => Some(Self::Bend(*d)),
            ObjectKind::Twist(d) => Some(Self::Twist(*d)),
            ObjectKind::Taper(d) => Some(Self::Taper(*d)),
            ObjectKind::Bulge(d) => Some(Self::Bulge(*d)),
            ObjectKind::Ffd { ffd, lattice, .. } => Some(Self::Ffd(*ffd, lattice.clone())),
            ObjectKind::Wrap(d) => Some(Self::Wrap(*d)),
            ObjectKind::FollowSpline { follow, .. } => Some(Self::FollowSpline(*follow, None)),
            ObjectKind::Displace { displace, .. } => Some(Self::Displace(*displace, None)),
            ObjectKind::ProjectToSurface { project, .. } => Some(Self::Project(*project, None)),
            ObjectKind::Smooth(d) => Some(Self::Smooth(*d)),
            ObjectKind::Explode(d) => Some(Self::Explode(*d)),
            ObjectKind::Plugin(object) => grid4d_plugin::registry().deformer(&object.type_id).map(|plugin| {
                Self::Plugin(PluginDeformer {
                    plugin,
                    object: Arc::new(object.clone()),
                })
            }),
            _ => None,
        }
    }

    /// Its box as drawn (deformed by itself).
    pub(crate) fn box_lines(&self) -> Polyline {
        match self {
            Self::Bend(d) => d.box_lines(),
            Self::Twist(d) => d.box_lines(),
            Self::Taper(d) => d.box_lines(),
            Self::Bulge(d) => d.box_lines(),
            Self::Ffd(d, lattice) => d.lattice_lines(&lattice.spline().points),
            Self::Wrap(d) => d.guide(),
            Self::FollowSpline(..) | Self::Displace(..) | Self::Project(..) | Self::Smooth(_) | Self::Explode(_) => {
                marker_guide()
            }
            Self::Plugin(d) => d.plugin.box_lines(&d.object),
        }
    }

    /// The deformer `kind` is (placed at `frame` with `world_inverse`), with
    /// what it links to resolved: a path in its space, a picture, a target.
    fn resolved(
        scene: &Scene,
        kind: &ObjectKind,
        frame: f64,
        world_inverse: Mat4,
        pictures: &Pictures,
    ) -> Option<Self> {
        let mut deformer = Self::of(kind)?;
        match (&mut deformer, kind) {
            (
                Self::FollowSpline(_, path),
                ObjectKind::FollowSpline {
                    spline: Some(spline), ..
                },
            ) => {
                *path = path_of(scene, *spline, frame).map(|mut line| {
                    let into = world_inverse * scene.world_matrix_at(*spline, frame);
                    line.points.iter_mut().for_each(|p| *p = into.transform_point3(*p));
                    Arc::new(line)
                });
            }
            (
                Self::Displace(_, picture),
                ObjectKind::Displace {
                    texture: Some(texture), ..
                },
            ) => {
                *picture = pictures.0.get(texture).cloned();
            }
            (Self::Project(_, target), ObjectKind::ProjectToSurface { target: Some(node), .. }) => {
                let shape = scene.get(*node).and_then(|n| match &*n.data.kind_at(frame) {
                    ObjectKind::Polygon { mesh, .. } => Some(TargetShape::Mesh(mesh.clone())),
                    kind => crate::generator::editable_mesh_exists(kind).then(|| TargetShape::Primitive(kind.clone())),
                });
                *target = shape.map(|shape| Target {
                    shape,
                    into: scene.world_matrix_at(*node, frame).inverse() * world_inverse.inverse(),
                });
            }
            _ => {}
        }
        Some(deformer)
    }

    /// `mesh`, in the deformer's space, deformed.
    fn apply(&self, mesh: &mut PolyMesh) {
        match self {
            Self::Bend(d) => mesh.points_mut().iter_mut().for_each(|p| *p = d.apply(*p)),
            Self::Twist(d) => mesh.points_mut().iter_mut().for_each(|p| *p = d.apply(*p)),
            Self::Taper(d) => mesh.points_mut().iter_mut().for_each(|p| *p = d.apply(*p)),
            Self::Bulge(d) => mesh.points_mut().iter_mut().for_each(|p| *p = d.apply(*p)),
            Self::Ffd(d, lattice) => {
                let lattice = &lattice.spline().points;
                mesh.points_mut().iter_mut().for_each(|p| *p = d.apply(lattice, *p));
            }
            Self::Wrap(d) => mesh.points_mut().iter_mut().for_each(|p| *p = d.apply(*p)),
            Self::FollowSpline(d, path) => {
                if let Some(path) = path {
                    d.apply(mesh.points_mut(), path);
                }
            }
            Self::Displace(d, picture) => d.apply(mesh, picture.as_ref().map(|p| &*p.0)),
            Self::Project(d, Some(target)) => {
                let built;
                let handle = match &target.shape {
                    TargetShape::Mesh(handle) => handle,
                    TargetShape::Primitive(kind) => {
                        let Some(made) = crate::editable_mesh(kind) else { return };
                        built = MeshHandle::new(made);
                        &built
                    }
                };
                let back = target.into.inverse();
                let placed = Placed {
                    mesh: handle,
                    into: target.into,
                    back,
                    normals: Mat3::from_mat4(back).inverse().transpose(),
                };
                d.apply(mesh, &placed);
            }
            Self::Project(_, None) => {}
            Self::Smooth(d) => d.apply(mesh),
            Self::Explode(d) => *mesh = d.apply(mesh),
            // All at once: a plugin reads its values once per mesh.
            Self::Plugin(d) => {
                d.plugin.deform_checked(&d.object, mesh.points_mut());
            }
        }
    }
}

/// The curve of spline object `id` at `frame`, in its own space: a spline
/// primitive's, a text's or an editable spline's; `None` for other objects.
fn path_of(scene: &Scene, id: NodeId, frame: f64) -> Option<Polyline> {
    let data = &scene.get(id)?.data;
    match &*data.kind_at(frame) {
        ObjectKind::SplinePrimitive(primitive) => Some(primitive.polyline()),
        ObjectKind::Text(text) => Some(grid4d_text::text_spline(text).sample(text.interpolation)),
        ObjectKind::Spline {
            spline, interpolation, ..
        } => Some(spline.spline().sample(*interpolation)),
        _ => None,
    }
}

/// One deformer of an object's chain: how to get from the object's space into
/// the deformer's and back.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Link {
    to: Mat4,
    from: Mat4,
    deformer: Deformer,
}

impl Link {
    fn new(deformer_world_inverse: Mat4, object_world: Mat4, deformer: Deformer) -> Self {
        let to = deformer_world_inverse * object_world;
        Self {
            to,
            from: to.inverse(),
            deformer,
        }
    }

    /// `mesh`, in the object's space, through the deformer and back.
    fn apply(&self, mesh: &mut PolyMesh) {
        mesh.points_mut()
            .iter_mut()
            .for_each(|p| *p = self.to.transform_point3(*p));
        self.deformer.apply(mesh);
        mesh.points_mut()
            .iter_mut()
            .for_each(|p| *p = self.from.transform_point3(*p));
    }
}

/// A deformer that reaches the objects below its parent, while walking the scene.
#[derive(Clone, Debug)]
pub(crate) struct Active {
    /// The depth of its parent: it applies to nodes at this depth or deeper.
    depth: u32,
    world_inverse: Mat4,
    deformer: Deformer,
}

/// Pushes the switched-on deformer children of `id` (at `depth`), in Object
/// Manager order, placed as they are at `frame`.
pub(crate) fn push_children(
    scene: &Scene,
    id: NodeId,
    depth: u32,
    frame: f64,
    pictures: &Pictures,
    active: &mut Vec<Active>,
) {
    for &child in scene.children(id) {
        // Switched off, or on a layer whose deformers are off: it does nothing.
        let Some(data) = scene
            .get(child)
            .map(|n| &n.data)
            .filter(|d| d.enabled && scene.layer_switches(child).deformers)
        else {
            continue;
        };
        let world_inverse = scene.world_matrix_at(child, frame).inverse();
        if let Some(deformer) = Deformer::resolved(scene, &data.kind_at(frame), frame, world_inverse, pictures) {
            active.push(Active {
                depth,
                world_inverse,
                deformer,
            });
        }
    }
}

/// Drops the deformers that do not reach a node at `depth` (those of earlier
/// siblings' subtrees).
pub(crate) fn leave(active: &mut Vec<Active>, depth: u32) {
    while active.last().is_some_and(|a| a.depth >= depth) {
        active.pop();
    }
}

/// The chain for an object at `world` (cleared first): the deepest deformers
/// first, siblings in Object Manager order.
pub(crate) fn chain(active: &[Active], world: Mat4, out: &mut Vec<Link>) {
    out.clear();
    let mut end = active.len();
    while end > 0 {
        let depth = active[end - 1].depth;
        let mut start = end;
        while start > 0 && active[start - 1].depth == depth {
            start -= 1;
        }
        out.extend(
            active[start..end]
                .iter()
                .map(|a| Link::new(a.world_inverse, world, a.deformer.clone())),
        );
        end = start;
    }
}

/// The chain for `node` inside generator `generator`'s input: the deformers
/// below the generator on the way up from `node` (deepest first), placed at
/// `frame`, as seen from `node`.
pub(crate) fn input_chain(
    scene: &Scene,
    node: NodeId,
    generator: NodeId,
    frame: f64,
    pictures: &Pictures,
) -> Vec<Link> {
    let mut links = Vec::new();
    let world = scene.world_matrix_at(node, frame);
    let mut at = Some(node);
    while let Some(current) = at.filter(|&c| c != generator) {
        for &child in scene.children(current) {
            let Some(data) = scene
                .get(child)
                .map(|n| &n.data)
                .filter(|d| d.enabled && scene.layer_switches(child).deformers)
            else {
                continue;
            };
            let world_inverse = scene.world_matrix_at(child, frame).inverse();
            if let Some(deformer) = Deformer::resolved(scene, &data.kind_at(frame), frame, world_inverse, pictures) {
                links.push(Link::new(world_inverse, world, deformer));
            }
        }
        at = scene.get(current).and_then(grid4d_scene::Node::parent);
    }
    links
}

/// `mesh` with every point passed through `links` in turn.
pub(crate) fn deform_mesh(mesh: &PolyMesh, links: &[Link]) -> PolyMesh {
    let mut out = mesh.clone();
    for link in links {
        link.apply(&mut out);
    }
    out
}

/// What a deformed mesh was made from.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Base {
    /// A polygon object's or a generator's mesh (compared by id).
    Mesh(MeshHandle),
    /// A primitive's parameters (its mesh is made when needed).
    Primitive(ObjectKind),
}

/// A node's deformed mesh and what it was made from.
#[derive(Debug)]
pub(crate) struct Deformed {
    base: Base,
    links: Vec<Link>,
    pub(crate) mesh: MeshHandle,
    /// The scene revision of the last update that used it.
    pub(crate) seen: u64,
}

/// `id`'s deformed mesh, rebuilt only when `base` or `links` changed; `None`
/// when the base has no mesh.
pub(crate) fn deformed<'a>(
    cache: &'a mut HashMap<NodeId, Deformed>,
    id: NodeId,
    base: Base,
    links: &[Link],
    revision: u64,
) -> Option<&'a Deformed> {
    let fresh = cache
        .get(&id)
        .is_some_and(|entry| entry.base == base && entry.links == links);
    if !fresh {
        let source = match &base {
            Base::Mesh(handle) => handle.mesh().clone(),
            Base::Primitive(kind) => crate::editable_mesh(kind)?,
        };
        let mesh = MeshHandle::new(deform_mesh(&source, links));
        cache.insert(
            id,
            Deformed {
                base,
                links: links.to_vec(),
                mesh,
                seen: revision,
            },
        );
    }
    let entry = cache.get_mut(&id)?;
    entry.seen = revision;
    Some(entry)
}
