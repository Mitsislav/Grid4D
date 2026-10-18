//! GPU vertices for the unit geometry named by a [`MeshKey`] (see `grid4d-eval`):
//! generated with `grid4d_geom`, triangulated, and uploaded once per key by
//! [`ViewportRenderer`](crate::ViewportRenderer).

use bytemuck::{Pod, Zeroable};
use grid4d_eval::MeshKey;
use grid4d_geom::{Smoothing, TriMesh, Triangulator};

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

/// Buffers reused while generating meshes for new keys.
#[derive(Debug, Default)]
pub struct TessellationScratch {
    triangulator: Triangulator,
    tri: TriMesh,
}

/// Builds and triangulates the unit geometry of `key` into GPU vertices and
/// indices (both cleared first), with Phong-angle smoothing.
pub fn tessellate(key: MeshKey, scratch: &mut TessellationScratch, vertices: &mut Vec<Vertex>, indices: &mut Vec<u32>) {
    let mesh = key.build();
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

#[cfg(test)]
mod tests {
    use glam::Vec3;
    use grid4d_eval::Ratio;

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
            tessellate(key, &mut scratch, &mut vertices, &mut indices);
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
}
