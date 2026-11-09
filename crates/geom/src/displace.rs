//! Displacement at render time (see
//! `docs/reference/material-subsurface-displacement.md`): a triangulated mesh
//! split finer, its points moved along their smoothed normals, and its
//! normals found again from the moved surface.
//!
//! Corners that sit at the same place are one point of the surface: they
//! move along the same direction, so the surface stays closed wherever the
//! height read there is the same (a picture that differs across a UV seam
//! opens it there).

use std::collections::HashMap;

use glam::{Vec2, Vec3};

use crate::{TriMesh, us};

/// Splits every triangle of `mesh` in four, `levels` times: each edge's
/// middle becomes a corner. Positions, normals (renormalised), the mesh's
/// UVs and every set in `uv_sets` (one value per corner, like the mesh's)
/// are interpolated; each new triangle keeps its polygon.
pub fn subdivide_flat(mesh: &mut TriMesh, uv_sets: &mut [Vec<Vec2>], levels: u32) {
    for _ in 0..levels {
        let triangles = mesh.indices.len() / 3;
        let mut indices = Vec::with_capacity(mesh.indices.len() * 4);
        let mut faces = Vec::with_capacity(triangles * 4);
        let has_uvs = mesh.uvs.len() == mesh.positions.len();
        for (t, corners) in mesh.indices.clone().as_chunks::<3>().0.iter().enumerate() {
            let [a, b, c] = *corners;
            let mut middle = |i: u32, j: u32| -> u32 {
                let (i, j) = (us(i), us(j));
                let at = u32::try_from(mesh.positions.len()).unwrap_or(u32::MAX);
                mesh.positions.push((mesh.positions[i] + mesh.positions[j]) * 0.5);
                mesh.normals
                    .push((mesh.normals[i] + mesh.normals[j]).normalize_or(mesh.normals[i]));
                if has_uvs {
                    mesh.uvs.push((mesh.uvs[i] + mesh.uvs[j]) * 0.5);
                }
                for set in uv_sets.iter_mut() {
                    let (p, q) = (
                        set.get(i).copied().unwrap_or_default(),
                        set.get(j).copied().unwrap_or_default(),
                    );
                    set.push((p + q) * 0.5);
                }
                at
            };
            let (ab, bc, ca) = (middle(a, b), middle(b, c), middle(c, a));
            indices.extend_from_slice(&[a, ab, ca, ab, b, bc, ca, bc, c, ab, bc, ca]);
            let face = mesh.triangle_faces.get(t).copied().unwrap_or(0);
            faces.extend_from_slice(&[face; 4]);
        }
        mesh.indices = indices;
        mesh.triangle_faces = faces;
    }
}

/// The places the corners sit at: one number per corner, the same for
/// corners at the same place.
fn places(positions: &[Vec3]) -> (Vec<u32>, usize) {
    let mut seen: HashMap<[u32; 3], u32> = HashMap::with_capacity(positions.len());
    let place = positions
        .iter()
        .map(|p| {
            let key = p.to_array().map(|v| (v + 0.0).to_bits());
            let next = u32::try_from(seen.len()).unwrap_or(u32::MAX);
            *seen.entry(key).or_insert(next)
        })
        .collect();
    (place, seen.len())
}

/// The direction each corner moves: the normals of every triangle meeting at
/// its place, weighed by the triangle's angle there (so how a face was split
/// into triangles does not tilt it), so corners at one place move alike.
#[must_use]
pub fn displacement_directions(mesh: &TriMesh) -> Vec<Vec3> {
    let (place, count) = places(&mesh.positions);
    let mut sums = vec![Vec3::ZERO; count];
    for corners in mesh.indices.as_chunks::<3>().0 {
        let [a, b, c] = corners.map(us);
        let p = [mesh.positions[a], mesh.positions[b], mesh.positions[c]];
        let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
        for (i, corner) in [a, b, c].into_iter().enumerate() {
            let (here, next, last) = (p[i], p[(i + 1) % 3], p[(i + 2) % 3]);
            let angle = (next - here).angle_between(last - here);
            if angle.is_finite() {
                sums[us(place[corner])] += n * angle;
            }
        }
    }
    place
        .iter()
        .zip(&mesh.normals)
        .map(|(&p, &own)| sums[us(p)].normalize_or(own))
        .collect()
}

/// Moves each corner of `mesh` along `directions` by `offsets` (cm), then
/// finds its normals again: averaged with the triangles meeting at its place
/// whose own normals are within `smoothing` radians of its triangle's.
pub fn displace(mesh: &mut TriMesh, directions: &[Vec3], offsets: &[f32], smoothing: f32) {
    for ((p, d), o) in mesh.positions.iter_mut().zip(directions).zip(offsets) {
        *p += *d * *o;
    }
    let (place, count) = places(&mesh.positions);
    // Each place's triangles, by their face normal (area weighted).
    let face_normals: Vec<Vec3> = mesh
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|corners| {
            let [a, b, c] = corners.map(us);
            (mesh.positions[b] - mesh.positions[a]).cross(mesh.positions[c] - mesh.positions[a])
        })
        .collect();
    let mut around: Vec<Vec<u32>> = vec![Vec::new(); count];
    for (t, corners) in mesh.indices.as_chunks::<3>().0.iter().enumerate() {
        for &corner in corners {
            around[us(place[us(corner)])].push(u32::try_from(t).unwrap_or(u32::MAX));
        }
    }
    let cos = smoothing.cos();
    for (t, corners) in mesh.indices.as_chunks::<3>().0.iter().enumerate() {
        let own = face_normals[t].normalize_or_zero();
        for &corner in corners {
            let mut sum = Vec3::ZERO;
            for &other in &around[us(place[us(corner)])] {
                let n = face_normals[us(other)];
                if n.normalize_or_zero().dot(own) >= cos {
                    sum += n;
                }
            }
            mesh.normals[us(corner)] = sum.normalize_or(own);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Smoothing, Triangulator, shapes};

    fn cube() -> TriMesh {
        let mut tri = TriMesh::default();
        Triangulator::new().triangulate(&shapes::cube(2.0), Smoothing::Flat, &mut tri);
        tri
    }

    /// Edges by the places of their ends, with how many triangles use each.
    fn edge_uses(mesh: &TriMesh) -> HashMap<(u32, u32), u32> {
        let (place, _) = places(&mesh.positions);
        let mut uses = HashMap::new();
        for corners in mesh.indices.as_chunks::<3>().0 {
            for (a, b) in [(0, 1), (1, 2), (2, 0)] {
                let (p, q) = (place[us(corners[a])], place[us(corners[b])]);
                *uses.entry((p.min(q), p.max(q))).or_insert(0) += 1;
            }
        }
        uses
    }

    #[test]
    fn each_level_splits_triangles_in_four_and_keeps_the_surface() {
        let mut mesh = cube();
        let mut uvs = vec![vec![Vec2::ONE; mesh.positions.len()]];
        subdivide_flat(&mut mesh, &mut uvs, 2);
        assert_eq!(mesh.indices.len() / 3, 12 * 16);
        assert_eq!(mesh.triangle_faces.len(), 12 * 16);
        assert_eq!(uvs[0].len(), mesh.positions.len());
        // Still the cube: every point on its faces.
        assert!(
            mesh.positions
                .iter()
                .all(|p| (p.abs().max_element() - 1.0).abs() < 1e-5)
        );
        // Closed: every edge between two triangles.
        assert!(edge_uses(&mesh).values().all(|&n| n == 2));
    }

    #[test]
    fn a_displaced_closed_mesh_stays_closed() {
        let mut mesh = cube();
        subdivide_flat(&mut mesh, &mut [], 3);
        let directions = displacement_directions(&mesh);
        // Each point by where it is: corners at one place move alike.
        let offsets: Vec<f32> = mesh
            .positions
            .iter()
            .map(|p| 0.1 * (p.x * 3.0).sin() + 0.05 * p.y)
            .collect();
        displace(&mut mesh, &directions, &offsets, 60f32.to_radians());
        assert!(edge_uses(&mesh).values().all(|&n| n == 2), "no cracks");
        assert!(mesh.normals.iter().all(|n| (n.length() - 1.0).abs() < 1e-4));
        // The cube's corners moved along their diagonal.
        let corner = displacement_directions(&cube())[0];
        assert!(
            (corner.abs() - Vec3::splat(1.0 / 3f32.sqrt())).length() < 1e-4,
            "{corner}"
        );
    }

    #[test]
    fn level_zero_moves_the_points_only() {
        let mut mesh = cube();
        let before = mesh.indices.clone();
        subdivide_flat(&mut mesh, &mut [], 0);
        let directions = displacement_directions(&mesh);
        let offsets = vec![0.5; mesh.positions.len()];
        displace(&mut mesh, &directions, &offsets, 0.5);
        assert_eq!(mesh.indices, before);
        let reach = 1.0 + 0.5 / 3f32.sqrt();
        assert!(
            mesh.positions
                .iter()
                .all(|p| (p.abs().max_element() - reach).abs() < 1e-4)
        );
    }
}
