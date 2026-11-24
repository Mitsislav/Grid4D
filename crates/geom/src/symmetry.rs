//! Symmetry: a mesh and its mirror image across one of three planes through
//! the origin, welded where they meet (spec
//! `docs/reference/generator-symmetry-instance-join.md`).

use glam::Vec2;

use crate::{PolyMesh, u32_index, us};

/// The plane a [`Symmetry`] mirrors across (in its own axes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MirrorPlane {
    /// Mirrors front and back (Z turned round).
    Xy,
    /// Mirrors left and right (X turned round).
    #[default]
    Zy,
    /// Mirrors up and down (Y turned round).
    Xz,
}

impl MirrorPlane {
    pub const ALL: [Self; 3] = [Self::Xy, Self::Zy, Self::Xz];

    /// The axis the plane turns round (0 = X, 1 = Y, 2 = Z).
    #[must_use]
    pub const fn axis(self) -> usize {
        match self {
            Self::Xy => 2,
            Self::Zy => 0,
            Self::Xz => 1,
        }
    }
}

/// A Symmetry generator's settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Symmetry {
    pub plane: MirrorPlane,
    /// Points of the half and its mirror that meet at the plane become one.
    pub weld: bool,
    /// How near the plane a point is welded (and held on it), ≥ 0.
    pub tolerance: f32,
    /// Points within the tolerance are moved onto the plane first.
    pub hold: bool,
    /// The mirror image comes first (which half is the copy).
    pub flip: bool,
}

impl Symmetry {
    /// The largest tolerance accepted (cm).
    pub const MAX_TOLERANCE: f32 = 100.0;
}

impl Default for Symmetry {
    /// Left and right, welded and held within 0.01 cm.
    fn default() -> Self {
        Self {
            plane: MirrorPlane::Zy,
            weld: true,
            tolerance: 0.01,
            hold: true,
            flip: false,
        }
    }
}

/// `mesh` and its mirror image across `symmetry`'s plane: the mirror's faces
/// wound the other way (so both face out) and its pictures mirrored back
/// (U → 1 − U), points on the plane shared when welding, faces lying in the
/// plane not doubled.
#[must_use]
pub fn mirror(mesh: &PolyMesh, symmetry: &Symmetry) -> PolyMesh {
    let axis = symmetry.plane.axis();
    let tolerance = symmetry.tolerance.clamp(0.0, Symmetry::MAX_TOLERANCE);
    let mut points = mesh.points().to_vec();
    if symmetry.hold {
        for p in &mut points {
            if p[axis].abs() <= tolerance {
                p[axis] = 0.0;
            }
        }
    }
    let on_plane: Vec<bool> = points.iter().map(|p| p[axis].abs() <= tolerance).collect();
    let in_plane = |f: usize| mesh.face(f).iter().all(|&p| on_plane[us(p)]);
    // The mirror point of each point: itself when welded at the plane, else
    // a new point (numbered after the half's, only if a face uses it).
    let count = u32_index(points.len());
    let mut mirrored = vec![u32::MAX; points.len()];
    let mut next = count;
    for f in (0..mesh.face_count()).filter(|&f| !in_plane(f)) {
        for &p in mesh.face(f) {
            let slot = &mut mirrored[us(p)];
            if *slot == u32::MAX {
                *slot = if symmetry.weld && on_plane[us(p)] {
                    p
                } else {
                    next += 1;
                    next - 1
                };
            }
        }
    }
    let mut out = PolyMesh::new();
    // The half's points keep their numbers (its selections still name them).
    for &p in &points {
        if out.add_point(p).is_err() {
            return PolyMesh::new();
        }
    }
    let mut reflected = vec![glam::Vec3::ZERO; us(next - count)];
    for (p, &m) in mirrored.iter().enumerate() {
        if m != u32::MAX && m >= count {
            let mut q = points[p];
            q[axis] = -q[axis];
            reflected[us(m - count)] = q;
        }
    }
    for q in reflected {
        if out.add_point(q).is_err() {
            return PolyMesh::new();
        }
    }
    let uvs = mesh.uvs();
    let mut corner_uvs: Vec<Vec2> = Vec::with_capacity(mesh.corner_count() * 2);
    let mut corners: Vec<u32> = Vec::new();
    let mut add = |out: &mut PolyMesh, f: usize, mirror: bool| -> bool {
        let range = mesh.corner_range(f);
        corners.clear();
        if mirror {
            // Wound the other way, its pictures mirrored back.
            corners.extend(mesh.face(f).iter().rev().map(|&p| mirrored[us(p)]));
            corner_uvs.extend(range.rev().map(|c| {
                let uv = uvs.map_or(Vec2::ZERO, |u| u[c]);
                Vec2::new(1.0 - uv.x, uv.y)
            }));
        } else {
            corners.extend_from_slice(mesh.face(f));
            corner_uvs.extend(range.map(|c| uvs.map_or(Vec2::ZERO, |u| u[c])));
        }
        out.add_face(&corners).is_ok()
    };
    let mirror_faces = || (0..mesh.face_count()).filter(|&f| !in_plane(f));
    let mut ok = true;
    if symmetry.flip {
        for f in mirror_faces() {
            ok &= add(&mut out, f, true);
        }
        for f in 0..mesh.face_count() {
            ok &= add(&mut out, f, false);
        }
    } else {
        for f in 0..mesh.face_count() {
            ok &= add(&mut out, f, false);
        }
        for f in mirror_faces() {
            ok &= add(&mut out, f, true);
        }
    }
    if ok && uvs.is_some() && out.corner_count() == corner_uvs.len() {
        let _ = out.set_uvs(Some(corner_uvs));
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test code")]

    use glam::Vec3;

    use super::*;
    use crate::{Topology, primitives, uv};

    /// The half of a box with x ≥ 0: open at the plane x = 0.
    fn half_box() -> PolyMesh {
        let cube = primitives::cube(Vec3::new(2.0, 2.0, 2.0), [2, 1, 1]);
        cube.with_faces_where(|f| cube.face_center(f).x > 0.0)
    }

    /// Whether some edge has a face on one side only.
    fn is_open(mesh: &PolyMesh) -> bool {
        let topology = Topology::build(mesh);
        (0..topology.edges().len()).any(|e| topology.edge_faces(e).len() == 1)
    }

    #[test]
    fn a_half_box_mirrored_and_welded_is_a_closed_box() {
        let half = half_box();
        let whole = mirror(&half, &Symmetry::default());
        assert_eq!(whole.face_count(), 2 * half.face_count());
        let topology = Topology::build(&whole);
        assert!(topology.is_manifold(), "every edge between two faces, wound alike");
        assert!((0..topology.edges().len()).all(|e| topology.edge_faces(e).len() == 2));
        // Points on the plane are shared: no point used twice.
        let mut used: Vec<u32> = whole.corner_points().to_vec();
        used.sort_unstable();
        used.dedup();
        let mut seen: Vec<[u32; 3]> = used
            .iter()
            .map(|&p| whole.points()[us(p)].to_array().map(f32::to_bits))
            .collect();
        seen.sort_unstable();
        assert!(seen.windows(2).all(|w| w[0] != w[1]));
        // The half's points come first, unchanged.
        assert_eq!(&whole.points()[..half.point_count()], half.points());
        assert_eq!(uv::flipped_faces(&whole), 0, "no mirrored pictures");
    }

    #[test]
    fn points_near_the_plane_are_held_on_it_and_welded() {
        let mut half = half_box();
        for p in half.points_mut() {
            if p.x.abs() < 1e-6 {
                p.x = 0.004;
            }
        }
        let whole = mirror(&half, &Symmetry::default());
        assert!(Topology::build(&whole).is_manifold());
        assert!(whole.points().iter().all(|p| p.x.abs() < 1e-6 || p.x.abs() > 0.5));
        // Neither held nor welded: two halves apart, a gap of 0.008.
        let apart = mirror(
            &half,
            &Symmetry {
                weld: false,
                hold: false,
                ..Symmetry::default()
            },
        );
        assert!(apart.point_count() > whole.point_count(), "nothing shared");
        assert!(is_open(&apart), "a gap at the plane");
        assert!(!is_open(&whole));
    }

    #[test]
    fn faces_in_the_plane_are_not_doubled_and_every_plane_works() {
        // A square lying in the plane it is mirrored across: itself.
        let square = primitives::plane(2.0, 2.0, 1, 1);
        let flat = mirror(
            &square,
            &Symmetry {
                plane: MirrorPlane::Xz,
                ..Symmetry::default()
            },
        );
        assert_eq!(flat.face_count(), 1);
        for plane in MirrorPlane::ALL {
            let mut moved = primitives::sphere(1.0, 8);
            for p in moved.points_mut() {
                p[plane.axis()] += 3.0;
            }
            let both = mirror(
                &moved,
                &Symmetry {
                    plane,
                    ..Symmetry::default()
                },
            );
            assert_eq!(both.face_count(), 2 * moved.face_count(), "{plane:?}");
            let lowest = both
                .points()
                .iter()
                .map(|p| p[plane.axis()])
                .fold(f32::INFINITY, f32::min);
            assert!((lowest + 4.0).abs() < 1e-5, "{plane:?}");
        }
    }
}
