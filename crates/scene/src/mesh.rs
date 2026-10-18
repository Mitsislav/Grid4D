//! Shared, immutable meshes: the geometry of polygon (editable) objects.

use std::{
    fmt,
    sync::{Arc, OnceLock},
};

use glam::Vec3;
use grid4d_geom::{Bvh, PolyMesh, RayHit};

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
        self.0
            .bvh
            .get_or_init(|| Bvh::build(&self.0.mesh))
            .raycast(self.0.mesh.points(), origin, dir)
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
    fn debug_output_stays_small() {
        let text = format!("{:?}", MeshHandle::new(primitives::sphere(1.0, 64)));
        assert!(text.contains("faces"), "{text}");
        assert!(text.len() < 100, "{text}");
    }
}
