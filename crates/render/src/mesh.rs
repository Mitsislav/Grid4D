//! GPU vertices for the geometry named by a [`MeshKey`](grid4d_eval::MeshKey):
//! a primitive's unit mesh or a polygon object's mesh, triangulated and uploaded
//! once per key by [`ViewportRenderer`](crate::ViewportRenderer).

use bytemuck::{Pod, Zeroable};
use glam::{Vec2, Vec4};
use grid4d_geom::{PolyMesh, Smoothing, TriMesh, Triangulator};

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    /// Texture coordinates (the mesh's own, or a projection's).
    pub uv: [f32; 2],
    /// Along increasing `u`, `w` the side of up the picture (normal maps;
    /// zero where none is needed).
    pub tangent: [f32; 4],
}

/// Buffers reused while generating meshes for new keys.
#[derive(Debug, Default)]
pub struct TessellationScratch {
    triangulator: Triangulator,
    tri: TriMesh,
    tangents: Vec<Vec4>,
}

/// Triangulates `mesh` into GPU vertices and indices (both cleared first),
/// smoothed with `smoothing` (the object's Phong angle). `uvs` (one per corner
/// of `mesh`) replaces the mesh's own texture coordinates; without either the
/// vertices get (0, 0). With `tangents` the vertices get them (normal and
/// bump maps); they cost a little, so only meshes that need them ask.
pub fn tessellate(
    mesh: &PolyMesh,
    smoothing: Smoothing,
    uvs: Option<&[Vec2]>,
    tangents: bool,
    scratch: &mut TessellationScratch,
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
) {
    scratch.triangulator.triangulate(mesh, smoothing, &mut scratch.tri);
    // The triangulated mesh has one vertex per corner, in corner order.
    let uvs = uvs
        .filter(|uvs| uvs.len() == scratch.tri.positions.len())
        .unwrap_or(&scratch.tri.uvs);
    scratch.tangents.clear();
    if tangents {
        let tri = &scratch.tri;
        grid4d_geom::uv::tangents(&tri.positions, &tri.normals, uvs, &tri.indices, &mut scratch.tangents);
    }
    let tangent_of = &scratch.tangents;
    vertices.clear();
    vertices.extend(
        scratch
            .tri
            .positions
            .iter()
            .zip(&scratch.tri.normals)
            .enumerate()
            .map(|(i, (p, n))| Vertex {
                position: p.to_array(),
                normal: n.to_array(),
                uv: uvs.get(i).map_or([0.0; 2], |uv| uv.to_array()),
                tangent: tangent_of.get(i).map_or([0.0; 4], |t| t.to_array()),
            }),
    );
    indices.clear();
    indices.extend_from_slice(&scratch.tri.indices);
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use glam::Vec3;
    use grid4d_eval::{MeshKey, Ratio};
    use grid4d_scene::MeshHandle;

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
            tessellate(
                &key.build().unwrap(),
                Smoothing::default(),
                None,
                true,
                &mut scratch,
                &mut vertices,
                &mut indices,
            );
            assert!(!indices.is_empty() && indices.len() % 3 == 0, "{key:?}");
            assert!(
                vertices.iter().all(|v| v.uv.iter().all(|c| c.is_finite())),
                "{key:?}: texture coordinates"
            );
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

    /// Phase 3 acceptance for big meshes: a million triangles become a GPU mesh,
    /// and get picked, in interactive time. Timing-dependent, so run it by hand:
    /// `cargo test --release -p grid4d-render -- --ignored --nocapture`.
    #[test]
    #[ignore = "timing; run in release"]
    fn a_million_triangles_upload_and_pick_quickly() {
        // 1000 × 500 quads: a million triangles.
        let mesh = grid4d_geom::primitives::torus(200.0, 50.0, 1000, 500);
        let (mut scratch, mut vertices, mut indices) = (TessellationScratch::default(), Vec::new(), Vec::new());
        let start = Instant::now();
        tessellate(
            &mesh,
            Smoothing::default(),
            None,
            false,
            &mut scratch,
            &mut vertices,
            &mut indices,
        );
        let tessellation = start.elapsed();
        let triangles = indices.len() / 3;

        let handle = MeshHandle::new(mesh);
        let start = Instant::now();
        let hit = handle.raycast(Vec3::new(200.0, 500.0, 0.0), -Vec3::Y);
        let first_pick = start.elapsed(); // builds the BVH
        let start = Instant::now();
        for i in 0..1000u16 {
            let x = 160.0 + f32::from(i) * 0.08; // across the tube (x = 150…250)
            assert!(handle.raycast(Vec3::new(x, 500.0, 0.0), -Vec3::Y).is_some());
        }
        let pick = start.elapsed() / 1000;
        println!(
            "{triangles} triangles: tessellate {tessellation:?}, first pick {first_pick:?}, then {pick:?} per pick"
        );

        assert!(triangles >= 1_000_000, "{triangles}");
        assert!((hit.unwrap().t - 450.0).abs() < 0.1, "top of the tube at y = 50");
        assert!(tessellation < Duration::from_secs(2), "{tessellation:?}");
        assert!(first_pick < Duration::from_secs(2), "{first_pick:?}");
        assert!(pick < Duration::from_millis(1), "{pick:?}");
    }
}
