use std::f32::consts::PI;

use glam::Vec3;

use crate::{PolyMesh, us};

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
        Smoothing::Angle(angle) => angle.clamp(0.0, PI).cos(),
    };

    build_point_faces(mesh, s);
    let corner_points = mesh.corner_points();
    for f in 0..mesh.face_count() {
        let own = s.face_normals[f];
        for c in mesh.corner_range(f) {
            let p = us(corner_points[c]);
            let (start, end) = (us(s.point_face_offsets[p]), us(s.point_face_offsets[p + 1]));
            let mut sum = Vec3::ZERO;
            for &g in &s.point_face_list[start..end] {
                let g = us(g);
                if g == f || own.dot(s.face_normals[g]) >= cos_limit {
                    sum += s.face_vectors[g];
                }
            }
            out.push(sum.try_normalize().or(own.try_normalize()).unwrap_or(FALLBACK));
        }
    }
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
}
