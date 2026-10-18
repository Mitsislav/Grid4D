//! Parametric primitives: the geometry behind the editor's Cube, Sphere,
//! Cylinder, Plane, Cone, Torus, Disc and Capsule objects.
//!
//! Every primitive is centred on the origin with its axis along +Y (like Cinema
//! 4D's default orientation), sizes in scene units, faces counter-clockwise from
//! outside. Segment counts are clamped to a sane range so a typo cannot ask for
//! a billion polygons. Caps are single n-gons (cheap, and good input for later
//! modelling tools) with their own ring of points, not shared with the side —
//! as in the reference application, so a rim stays hard under smoothing and
//! subdivision (see `docs/reference/modeling-modes.md` §2).

use std::{collections::HashMap, f32::consts::PI};

use glam::Vec3;

use crate::{PolyMesh, u32_index, us};

/// Upper limit for any segment count.
pub const MAX_SEGMENTS: u32 = 1000;
/// Radii below this count as zero (a cone's apex, a disc without hole).
const EPSILON: f32 = 1e-6;

fn segments(n: u32, min: u32) -> u32 {
    n.clamp(min, MAX_SEGMENTS)
}

fn fraction(i: u32, n: u32) -> f32 {
    // Exact for every segment count up to MAX_SEGMENTS.
    f32::from(u16::try_from(i).unwrap_or(u16::MAX)) / f32::from(u16::try_from(n.max(1)).unwrap_or(u16::MAX))
}

/// Collects points and faces, then builds a validated mesh.
#[derive(Default)]
struct Builder {
    points: Vec<Vec3>,
    corners: Vec<u32>,
    sizes: Vec<usize>,
}

impl Builder {
    fn point(&mut self, p: Vec3) -> u32 {
        self.points.push(p);
        u32_index(self.points.len() - 1)
    }

    fn face(&mut self, corners: &[u32]) {
        self.corners.extend_from_slice(corners);
        self.sizes.push(corners.len());
    }

    fn build(self) -> PolyMesh {
        let Self { points, corners, sizes } = self;
        let mut start = 0;
        let faces = sizes.iter().map(|&n| {
            let face = &corners[start..start + n];
            start += n;
            face
        });
        // Generators only emit valid faces (covered by the tests); an empty mesh
        // is the safe answer should that ever break.
        PolyMesh::from_faces(points, faces).unwrap_or_default()
    }
}

/// How a lathed surface ends at the top or bottom of its profile.
#[derive(Clone, Copy)]
enum End {
    /// Converges to a single point at this height (sphere pole, cone apex).
    Pole(f32),
    /// Closed by one n-gon on its own copy of the ring's points (cylinder cap).
    Cap,
}

/// Revolves `profile` — rings of `(radius, y)`, top to bottom — around +Y with
/// `around` segments. A ring's point `j` sits at angle `2π·j/around` measured from
/// +Z towards +X.
fn lathe(profile: &[(f32, f32)], around: u32, top: End, bottom: End) -> PolyMesh {
    let mut b = Builder::default();
    let n = around;
    let rings: Vec<u32> = profile
        .iter()
        .map(|&(radius, y)| {
            let first = u32_index(b.points.len());
            for j in 0..n {
                let a = 2.0 * PI * fraction(j, n);
                b.point(Vec3::new(radius * a.sin(), y, radius * a.cos()));
            }
            first
        })
        .collect();
    let at = |ring: u32, j: u32| ring + j % n;

    for pair in rings.windows(2) {
        let (upper, lower) = (pair[0], pair[1]);
        for j in 0..n {
            b.face(&[at(lower, j), at(lower, j + 1), at(upper, j + 1), at(upper, j)]);
        }
    }
    let (Some(&first), Some(&last)) = (rings.first(), rings.last()) else {
        return b.build();
    };
    // A cap's points: a copy of the ring, so the cap is separate from the side.
    let cap_ring = |b: &mut Builder, ring: u32| {
        let copy = u32_index(b.points.len());
        for j in 0..n {
            let p = b.points[us(at(ring, j))];
            b.point(p);
        }
        copy
    };
    match top {
        End::Pole(y) => {
            let pole = b.point(Vec3::new(0.0, y, 0.0));
            for j in 0..n {
                b.face(&[at(first, j), at(first, j + 1), pole]);
            }
        }
        End::Cap => {
            let ring = cap_ring(&mut b, first);
            let cap: Vec<u32> = (0..n).map(|j| at(ring, j)).collect();
            b.face(&cap);
        }
    }
    match bottom {
        End::Pole(y) => {
            let pole = b.point(Vec3::new(0.0, y, 0.0));
            for j in 0..n {
                b.face(&[at(last, j), pole, at(last, j + 1)]);
            }
        }
        End::Cap => {
            let ring = cap_ring(&mut b, last);
            let cap: Vec<u32> = (0..n).rev().map(|j| at(ring, j)).collect();
            b.face(&cap);
        }
    }
    b.build()
}

/// Box of `size` with `segments` subdivisions per axis (X, Y, Z).
#[must_use]
pub fn cube(size: Vec3, segments_xyz: [u32; 3]) -> PolyMesh {
    let n = segments_xyz.map(|s| segments(s, 1));
    let mut b = Builder::default();
    // Points on the surface are lattice coordinates with at least one coordinate on
    // the border; faces sharing an edge reuse the same point.
    let mut index: HashMap<[u32; 3], u32> = HashMap::new();
    let mut point = |b: &mut Builder, c: [u32; 3]| {
        *index.entry(c).or_insert_with(|| {
            let p = Vec3::new(
                (fraction(c[0], n[0]) - 0.5) * size.x,
                (fraction(c[1], n[1]) - 0.5) * size.y,
                (fraction(c[2], n[2]) - 0.5) * size.z,
            );
            b.point(p)
        })
    };
    // (fixed axis, at the max side?, u axis, v axis) with u × v = outward normal.
    const FACES: [(usize, bool, usize, usize); 6] = [
        (0, true, 1, 2),  // +X: Y × Z
        (0, false, 2, 1), // −X: Z × Y
        (1, true, 2, 0),  // +Y: Z × X
        (1, false, 0, 2), // −Y: X × Z
        (2, true, 0, 1),  // +Z: X × Y
        (2, false, 1, 0), // −Z: Y × X
    ];
    for (axis, max_side, u, v) in FACES {
        for i in 0..n[u] {
            for j in 0..n[v] {
                let corner = |du: u32, dv: u32| {
                    let mut c = [0u32; 3];
                    c[axis] = if max_side { n[axis] } else { 0 };
                    c[u] = i + du;
                    c[v] = j + dv;
                    c
                };
                let quad = [corner(0, 0), corner(1, 0), corner(1, 1), corner(0, 1)].map(|c| point(&mut b, c));
                b.face(&quad);
            }
        }
    }
    b.build()
}

/// UV sphere with `segments` around the axis and half as many rings.
#[must_use]
pub fn sphere(radius: f32, segments_around: u32) -> PolyMesh {
    let around = segments(segments_around, 3);
    let rings = (around / 2).max(2);
    let profile: Vec<(f32, f32)> = (1..rings)
        .map(|k| {
            let theta = PI * fraction(k, rings);
            (radius * theta.sin(), radius * theta.cos())
        })
        .collect();
    lathe(&profile, around, End::Pole(radius), End::Pole(-radius))
}

/// Capped cylinder.
#[must_use]
pub fn cylinder(radius: f32, height: f32, rotation_segments: u32, height_segments: u32) -> PolyMesh {
    cone(radius, radius, height, rotation_segments, height_segments)
}

/// Frustum from `bottom_radius` to `top_radius`; a zero radius ends in an apex.
/// Empty if both radii are zero.
#[must_use]
pub fn cone(
    bottom_radius: f32,
    top_radius: f32,
    height: f32,
    rotation_segments: u32,
    height_segments: u32,
) -> PolyMesh {
    let around = segments(rotation_segments, 3);
    let rows = segments(height_segments, 1);
    let (rb, rt) = (bottom_radius.max(0.0), top_radius.max(0.0));
    if rb < EPSILON && rt < EPSILON {
        return PolyMesh::new();
    }
    let h = height * 0.5;
    // Rings top to bottom; an end with zero radius becomes a pole instead of a ring.
    let first = if rt < EPSILON { 1 } else { 0 };
    let last = if rb < EPSILON { rows - 1 } else { rows };
    let profile: Vec<(f32, f32)> = (first..=last)
        .map(|k| {
            let t = fraction(k, rows);
            (rt + (rb - rt) * t, h - height * t)
        })
        .collect();
    let top = if rt < EPSILON { End::Pole(h) } else { End::Cap };
    let bottom = if rb < EPSILON { End::Pole(-h) } else { End::Cap };
    lathe(&profile, around, top, bottom)
}

/// Grid of `width` (X) × `depth` (Z) on the XZ plane, facing +Y.
#[must_use]
pub fn plane(width: f32, depth: f32, width_segments: u32, depth_segments: u32) -> PolyMesh {
    let (nx, nz) = (segments(width_segments, 1), segments(depth_segments, 1));
    let mut b = Builder::default();
    for j in 0..=nz {
        for i in 0..=nx {
            b.point(Vec3::new(
                (fraction(i, nx) - 0.5) * width,
                0.0,
                (fraction(j, nz) - 0.5) * depth,
            ));
        }
    }
    let p = |i: u32, j: u32| j * (nx + 1) + i;
    for j in 0..nz {
        for i in 0..nx {
            // +Z then +X: Z × X = +Y.
            b.face(&[p(i, j), p(i, j + 1), p(i + 1, j + 1), p(i + 1, j)]);
        }
    }
    b.build()
}

/// Ring torus around +Y: `ring_radius` to the tube's centre, `pipe_radius` of the tube.
#[must_use]
pub fn torus(ring_radius: f32, pipe_radius: f32, ring_segments: u32, pipe_segments: u32) -> PolyMesh {
    let (n, m) = (segments(ring_segments, 3), segments(pipe_segments, 3));
    let mut b = Builder::default();
    for i in 0..n {
        let u = 2.0 * PI * fraction(i, n);
        for j in 0..m {
            let v = 2.0 * PI * fraction(j, m);
            let r = ring_radius + pipe_radius * v.cos();
            b.point(Vec3::new(r * u.cos(), pipe_radius * v.sin(), r * u.sin()));
        }
    }
    let p = |i: u32, j: u32| (i % n) * m + j % m;
    for i in 0..n {
        for j in 0..m {
            // Around the tube, then around the axis: outward.
            b.face(&[p(i, j), p(i, j + 1), p(i + 1, j + 1), p(i + 1, j)]);
        }
    }
    b.build()
}

/// Flat ring (or disc, with a zero `inner_radius`) on the XZ plane, facing +Y.
#[must_use]
pub fn disc(inner_radius: f32, outer_radius: f32, rotation_segments: u32, disc_segments: u32) -> PolyMesh {
    let (n, m) = (segments(rotation_segments, 3), segments(disc_segments, 1));
    let (ri, ro) = (inner_radius.max(0.0), outer_radius.max(0.0));
    let mut b = Builder::default();
    let solid = ri < EPSILON;
    let center = solid.then(|| b.point(Vec3::ZERO));
    // Rings from the inside out; a solid disc starts at its first ring off-centre.
    let rings: Vec<u32> = (u32::from(solid)..=m)
        .map(|k| {
            let r = ri + (ro - ri) * fraction(k, m);
            let first = u32_index(b.points.len());
            for j in 0..n {
                let a = 2.0 * PI * fraction(j, n);
                b.point(Vec3::new(r * a.sin(), 0.0, r * a.cos()));
            }
            first
        })
        .collect();
    let at = |ring: u32, j: u32| ring + j % n;
    if let (Some(c), Some(&first)) = (center, rings.first()) {
        for j in 0..n {
            b.face(&[c, at(first, j), at(first, j + 1)]);
        }
    }
    for pair in rings.windows(2) {
        let (inner, outer) = (pair[0], pair[1]);
        for j in 0..n {
            // Outwards (+Z at angle 0), then around (+X): Z × X = +Y.
            b.face(&[at(inner, j), at(outer, j), at(outer, j + 1), at(inner, j + 1)]);
        }
    }
    b.build()
}

/// Cylinder with hemispherical ends; `height` is the total height, at least `2·radius`.
#[must_use]
pub fn capsule(radius: f32, height: f32, rotation_segments: u32, height_segments: u32, cap_segments: u32) -> PolyMesh {
    let around = segments(rotation_segments, 3);
    let rows = segments(height_segments, 1);
    let caps = segments(cap_segments, 1);
    let r = radius.max(0.0);
    let straight = (height - 2.0 * r).max(0.0);
    let half = straight * 0.5;
    let mut profile = Vec::new();
    // Top hemisphere, pole excluded, down to its equator.
    for k in 1..=caps {
        let theta = 0.5 * PI * fraction(k, caps);
        profile.push((r * theta.sin(), half + r * theta.cos()));
    }
    // Straight part between the two equators (no zero-height rows when it is empty).
    if straight > 0.0 {
        for k in 1..=rows {
            profile.push((r, half - straight * fraction(k, rows)));
        }
    }
    // Bottom hemisphere from just below its equator, pole excluded.
    for k in 1..caps {
        let theta = 0.5 * PI + 0.5 * PI * fraction(k, caps);
        profile.push((r * theta.sin(), -half + r * theta.cos()));
    }
    lathe(&profile, around, End::Pole(half + r), End::Pole(-half - r))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Topology;

    /// (closed, Euler characteristic) plus manifold/orientation checks.
    fn shape(mesh: &PolyMesh) -> (bool, i64) {
        let t = Topology::build(mesh);
        assert!(
            t.is_manifold(),
            "every primitive must be manifold and consistently wound"
        );
        assert!(mesh.face_count() > 0);
        (t.is_closed(), t.euler_characteristic())
    }

    /// Every face points away from the origin (for shapes that are star-shaped around it).
    fn assert_outward(name: &str, mesh: &PolyMesh) {
        for f in 0..mesh.face_count() {
            let out = mesh.face_vector(f).dot(mesh.face_center(f));
            assert!(out > 0.0, "{name}: face {f} points inwards");
        }
    }

    /// Merges points at bit-identical positions — what Optimize ▸ Points (with
    /// zero tolerance) will do; enough to check that separate caps fit exactly.
    fn welded(mesh: &PolyMesh) -> PolyMesh {
        let mut index: HashMap<[u32; 3], u32> = HashMap::new();
        let mut points = Vec::new();
        let remap: Vec<u32> = mesh
            .points()
            .iter()
            .map(|p| {
                *index.entry(p.to_array().map(f32::to_bits)).or_insert_with(|| {
                    points.push(*p);
                    u32_index(points.len() - 1)
                })
            })
            .collect();
        let faces = mesh
            .faces()
            .map(|f| f.iter().map(|&p| remap[us(p)]).collect::<Vec<_>>());
        PolyMesh::from_faces(points, faces).unwrap()
    }

    fn assert_bounds(name: &str, mesh: &PolyMesh, half: Vec3) {
        let (min, max) = mesh.bounds().unwrap();
        assert!(
            min.abs_diff_eq(-half, 1e-3) && max.abs_diff_eq(half, 1e-3),
            "{name}: {min:?} {max:?}"
        );
    }

    #[test]
    fn closed_primitives_are_watertight_spheres() {
        let cases = [
            ("cube", cube(Vec3::new(200.0, 100.0, 50.0), [2, 3, 4])),
            ("sphere", sphere(100.0, 24)),
            ("capsule", capsule(50.0, 200.0, 36, 4, 8)),
            ("short capsule", capsule(50.0, 40.0, 12, 4, 3)),
        ];
        for (name, mesh) in cases {
            assert_eq!(shape(&mesh), (true, 2), "{name}");
            assert_outward(name, &mesh);
        }
        assert_eq!(shape(&torus(200.0, 50.0, 36, 18)), (true, 0), "a torus has genus 1");
    }

    /// Caps are separate pieces (each rim exists twice), and merging the
    /// duplicates closes the shape exactly.
    #[test]
    fn caps_have_their_own_points_and_fit_the_side_exactly() {
        let cases = [
            ("cylinder", cylinder(50.0, 200.0, 36, 4), 2 * 36),
            ("cone", cone(100.0, 0.0, 200.0, 36, 8), 36),
            ("inverted cone", cone(0.0, 40.0, 100.0, 12, 3), 12),
            ("frustum", cone(100.0, 30.0, 80.0, 16, 2), 2 * 16),
        ];
        for (name, mesh, rim_points) in cases {
            assert_eq!(shape(&mesh), (false, 2), "{name}: side and caps are open pieces");
            assert_outward(name, &mesh);
            let closed = welded(&mesh);
            assert_eq!(mesh.point_count() - closed.point_count(), rim_points, "{name}");
            assert_eq!(shape(&closed), (true, 2), "{name}: merged, it is watertight");
        }
    }

    #[test]
    fn flat_primitives_are_discs_facing_up() {
        for (name, mesh, euler) in [
            ("plane", plane(400.0, 300.0, 20, 10), 1),
            ("disc", disc(0.0, 100.0, 36, 2), 1),
            ("ring", disc(40.0, 100.0, 36, 3), 0),
        ] {
            assert_eq!(shape(&mesh), (false, euler), "{name}");
            for f in 0..mesh.face_count() {
                assert!(mesh.face_normal(f).abs_diff_eq(Vec3::Y, 1e-5), "{name}: face {f}");
            }
        }
    }

    #[test]
    fn sizes_and_counts_match_the_parameters() {
        let c = cube(Vec3::new(200.0, 100.0, 50.0), [2, 3, 4]);
        assert_bounds("cube", &c, Vec3::new(100.0, 50.0, 25.0));
        assert_eq!(c.face_count(), 2 * (3 * 4 + 2 * 4 + 2 * 3));
        assert_eq!(c.point_count(), 3 * 4 * 5 - 2 * 3, "border lattice points, each once");

        let s = sphere(100.0, 24);
        assert_eq!((s.point_count(), s.face_count()), (24 * 11 + 2, 24 * 12));
        assert!(s.points().iter().all(|p| (p.length() - 100.0).abs() < 1e-3));

        assert_bounds("cylinder", &cylinder(50.0, 200.0, 36, 4), Vec3::new(50.0, 100.0, 50.0));
        let cyl = cylinder(50.0, 200.0, 36, 4);
        assert_eq!(cyl.face_count(), 36 * 4 + 2);
        assert_eq!(cyl.point_count(), 36 * 5 + 2 * 36, "each cap has its own ring");
        assert!(cyl.faces().any(|f| f.len() == 36), "caps are single n-gons");

        let p = plane(400.0, 300.0, 20, 10);
        assert_eq!((p.point_count(), p.face_count()), (21 * 11, 200));
        assert_bounds("plane", &p, Vec3::new(200.0, 0.0, 150.0));

        let cone_mesh = cone(100.0, 0.0, 200.0, 36, 8);
        assert_eq!(
            cone_mesh.point_count(),
            36 * 8 + 1 + 36,
            "a single apex point; the base cap has its own ring"
        );
        assert_bounds("cone", &cone_mesh, Vec3::new(100.0, 100.0, 100.0));

        // 16 pipe segments sample the top of the tube exactly (90° = 4/16).
        assert_bounds("torus", &torus(200.0, 50.0, 36, 16), Vec3::new(250.0, 50.0, 250.0));
        assert_bounds("capsule", &capsule(50.0, 200.0, 36, 4, 8), Vec3::new(50.0, 100.0, 50.0));
        assert_bounds("short capsule", &capsule(50.0, 40.0, 36, 4, 8), Vec3::splat(50.0));
        assert_bounds("disc", &disc(0.0, 100.0, 36, 2), Vec3::new(100.0, 0.0, 100.0));
    }

    #[test]
    fn segment_counts_are_clamped() {
        // Zero segments still give a valid minimal shape…
        assert_eq!(shape(&sphere(1.0, 0)), (true, 2));
        assert_eq!(shape(&cube(Vec3::ONE, [0, 0, 0])), (true, 2));
        assert_eq!(cube(Vec3::ONE, [0, 0, 0]).face_count(), 6);
        assert_eq!(shape(&torus(2.0, 0.5, 0, 0)), (true, 0));
        // …and absurd counts are capped.
        assert_eq!(
            plane(1.0, 1.0, 5000, 1).face_count(),
            usize::try_from(MAX_SEGMENTS).unwrap()
        );
    }

    #[test]
    fn a_cone_without_radius_is_empty() {
        assert_eq!(cone(0.0, 0.0, 100.0, 12, 1).face_count(), 0);
    }

    #[test]
    fn primitives_triangulate_cleanly() {
        let mut out = crate::TriMesh::default();
        let mut triangulator = crate::Triangulator::new();
        for mesh in [
            sphere(1.0, 16),
            cylinder(1.0, 2.0, 16, 2),
            cone(1.0, 0.0, 1.0, 16, 2),
            torus(2.0, 0.5, 16, 8),
            disc(0.0, 1.0, 16, 2),
            capsule(1.0, 3.0, 16, 2, 4),
        ] {
            triangulator.triangulate(&mesh, crate::Smoothing::default(), &mut out);
            let expected: usize = mesh.faces().map(|f| f.len() - 2).sum();
            assert_eq!(out.triangle_count(), expected);
        }
    }
}
