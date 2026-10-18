use glam::{Vec2, Vec3};

use crate::{
    PolyMesh, Smoothing,
    mesh::newell,
    normals::{NormalScratch, corner_normals},
    u32_index, us,
};

/// Triangles ready for the GPU, with one vertex per polygon corner (so a corner
/// can carry its own normal and UV at hard edges and seams).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TriMesh {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    /// Empty when the mesh has no UVs.
    pub uvs: Vec<Vec2>,
    /// Three vertex indices per triangle, counter-clockwise from the front.
    pub indices: Vec<u32>,
    /// The polygon each triangle came from (picking, selection display).
    pub triangle_faces: Vec<u32>,
}

impl TriMesh {
    #[must_use]
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    #[must_use]
    pub fn triangle_count(&self) -> usize {
        self.triangle_faces.len()
    }

    /// Empties every array, keeping their capacity.
    pub fn clear(&mut self) {
        self.positions.clear();
        self.normals.clear();
        self.uvs.clear();
        self.indices.clear();
        self.triangle_faces.clear();
    }
}

/// Turns [`PolyMesh`]es into [`TriMesh`]es. Keep one around: its buffers (and
/// the output's) are reused, so re-triangulating a mesh of the same size does not
/// allocate.
#[derive(Debug, Default)]
pub struct Triangulator {
    normals: NormalScratch,
    /// The current face projected onto its plane.
    polygon: Vec<Vec2>,
    /// Local corners of the current face not yet clipped off.
    remaining: Vec<u32>,
    /// Triangles of the current face, in local corner indices.
    local: Vec<[u32; 3]>,
}

impl Triangulator {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Triangulates every face of `mesh` into `out` (cleared first): a face with
    /// `n` corners becomes `n − 2` triangles.
    pub fn triangulate(&mut self, mesh: &PolyMesh, smoothing: Smoothing, out: &mut TriMesh) {
        out.clear();
        corner_normals(mesh, smoothing, &mut self.normals, &mut out.normals);
        out.positions
            .extend(mesh.corner_points().iter().map(|&p| mesh.points()[us(p)]));
        if let Some(uvs) = mesh.uvs() {
            out.uvs.extend_from_slice(uvs);
        }
        for f in 0..mesh.face_count() {
            let range = mesh.corner_range(f);
            self.triangulate_face(&out.positions[range.clone()]);
            let base = u32_index(range.start);
            let face = u32_index(f);
            for [a, b, c] in self.local.drain(..) {
                out.indices.extend_from_slice(&[base + a, base + b, base + c]);
                out.triangle_faces.push(face);
            }
        }
    }

    /// Fills `self.local` with the triangles of one polygon.
    fn triangulate_face(&mut self, corners: &[Vec3]) {
        self.local.clear();
        let n = corners.len();
        if n == 3 {
            self.local.push([0, 1, 2]);
            return;
        }
        let normal = newell(corners.iter().copied()).normalize_or_zero();
        if normal == Vec3::ZERO {
            fan(u32_index(n), &mut self.local); // zero area: any split is as good
            return;
        }
        // A basis of the face's plane with u × v = normal, so the polygon stays
        // counter-clockwise in 2D.
        let u = normal.any_orthonormal_vector();
        let v = normal.cross(u);
        self.polygon.clear();
        self.polygon
            .extend(corners.iter().map(|&p| Vec2::new(p.dot(u), p.dot(v))));

        if n == 4 && is_convex(&self.polygon) {
            // The shorter diagonal gives the better shape for non-planar quads.
            if corners[0].distance_squared(corners[2]) <= corners[1].distance_squared(corners[3]) {
                self.local.extend_from_slice(&[[0, 1, 2], [0, 2, 3]]);
            } else {
                self.local.extend_from_slice(&[[0, 1, 3], [1, 2, 3]]);
            }
        } else {
            ear_clip(&self.polygon, &mut self.remaining, &mut self.local);
        }
    }
}

fn fan(n: u32, out: &mut Vec<[u32; 3]>) {
    out.extend((1..n - 1).map(|k| [0, k, k + 1]));
}

fn is_convex(polygon: &[Vec2]) -> bool {
    let n = polygon.len();
    (0..n).all(|i| {
        let (a, b, c) = (polygon[i], polygon[(i + 1) % n], polygon[(i + 2) % n]);
        (b - a).perp_dot(c - b) >= 0.0
    })
}

/// Ear clipping of a counter-clockwise simple polygon. Falls back to a fan for
/// whatever is left if no ear can be found (degenerate or self-intersecting
/// input), so it always terminates with `n − 2` triangles.
fn ear_clip(polygon: &[Vec2], remaining: &mut Vec<u32>, out: &mut Vec<[u32; 3]>) {
    remaining.clear();
    remaining.extend((0..polygon.len()).map(u32_index));
    // Turns smaller than this count as straight (relative to the polygon's size).
    let (min, max) = polygon
        .iter()
        .fold((polygon[0], polygon[0]), |(lo, hi), &p| (lo.min(p), hi.max(p)));
    let epsilon = (max - min).length_squared() * 1e-10;

    while remaining.len() > 3 {
        let m = remaining.len();
        let ear = (0..m).find(|&i| {
            let (ia, ib, ic) = (remaining[(i + m - 1) % m], remaining[i], remaining[(i + 1) % m]);
            let (a, b, c) = (polygon[us(ia)], polygon[us(ib)], polygon[us(ic)]);
            if (b - a).perp_dot(c - b) <= epsilon {
                return false; // reflex or flat corner
            }
            !remaining.iter().any(|&j| {
                let p = polygon[us(j)];
                j != ia && j != ib && j != ic && p != a && p != b && p != c && in_triangle(p, a, b, c)
            })
        });
        let Some(i) = ear else {
            let rest = &remaining[..];
            out.extend((1..rest.len() - 1).map(|k| [rest[0], rest[k], rest[k + 1]]));
            return;
        };
        out.push([remaining[(i + m - 1) % m], remaining[i], remaining[(i + 1) % m]]);
        remaining.remove(i);
    }
    out.push([remaining[0], remaining[1], remaining[2]]);
}

/// Inside or on the border of the counter-clockwise triangle `a b c`.
fn in_triangle(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> bool {
    (b - a).perp_dot(p - a) >= 0.0 && (c - b).perp_dot(p - b) >= 0.0 && (a - c).perp_dot(p - c) >= 0.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes;

    fn triangulate(mesh: &PolyMesh) -> TriMesh {
        let mut out = TriMesh::default();
        Triangulator::new().triangulate(mesh, Smoothing::default(), &mut out);
        out
    }

    fn triangle(tri: &TriMesh, t: usize) -> [Vec3; 3] {
        [0, 1, 2].map(|k| tri.positions[us(tri.indices[t * 3 + k])])
    }

    fn all_shapes() -> Vec<(&'static str, PolyMesh)> {
        let mut shapes = shapes::closed();
        shapes.extend([
            ("open box", shapes::open_box()),
            ("grid", shapes::grid(3, 2)),
            ("L", shapes::l_shape()),
            ("star", shapes::star()),
        ]);
        shapes
    }

    #[test]
    fn every_face_becomes_n_minus_2_triangles() {
        for (name, mesh) in all_shapes().into_iter().chain([("collinear", shapes::collinear())]) {
            let tri = triangulate(&mesh);
            let expected: usize = mesh.faces().map(|f| f.len() - 2).sum();
            assert_eq!(tri.triangle_count(), expected, "{name}");
            assert_eq!(tri.indices.len(), expected * 3, "{name}");
            assert_eq!(tri.vertex_count(), mesh.corner_count(), "{name}");
            assert!(tri.indices.iter().all(|&i| us(i) < tri.vertex_count()), "{name}");
        }
    }

    /// Every triangle faces the same way as its polygon and as its vertex normals
    /// (counter-clockwise from outside) — the GPU test of `grid4d-render`, ported.
    #[test]
    fn winding_matches_normals() {
        for (name, mesh) in all_shapes() {
            let tri = triangulate(&mesh);
            for t in 0..tri.triangle_count() {
                let [a, b, c] = triangle(&tri, t);
                let geometric = (b - a).cross(c - a);
                assert!(geometric.length() > 1e-7, "{name}: degenerate triangle {t}");
                let face = mesh.face_normal(us(tri.triangle_faces[t]));
                assert!(
                    geometric.dot(face) > 0.0,
                    "{name}: triangle {t} is flipped against its face"
                );
                let corner_normals: Vec3 = (0..3).map(|k| tri.normals[us(tri.indices[t * 3 + k])]).sum();
                assert!(
                    geometric.dot(corner_normals) > 0.0,
                    "{name}: triangle {t} disagrees with its normals"
                );
            }
        }
    }

    #[test]
    fn triangles_cover_each_polygon_exactly() {
        for (name, mesh) in [
            ("cube", shapes::cube(2.0)),
            ("cylinder", shapes::cylinder(12)),
            ("L", shapes::l_shape()),
            ("star", shapes::star()),
        ] {
            let tri = triangulate(&mesh);
            let mut per_face = vec![0.0f32; mesh.face_count()];
            for t in 0..tri.triangle_count() {
                let [a, b, c] = triangle(&tri, t);
                per_face[us(tri.triangle_faces[t])] += (b - a).cross(c - a).length() * 0.5;
            }
            for (f, area) in per_face.into_iter().enumerate() {
                assert!(
                    (area - mesh.face_area(f)).abs() < 1e-4,
                    "{name} face {f}: {area} vs {}",
                    mesh.face_area(f)
                );
            }
        }
    }

    #[test]
    fn convex_quads_split_along_the_shorter_diagonal() {
        // A kite: diagonal 1–3 is much shorter than 0–2.
        let points = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, -0.3),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.3),
        ];
        let kite = PolyMesh::from_faces(points, [[0u32, 3, 2, 1]]).unwrap();
        let tri = triangulate(&kite);
        assert_eq!(tri.indices, [0, 1, 3, 1, 2, 3], "split between the close points");
    }

    #[test]
    fn uvs_and_faces_are_carried_through() {
        let mut grid = shapes::grid(2, 1);
        let uvs: Vec<Vec2> = (0..grid.corner_count())
            .map(|c| Vec2::splat(f32::from(u8::try_from(c).unwrap())))
            .collect();
        grid.set_uvs(Some(uvs.clone())).unwrap();
        let tri = triangulate(&grid);
        assert_eq!(tri.uvs, uvs);
        assert_eq!(tri.triangle_faces, [0, 0, 1, 1]);
        assert!(triangulate(&shapes::cube(1.0)).uvs.is_empty());
    }

    #[test]
    fn retriangulating_reuses_every_buffer() {
        let torus = shapes::torus(24, 12);
        let mut triangulator = Triangulator::new();
        let mut out = TriMesh::default();
        triangulator.triangulate(&torus, Smoothing::default(), &mut out);
        let capacities = |o: &TriMesh| {
            [
                o.positions.capacity(),
                o.normals.capacity(),
                o.indices.capacity(),
                o.triangle_faces.capacity(),
            ]
        };
        let before = capacities(&out);
        for _ in 0..5 {
            triangulator.triangulate(&torus, Smoothing::Smooth, &mut out);
        }
        assert_eq!(
            capacities(&out),
            before,
            "steady-state triangulation must not reallocate"
        );
    }
}
