use std::f32::consts::PI;

use glam::Vec3;

use crate::{PolyMesh, Topology, us};

/// How corner (vertex) normals are derived from face normals.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Smoothing {
    /// Every face is shaded flat.
    Flat,
    /// Normals are averaged over all faces around a point.
    Smooth,
    /// Normals are averaged only over faces whose normals differ by at most this
    /// angle (radians) from the corner's own face, so sharper edges stay hard —
    /// what Cinema 4D calls Phong angle smoothing.
    Angle(f32),
    /// As `Angle`, but only the strongest faces count: each face weighs its
    /// area × the corner's angle, and faces below half the largest weight
    /// at the point are left out — so large flat faces stay flat and only
    /// narrow bevels bend (weighted normals).
    Weighted(f32),
}

impl Smoothing {
    /// 80°: box edges stay hard, curved surfaces made of many faces look smooth.
    pub const DEFAULT_ANGLE: f32 = 80.0 * PI / 180.0;
}

impl Default for Smoothing {
    fn default() -> Self {
        Self::Angle(Self::DEFAULT_ANGLE)
    }
}

/// Buffers reused by [`corner_normals`] across calls.
#[derive(Debug, Default)]
pub(crate) struct NormalScratch {
    /// Newell vector per face (length = twice the area, so sums are area-weighted).
    face_vectors: Vec<Vec3>,
    face_normals: Vec<Vec3>,
    point_face_offsets: Vec<u32>,
    point_face_list: Vec<u32>,
    cursor: Vec<u32>,
}

/// Fallback for corners of degenerate faces: never emit a zero or NaN normal
/// (shaders normalize it).
const FALLBACK: Vec3 = Vec3::Y;

/// Writes one unit normal per corner of `mesh`, in corner order, into `out`
/// (cleared first). Allocation-free once the buffers have grown to the mesh.
pub(crate) fn corner_normals(mesh: &PolyMesh, smoothing: Smoothing, scratch: &mut NormalScratch, out: &mut Vec<Vec3>) {
    out.clear();
    let s = scratch;
    s.face_vectors.clear();
    s.face_vectors
        .extend((0..mesh.face_count()).map(|f| mesh.face_vector(f)));
    s.face_normals.clear();
    s.face_normals
        .extend(s.face_vectors.iter().map(|v| v.normalize_or_zero()));

    let cos_limit = match smoothing {
        Smoothing::Flat => {
            for (f, &n) in s.face_normals.iter().enumerate() {
                let n = if n == Vec3::ZERO { FALLBACK } else { n };
                out.extend(std::iter::repeat_n(n, mesh.corner_range(f).len()));
            }
            return;
        }
        // Below any possible cosine: every neighbour counts.
        Smoothing::Smooth => -2.0,
        Smoothing::Angle(angle) | Smoothing::Weighted(angle) => angle.clamp(0.0, PI).cos(),
    };
    let weighted = matches!(smoothing, Smoothing::Weighted(_));

    build_point_faces(mesh, s);
    let corner_points = mesh.corner_points();
    for f in 0..mesh.face_count() {
        let own = s.face_normals[f];
        for c in mesh.corner_range(f) {
            let p = us(corner_points[c]);
            let (start, end) = (us(s.point_face_offsets[p]), us(s.point_face_offsets[p + 1]));
            let mut sum = Vec3::ZERO;
            let counts = |g: usize| g == f || own.dot(s.face_normals[g]) >= cos_limit;
            if weighted {
                let weight = |g: usize| s.face_vectors[g].length() * corner_angle(mesh, g, corner_points[c]);
                let strongest = s.point_face_list[start..end]
                    .iter()
                    .map(|&g| us(g))
                    .filter(|&g| counts(g))
                    .map(weight)
                    .fold(0.0f32, f32::max);
                for &g in &s.point_face_list[start..end] {
                    let g = us(g);
                    let w = weight(g);
                    if counts(g) && w >= strongest * 0.5 {
                        sum += s.face_normals[g] * w;
                    }
                }
            } else {
                for &g in &s.point_face_list[start..end] {
                    let g = us(g);
                    if counts(g) {
                        sum += s.face_vectors[g];
                    }
                }
            }
            out.push(sum.try_normalize().or(own.try_normalize()).unwrap_or(FALLBACK));
        }
    }
}

/// The angle of face `face`'s corner at point `point` (0 if it has none).
fn corner_angle(mesh: &PolyMesh, face: usize, point: u32) -> f32 {
    let corners = mesh.face(face);
    let Some(i) = corners.iter().position(|&p| p == point) else {
        return 0.0;
    };
    let n = corners.len();
    let at = mesh.points()[us(point)];
    let (prev, next) = (
        mesh.points()[us(corners[(i + n - 1) % n])],
        mesh.points()[us(corners[(i + 1) % n])],
    );
    (prev - at).angle_between(next - at)
}

/// `mesh` with its points doubled along the `hard` edges (unique edge
/// indices of `topology`): faces on either side of a hard edge no longer
/// share its points, so smoothing never crosses it. Faces and corners keep
/// their numbers; new points follow the old ones.
#[must_use]
pub fn split_hard_edges(mesh: &PolyMesh, topology: &Topology, hard: &crate::BitSet) -> PolyMesh {
    if hard.is_empty() {
        return mesh.clone();
    }
    let mut points = mesh.points().to_vec();
    let mut corners = mesh.corner_points().to_vec();
    let mut on_hard = crate::BitSet::new();
    for e in hard.iter().filter(|&e| e < topology.edge_count()) {
        for p in topology.edges()[e] {
            on_hard.insert(us(p));
        }
    }
    let corner_face = {
        let mut list = vec![0usize; mesh.corner_count()];
        for f in 0..mesh.face_count() {
            for c in mesh.corner_range(f) {
                list[c] = f;
            }
        }
        list
    };
    let mut group: Vec<usize> = Vec::new();
    for p in on_hard.iter() {
        let faces = topology.point_faces(p);
        // Faces around `p` joined across its soft edges.
        group.clear();
        group.extend(0..faces.len());
        let find = |group: &mut Vec<usize>, mut i: usize| {
            while group[i] != i {
                group[i] = group[group[i]];
                i = group[i];
            }
            i
        };
        for &e in topology.point_edges(p) {
            let e = us(e);
            if hard.contains(e) {
                continue;
            }
            if let [f, g] = topology.edge_faces(e) {
                let (Some(i), Some(j)) = (faces.iter().position(|x| x == f), faces.iter().position(|x| x == g)) else {
                    continue;
                };
                let (a, b) = (find(&mut group, i), find(&mut group, j));
                group[a] = b;
            }
        }
        // The first group keeps the point; each other one gets a copy.
        let mut copy_of: Vec<(usize, u32)> = Vec::new();
        for (i, &f) in faces.iter().enumerate() {
            let root = find(&mut group, i);
            let target = if root == find(&mut group, 0) {
                crate::u32_index(p)
            } else if let Some(&(_, q)) = copy_of.iter().find(|(r, _)| *r == root) {
                q
            } else {
                points.push(mesh.points()[p]);
                let q = crate::u32_index(points.len() - 1);
                copy_of.push((root, q));
                q
            };
            for c in mesh.corner_range(us(f)) {
                if us(corners[c]) == p && corner_face[c] == us(f) {
                    corners[c] = target;
                }
            }
        }
    }
    let faces = (0..mesh.face_count()).map(|f| &corners[mesh.corner_range(f)]);
    let Ok(mut out) = PolyMesh::from_faces(points, faces) else {
        return mesh.clone();
    };
    if let Some(uvs) = mesh.uvs()
        && out.set_uvs(Some(uvs.to_vec())).is_err()
    {
        return mesh.clone();
    }
    out
}

/// Faces around each point, packed into the scratch buffers.
fn build_point_faces(mesh: &PolyMesh, s: &mut NormalScratch) {
    let points = mesh.point_count();
    s.point_face_offsets.clear();
    s.point_face_offsets.resize(points + 1, 0);
    for &p in mesh.corner_points() {
        s.point_face_offsets[us(p) + 1] += 1;
    }
    for i in 0..points {
        s.point_face_offsets[i + 1] += s.point_face_offsets[i];
    }
    s.cursor.clear();
    s.cursor.extend_from_slice(&s.point_face_offsets[..points]);
    s.point_face_list.clear();
    s.point_face_list.resize(us(s.point_face_offsets[points]), 0);
    for (f, face) in mesh.faces().enumerate() {
        for &p in face {
            let slot = &mut s.cursor[us(p)];
            s.point_face_list[us(*slot)] = crate::u32_index(f);
            *slot += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes;

    fn normals(mesh: &PolyMesh, smoothing: Smoothing) -> Vec<Vec3> {
        let mut out = Vec::new();
        corner_normals(mesh, smoothing, &mut NormalScratch::default(), &mut out);
        assert_eq!(out.len(), mesh.corner_count());
        assert!(out.iter().all(|n| (n.length() - 1.0).abs() < 1e-5), "unit normals only");
        out
    }

    #[test]
    fn flat_corners_take_their_face_normal() {
        let cube = shapes::cube(1.0);
        let out = normals(&cube, Smoothing::Flat);
        for f in 0..cube.face_count() {
            for c in cube.corner_range(f) {
                assert_eq!(out[c], cube.face_normal(f));
            }
        }
    }

    #[test]
    fn smooth_corners_average_all_faces() {
        let cube = shapes::cube(2.0);
        let out = normals(&cube, Smoothing::Smooth);
        for (c, &p) in cube.corner_points().iter().enumerate() {
            let diagonal = cube.points()[us(p)].normalize();
            assert!(out[c].abs_diff_eq(diagonal, 1e-5), "corner {c}: {:?}", out[c]);
        }
    }

    #[test]
    fn the_default_angle_keeps_box_edges_hard_and_curves_smooth() {
        let cube = shapes::cube(1.0);
        assert_eq!(normals(&cube, Smoothing::default()), normals(&cube, Smoothing::Flat));

        let cylinder = shapes::cylinder(12);
        let out = normals(&cylinder, Smoothing::default());
        for f in 0..12 {
            for c in cylinder.corner_range(f) {
                let p = cylinder.points()[us(cylinder.corner_points()[c])];
                let radial = Vec3::new(p.x, 0.0, p.z).normalize();
                assert!(
                    out[c].abs_diff_eq(radial, 1e-5),
                    "side corners are smooth and horizontal"
                );
            }
        }
        for c in cylinder.corner_range(12) {
            assert!(out[c].abs_diff_eq(Vec3::Y, 1e-6), "cap corners stay flat");
        }
        let smooth = normals(&cylinder, Smoothing::Smooth);
        assert!(
            smooth[cylinder.corner_range(12).start].y < 0.99,
            "Smooth also blends the caps in"
        );
    }

    #[test]
    fn coplanar_faces_share_normals_even_at_zero_angle() {
        let grid = shapes::grid(3, 3);
        assert!(
            normals(&grid, Smoothing::Angle(0.0))
                .iter()
                .all(|n| n.abs_diff_eq(Vec3::Y, 1e-6))
        );
    }

    #[test]
    fn degenerate_faces_never_produce_nan() {
        let line = shapes::collinear();
        for smoothing in [Smoothing::Flat, Smoothing::Smooth, Smoothing::default()] {
            assert!(normals(&line, smoothing).iter().all(|n| *n == FALLBACK));
        }
    }

    #[test]
    fn weighted_normals_keep_big_faces_flat() {
        // A box with one bevelled edge: a narrow strip between two big faces.
        let mesh = PolyMesh::from_faces(
            vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(10.0, 0.0, 0.0),
                Vec3::new(10.0, 9.0, 0.0),
                Vec3::new(0.0, 9.0, 0.0),
                Vec3::new(10.0, 10.0, -1.0),
                Vec3::new(0.0, 10.0, -1.0),
                Vec3::new(10.0, 10.0, -10.0),
                Vec3::new(0.0, 10.0, -10.0),
            ],
            [vec![0u32, 1, 2, 3], vec![3, 2, 4, 5], vec![5, 4, 6, 7]],
        )
        .unwrap();
        let angle = std::f32::consts::FRAC_PI_2;
        let plain = normals(&mesh, Smoothing::Angle(angle));
        let weighted = normals(&mesh, Smoothing::Weighted(angle));
        for f in [0, 2] {
            for c in mesh.corner_range(f) {
                let face = mesh.face_normal(f);
                assert!(
                    weighted[c].angle_between(face) < 1f32.to_radians(),
                    "the big face stays flat"
                );
            }
        }
        assert!(plain[mesh.corner_range(0).start + 2].angle_between(mesh.face_normal(0)) > 5f32.to_radians());
    }

    #[test]
    fn hard_edges_split_the_shading_and_keep_the_numbers() {
        let cube = shapes::cube(2.0);
        let topology = Topology::build(&cube);
        // The loop of edges round face 0: its corners part from the sides.
        let hard: crate::BitSet = cube
            .corner_range(0)
            .filter_map(|c| topology.corner_edge(c).map(us))
            .collect();
        let split = split_hard_edges(&cube, &topology, &hard);
        assert_eq!(split.face_count(), cube.face_count());
        assert_eq!(
            split.point_count(),
            cube.point_count() + 4,
            "the loop's four points doubled"
        );
        for f in 0..cube.face_count() {
            for (c, &p) in cube.corner_range(f).zip(cube.face(f)) {
                assert_eq!(split.points()[us(split.corner_points()[c])], cube.points()[us(p)]);
            }
        }
        let smooth = normals(&split, Smoothing::Smooth);
        let a = cube.face(0)[0];
        let at_a: Vec<Vec3> = (0..cube.corner_count())
            .filter(|&c| cube.corner_points()[c] == a)
            .map(|c| smooth[c])
            .collect();
        assert!(
            at_a.iter().any(|n| n.angle_between(at_a[0]) > 0.1),
            "split along the hard edge"
        );
    }
}
