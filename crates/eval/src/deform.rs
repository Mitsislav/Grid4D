//! Deformers in evaluation (see `docs/reference/deformer-basics.md`): which
//! deformers reach an object, the chain of them it goes through, and the
//! deformed mesh, cached per node and rebuilt only when the chain or the
//! source changes.

use std::collections::HashMap;

use glam::{Mat4, Vec3};
use grid4d_geom::{
    PolyMesh,
    deform::{Bend, Bulge, Ffd, Taper, Twist},
    spline::Polyline,
};
use grid4d_scene::{MeshHandle, NodeId, ObjectKind, Scene, SplineHandle};

/// A deformer's parameters, as a chain applies them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Deformer {
    Bend(Bend),
    Twist(Twist),
    Taper(Taper),
    Bulge(Bulge),
    /// With its lattice (shared: cloning does not copy it).
    Ffd(Ffd, SplineHandle),
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
        }
    }

    /// `point`, in the deformer's space, deformed.
    fn apply(&self, point: Vec3) -> Vec3 {
        match self {
            Self::Bend(d) => d.apply(point),
            Self::Twist(d) => d.apply(point),
            Self::Taper(d) => d.apply(point),
            Self::Bulge(d) => d.apply(point),
            Self::Ffd(d, lattice) => d.apply(&lattice.spline().points, point),
        }
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

    fn apply(&self, point: Vec3) -> Vec3 {
        self.from
            .transform_point3(self.deformer.apply(self.to.transform_point3(point)))
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
pub(crate) fn push_children(scene: &Scene, id: NodeId, depth: u32, frame: f64, active: &mut Vec<Active>) {
    for &child in scene.children(id) {
        let Some(data) = scene.get(child).map(|n| &n.data).filter(|d| d.enabled) else {
            continue;
        };
        if let Some(deformer) = Deformer::of(&data.kind_at(frame)) {
            active.push(Active {
                depth,
                world_inverse: scene.world_matrix_at(child, frame).inverse(),
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
pub(crate) fn input_chain(scene: &Scene, node: NodeId, generator: NodeId, frame: f64) -> Vec<Link> {
    let mut links = Vec::new();
    let world = scene.world_matrix_at(node, frame);
    let mut at = Some(node);
    while let Some(current) = at.filter(|&c| c != generator) {
        for &child in scene.children(current) {
            let Some(data) = scene.get(child).map(|n| &n.data).filter(|d| d.enabled) else {
                continue;
            };
            if let Some(deformer) = Deformer::of(&data.kind_at(frame)) {
                links.push(Link::new(
                    scene.world_matrix_at(child, frame).inverse(),
                    world,
                    deformer,
                ));
            }
        }
        at = scene.get(current).and_then(grid4d_scene::Node::parent);
    }
    links
}

/// `mesh` with every point passed through `links` in turn.
pub(crate) fn deform_mesh(mesh: &PolyMesh, links: &[Link]) -> PolyMesh {
    let mut out = mesh.clone();
    for point in out.points_mut() {
        *point = links.iter().fold(*point, |p, link| link.apply(p));
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
