//! What the viewport draws an object with: a [`MeshKey`] names the object's
//! geometry at *unit size*, and its size becomes the per-instance scale.
//!
//! Objects that differ only in size share one key, one GPU mesh and one
//! instanced draw call. Parameters that change the shape itself — segment counts,
//! a cone's radius ratio, a torus's pipe thickness — are part of the key, and the
//! geometry for a new key is generated once with `oc4d_geom::primitives`.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use oc4d_geom::{PolyMesh, Smoothing, TriMesh, Triangulator, primitives};

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

/// A shape ratio in `[0, 1]`, stored as `f32` bits so keys can be hashed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ratio(u32);

impl Ratio {
    /// `value / of`, clamped to `[0, 1]`; `-0.0` and `+0.0` give the same key.
    #[must_use]
    pub fn new(value: f32, of: f32) -> Self {
        let r = if of > 0.0 { (value / of).clamp(0.0, 1.0) } else { 0.0 };
        Self((r + 0.0).to_bits())
    }

    #[must_use]
    pub fn get(self) -> f32 {
        f32::from_bits(self.0)
    }
}

/// Unit-size geometry of a drawable object (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MeshKey {
    /// Cube spanning `[-0.5, 0.5]³`.
    Cube { segments: [u32; 3] },
    /// Sphere of radius 1.
    Sphere { segments: u32 },
    /// Radius 1, height 1.
    Cylinder { rotation: u32, height: u32 },
    /// 1 × 1 on the XZ plane.
    Plane { width: u32, depth: u32 },
    /// Height 1; the larger radius is 1.
    Cone {
        bottom: Ratio,
        top: Ratio,
        rotation: u32,
        height: u32,
    },
    /// Outer radius 1: `ring` is the ring radius' share of it, the pipe takes the rest.
    Torus {
        ring: Ratio,
        ring_segments: u32,
        pipe_segments: u32,
    },
    /// The larger radius is 1.
    Disc {
        inner: Ratio,
        outer: Ratio,
        rotation: u32,
        segments: u32,
    },
    /// Radius 1; `straight` is the share of the total height that is cylinder.
    Capsule {
        straight: Ratio,
        rotation: u32,
        height: u32,
        cap: u32,
    },
}

impl MeshKey {
    /// The unit geometry for this key.
    #[must_use]
    pub fn build(self) -> PolyMesh {
        match self {
            Self::Cube { segments } => primitives::cube(Vec3::ONE, segments),
            Self::Sphere { segments } => primitives::sphere(1.0, segments),
            Self::Cylinder { rotation, height } => primitives::cylinder(1.0, 1.0, rotation, height),
            Self::Plane { width, depth } => primitives::plane(1.0, 1.0, width, depth),
            Self::Cone {
                bottom,
                top,
                rotation,
                height,
            } => primitives::cone(bottom.get(), top.get(), 1.0, rotation, height),
            Self::Torus {
                ring,
                ring_segments,
                pipe_segments,
            } => primitives::torus(ring.get(), 1.0 - ring.get(), ring_segments, pipe_segments),
            Self::Disc {
                inner,
                outer,
                rotation,
                segments,
            } => primitives::disc(inner.get(), outer.get(), rotation, segments),
            Self::Capsule {
                straight,
                rotation,
                height,
                cap,
            } => {
                // Radius 1 and total height h: the straight part is h − 2 = s·h.
                let s = straight.get().min(0.999);
                primitives::capsule(1.0, 2.0 / (1.0 - s), rotation, height, cap)
            }
        }
    }

    /// Builds and triangulates the unit geometry into GPU vertices and indices.
    /// `scratch` buffers are reused across calls.
    pub fn tessellate(self, scratch: &mut TessellationScratch, vertices: &mut Vec<Vertex>, indices: &mut Vec<u32>) {
        let mesh = self.build();
        scratch
            .triangulator
            .triangulate(&mesh, Smoothing::default(), &mut scratch.tri);
        vertices.clear();
        vertices.extend(
            scratch
                .tri
                .positions
                .iter()
                .zip(&scratch.tri.normals)
                .map(|(p, n)| Vertex {
                    position: p.to_array(),
                    normal: n.to_array(),
                }),
        );
        indices.clear();
        indices.extend_from_slice(&scratch.tri.indices);
    }
}

/// Buffers reused while generating meshes for new keys.
#[derive(Debug, Default)]
pub struct TessellationScratch {
    triangulator: Triangulator,
    tri: TriMesh,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_keys() -> Vec<MeshKey> {
        vec![
            MeshKey::Cube { segments: [1, 2, 3] },
            MeshKey::Sphere { segments: 24 },
            MeshKey::Cylinder {
                rotation: 36,
                height: 4,
            },
            MeshKey::Plane { width: 4, depth: 2 },
            MeshKey::Cone {
                bottom: Ratio::new(1.0, 1.0),
                top: Ratio::new(0.0, 1.0),
                rotation: 36,
                height: 8,
            },
            MeshKey::Torus {
                ring: Ratio::new(200.0, 250.0),
                ring_segments: 36,
                pipe_segments: 18,
            },
            MeshKey::Disc {
                inner: Ratio::new(0.0, 1.0),
                outer: Ratio::new(1.0, 1.0),
                rotation: 36,
                segments: 2,
            },
            MeshKey::Capsule {
                straight: Ratio::new(100.0, 200.0),
                rotation: 36,
                height: 4,
                cap: 8,
            },
        ]
    }

    /// Every unit mesh is non-empty, indexes in bounds and faces the way its
    /// vertex normals do (counter-clockwise from outside).
    #[test]
    fn unit_meshes_are_drawable_and_wound_outwards() {
        let mut scratch = TessellationScratch::default();
        let (mut vertices, mut indices) = (Vec::new(), Vec::new());
        for key in sample_keys() {
            key.tessellate(&mut scratch, &mut vertices, &mut indices);
            assert!(!indices.is_empty() && indices.len() % 3 == 0, "{key:?}");
            let n = u32::try_from(vertices.len()).unwrap();
            assert!(indices.iter().all(|&i| i < n), "{key:?}");
            for tri in indices.as_chunks::<3>().0 {
                let [a, b, c] = tri.map(|i| vertices[i as usize]);
                let (pa, pb, pc) = (Vec3::from(a.position), Vec3::from(b.position), Vec3::from(c.position));
                let face = (pb - pa).cross(pc - pa);
                let normals = Vec3::from(a.normal) + Vec3::from(b.normal) + Vec3::from(c.normal);
                assert!(face.dot(normals) > 0.0, "{key:?}: triangle {tri:?} is wound inside-out");
            }
        }
    }

    #[test]
    fn unit_meshes_have_unit_size() {
        let size = |key: MeshKey| key.build().bounds().map(|(min, max)| max - min).unwrap();
        assert!(size(MeshKey::Cube { segments: [1, 1, 1] }).abs_diff_eq(Vec3::ONE, 1e-6));
        assert!(size(MeshKey::Sphere { segments: 24 }).abs_diff_eq(Vec3::splat(2.0), 1e-3));
        assert!(
            size(MeshKey::Cylinder {
                rotation: 36,
                height: 1
            })
            .abs_diff_eq(Vec3::new(2.0, 1.0, 2.0), 1e-3)
        );
        let capsule = MeshKey::Capsule {
            straight: Ratio::new(100.0, 200.0),
            rotation: 36,
            height: 1,
            cap: 8,
        };
        assert!(
            size(capsule).abs_diff_eq(Vec3::new(2.0, 4.0, 2.0), 1e-3),
            "radius 1, total height 4"
        );
    }

    #[test]
    fn ratios_are_canonical() {
        assert_eq!(Ratio::new(-0.0, 1.0), Ratio::new(0.0, 1.0));
        assert_eq!(Ratio::new(5.0, 0.0).get(), 0.0);
        assert_eq!(Ratio::new(3.0, 2.0).get(), 1.0);
    }
}
