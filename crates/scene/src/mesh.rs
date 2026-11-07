//! Shared, immutable meshes — the geometry of polygon (editable) objects — and
//! their component selections.

use std::{
    fmt,
    sync::{Arc, OnceLock},
};

use glam::Vec3;
use grid4d_geom::{BitSet, Bvh, ComponentSet, PolyMesh, RayHit, Topology};

use crate::Aabb;

/// Identifies one version of a mesh's content. Unique across all documents, like
/// node revisions, so caches keyed by it (GPU buffers) can never mix up two meshes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MeshId(u64);

/// The mesh of a polygon object: immutable and cheap to clone, because clones
/// share the geometry. Undo steps, the evaluated scene and caches all hold
/// handles without copying points. An edit makes a new handle with a new
/// [`MeshId`]; the previous handle (kept by the undo step) still describes the
/// old geometry.
#[derive(Clone)]
pub struct MeshHandle(Arc<Shared>);

struct Shared {
    id: MeshId,
    mesh: PolyMesh,
    /// Computed once: object bounds are read every time the scene is evaluated.
    bounds: Option<Aabb>,
    /// Built on the first ray cast (picking), then shared by every clone.
    bvh: OnceLock<Bvh>,
    /// Built when first needed (component display, picking, conversions);
    /// shared with handles that only moved points ([`MeshHandle::with_points`]).
    topology: OnceLock<Arc<Topology>>,
}

impl MeshHandle {
    #[must_use]
    pub fn new(mesh: PolyMesh) -> Self {
        let bounds = mesh.bounds().map(|(min, max)| Aabb { min, max });
        Self(Arc::new(Shared {
            id: MeshId(crate::scene::next_revision()),
            mesh,
            bounds,
            bvh: OnceLock::new(),
            topology: OnceLock::new(),
        }))
    }

    #[must_use]
    pub fn id(&self) -> MeshId {
        self.0.id
    }

    #[must_use]
    pub fn mesh(&self) -> &PolyMesh {
        &self.0.mesh
    }

    /// Bounds of the points in object space; `None` for a mesh without points.
    #[must_use]
    pub fn bounds(&self) -> Option<Aabb> {
        self.0.bounds
    }

    /// The closest hit of the object-space ray `origin + t · dir` with the mesh
    /// (`t ≥ 0`, either side of a polygon). The first call builds a BVH, which
    /// every clone of this handle then reuses.
    #[must_use]
    pub fn raycast(&self, origin: Vec3, dir: Vec3) -> Option<RayHit> {
        self.raycast_where(origin, dir, |_| true)
    }

    /// Like [`raycast`](Self::raycast), hitting only polygons `accept` returns
    /// `true` for (hidden polygons are transparent).
    #[must_use]
    pub fn raycast_where(&self, origin: Vec3, dir: Vec3, accept: impl Fn(u32) -> bool) -> Option<RayHit> {
        self.0
            .bvh
            .get_or_init(|| Bvh::build(&self.0.mesh))
            .raycast_where(self.0.mesh.points(), origin, dir, accept)
    }

    /// The mesh's adjacency, built on first use and shared by every clone.
    #[must_use]
    pub fn topology(&self) -> &Topology {
        self.0.topology.get_or_init(|| Arc::new(Topology::build(&self.0.mesh)))
    }

    /// The same faces with the points moved to `points` (one per point; extra
    /// entries are ignored, missing ones keep their old position): a new handle
    /// with a new [`MeshId`] and bounds, sharing the topology, which moving
    /// points cannot change. What transforming components produces.
    #[must_use]
    pub fn with_points(&self, points: &[Vec3]) -> Self {
        let mut mesh = self.0.mesh.clone();
        for (point, &new) in mesh.points_mut().iter_mut().zip(points) {
            *point = new;
        }
        let moved = Self::new(mesh);
        if let Some(topology) = self.0.topology.get() {
            let _ = moved.0.topology.set(Arc::clone(topology));
        }
        moved
    }

    /// The same mesh with the corner UVs `uvs` (one per corner, else `None`):
    /// a new handle sharing the topology, which UVs cannot change. What a UV
    /// drag produces.
    #[must_use]
    pub fn with_uvs(&self, uvs: Vec<glam::Vec2>) -> Option<Self> {
        let mut mesh = self.0.mesh.clone();
        mesh.set_uvs(Some(uvs)).ok()?;
        let changed = Self::new(mesh);
        if let Some(topology) = self.0.topology.get() {
            let _ = changed.0.topology.set(Arc::clone(topology));
        }
        Some(changed)
    }
}

/// Equal when they share geometry or describe the same geometry. Comparing a
/// handle with its own clone (every frame, in the Attribute Manager and in undo
/// bookkeeping) costs nothing.
impl PartialEq for MeshHandle {
    fn eq(&self, other: &Self) -> bool {
        self.0.id == other.0.id || self.0.mesh == other.0.mesh
    }
}

/// Counts instead of contents: a mesh can have millions of points.
impl fmt::Debug for MeshHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MeshHandle")
            .field("id", &self.0.id)
            .field("points", &self.0.mesh.point_count())
            .field("faces", &self.0.mesh.face_count())
            .finish()
    }
}

/// What is selected and hidden on a polygon object, per component kind, and
/// where its UVs are cut.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComponentState {
    pub selected: ComponentSet,
    /// Hidden components are neither drawn nor pickable.
    pub hidden: ComponentSet,
    /// UV seams: edges flagged by corner, like an edge selection (an edge is a
    /// seam when any of its corners is flagged). Unwrap cuts along them.
    pub seams: BitSet,
}

/// A polygon object's [`ComponentState`], shared copy-on-write: snapshots (undo
/// steps, the evaluated scene) are cheap, and [`make_mut`](Self::make_mut) copies
/// only while the state is shared. It sits *next to* the [`MeshHandle`], so
/// selecting never creates a new [`MeshId`] or re-uploads the mesh.
#[derive(Clone, Default)]
pub struct Components(Arc<ComponentState>);

impl Components {
    #[must_use]
    pub fn new(state: ComponentState) -> Self {
        Self(Arc::new(state))
    }

    #[must_use]
    pub fn selected(&self) -> &ComponentSet {
        &self.0.selected
    }

    #[must_use]
    pub fn hidden(&self) -> &ComponentSet {
        &self.0.hidden
    }

    #[must_use]
    pub fn state(&self) -> &ComponentState {
        &self.0
    }

    /// Write access, copying the state first if anything else shares it.
    pub fn make_mut(&mut self) -> &mut ComponentState {
        Arc::make_mut(&mut self.0)
    }

    /// The very same shared state (not merely equal): cheap change detection.
    #[must_use]
    pub fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl PartialEq for Components {
    fn eq(&self, other: &Self) -> bool {
        self.same(other) || self.0 == other.0
    }
}

impl fmt::Debug for Components {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use grid4d_geom::primitives;

    use super::*;

    #[test]
    fn handles_share_geometry_and_compare_by_content() {
        let a = MeshHandle::new(primitives::cube(Vec3::ONE, [1, 1, 1]));
        let clone = a.clone();
        assert_eq!(clone.id(), a.id());
        assert!(std::ptr::eq(clone.mesh(), a.mesh()), "a clone shares the points");
        let same = MeshHandle::new(primitives::cube(Vec3::ONE, [1, 1, 1]));
        assert_ne!(same.id(), a.id(), "every handle has its own id");
        assert_eq!(same, a, "same geometry");
        assert_ne!(MeshHandle::new(primitives::cube(Vec3::ONE, [2, 1, 1])), a);
    }

    #[test]
    fn bounds_are_cached_and_empty_meshes_have_none() {
        let cube = MeshHandle::new(primitives::cube(Vec3::new(2.0, 4.0, 6.0), [1, 1, 1]));
        let b = cube.bounds().unwrap();
        assert!(b.min.abs_diff_eq(Vec3::new(-1.0, -2.0, -3.0), 1e-6));
        assert!(b.max.abs_diff_eq(Vec3::new(1.0, 2.0, 3.0), 1e-6));
        assert_eq!(MeshHandle::new(PolyMesh::new()).bounds(), None);
    }

    #[test]
    fn the_bvh_is_built_on_the_first_ray_and_shared() {
        let sphere = MeshHandle::new(primitives::sphere(10.0, 24));
        let clone = sphere.clone();
        assert!(sphere.0.bvh.get().is_none(), "nothing built before picking");
        let hit = clone.raycast(Vec3::new(0.0, 0.0, 50.0), -Vec3::Z).unwrap();
        assert!((hit.t - 40.0).abs() < 0.1, "front of the sphere, t = {}", hit.t);
        assert!(sphere.0.bvh.get().is_some(), "the original sees the clone's BVH");
        assert_eq!(sphere.raycast(Vec3::new(0.0, 50.0, 50.0), -Vec3::Z), None);
    }

    #[test]
    fn components_copy_only_while_shared() {
        let mut a = Components::default();
        let before: *const ComponentState = a.state();
        a.make_mut().selected.points.insert(3);
        assert!(std::ptr::eq(before, a.state()), "not shared: edited in place");
        let snapshot = a.clone();
        assert!(a.same(&snapshot));
        a.make_mut().selected.points.insert(4);
        assert!(!a.same(&snapshot), "shared: copied first");
        assert_eq!(snapshot.selected().points.len(), 1, "the snapshot is untouched");
        assert_ne!(a, snapshot);
        a.make_mut().selected.points.remove(4);
        assert_eq!(a, snapshot, "equal by content");
    }

    #[test]
    fn topology_is_built_once_and_hidden_faces_let_rays_through() {
        let cube = MeshHandle::new(primitives::cube(Vec3::splat(2.0), [1, 1, 1]));
        assert!(std::ptr::eq(cube.topology(), cube.clone().topology()));
        assert_eq!(cube.topology().edge_count(), 12);
        let front = cube.raycast(Vec3::new(0.0, 0.0, 5.0), -Vec3::Z).unwrap();
        let back = cube
            .raycast_where(Vec3::new(0.0, 0.0, 5.0), -Vec3::Z, |f| f != front.face)
            .unwrap();
        assert!((back.t - 6.0).abs() < 1e-5);
    }

    #[test]
    fn moved_points_keep_the_topology_but_not_the_bvh() {
        let cube = MeshHandle::new(primitives::cube(Vec3::splat(2.0), [1, 1, 1]));
        let _ = cube.raycast(Vec3::new(0.0, 0.0, 5.0), -Vec3::Z); // build the BVH…
        let _ = cube.topology(); // …and the topology
        let mut points = cube.mesh().points().to_vec();
        points[0] += Vec3::new(-10.0, 0.0, 0.0);
        let moved = cube.with_points(&points);
        assert_ne!(moved.id(), cube.id());
        assert!(std::ptr::eq(moved.topology(), cube.topology()), "faces did not change");
        assert!(moved.0.bvh.get().is_none(), "positions did: the BVH is rebuilt");
        let lowest = points.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
        assert_eq!(moved.bounds().unwrap().min.x, lowest, "bounds follow the points");
        assert_eq!(moved.mesh().face(0), cube.mesh().face(0));
    }

    #[test]
    fn debug_output_stays_small() {
        let text = format!("{:?}", MeshHandle::new(primitives::sphere(1.0, 64)));
        assert!(text.contains("faces"), "{text}");
        assert!(text.len() < 100, "{text}");
    }
}
