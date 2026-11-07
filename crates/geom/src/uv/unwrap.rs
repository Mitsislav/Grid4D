//! Unwrapping (see `docs/reference/manager-uv-editor.md`): the chosen faces
//! are cut along seams into parts, each part is flattened with **least
//! squares conformal maps** (LSCM, Lévy et al. 2002: the map that keeps
//! angles best, found as a sparse least-squares problem with two points
//! pinned), scaled to its surface area and packed into the UV square.

use glam::{DVec2, DVec3, Vec2, Vec3};

use super::{
    islands::{corner_faces, group, uv_area},
    pack::{PackOptions, pack},
};
use crate::{BitSet, PolyMesh, Smoothing, Topology, TriMesh, Triangulator, us};

/// How [`unwrap`] lays out its result.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UnwrapOptions {
    /// How the flattened parts are packed into the square.
    pub pack: PackOptions,
}

/// The corner UVs of `mesh` with the faces `faces` (all of them when empty)
/// unwrapped: cut along the edges `cuts` (by [`Topology`] index), the borders
/// of `faces` and the mesh's open borders, each part flattened, scaled to its
/// surface area and the parts packed together into the square. Other faces
/// keep their UVs (a mesh without UVs gives them a Box projection's).
#[must_use]
pub fn unwrap(
    mesh: &PolyMesh,
    topology: &Topology,
    faces: &BitSet,
    cuts: &BitSet,
    options: &UnwrapOptions,
) -> Vec<Vec2> {
    let mut uvs = mesh.uvs().map_or_else(|| super::box_uvs(mesh), <[Vec2]>::to_vec);
    let chosen = |f: usize| faces.is_empty() || faces.contains(f);
    let face_of = corner_faces(mesh);

    // Parts: faces joined across edges that are not cut. A part that is not a
    // disc (closed, or with handles) cannot lie flat in one piece: it is
    // split further by the direction its faces face.
    let first = group(mesh, topology, chosen, |edge, _, _| !cuts.contains(edge));
    let discs = disc_groups(mesh, &first);
    let class: Vec<u8> = (0..mesh.face_count()).map(|f| facing(mesh.face_normal(f))).collect();
    let parts = group(mesh, topology, chosen, |edge, a, b| {
        if cuts.contains(edge) {
            return false;
        }
        let (fa, fb) = (us(face_of[a[0]]), us(face_of[b[0]]));
        let part = first.group_of[fa];
        discs.get(us(part)).copied().unwrap_or(true) || class[fa] == class[fb]
    });

    // Triangles by face, as corners.
    let mut tri = TriMesh::default();
    Triangulator::default().triangulate(mesh, Smoothing::default(), &mut tri);
    let mut faces_of_part = vec![Vec::new(); parts.group_count];
    for f in 0..mesh.face_count() {
        if let Some(list) = faces_of_part.get_mut(us(parts.group_of[f])) {
            list.push(f);
        }
    }
    let mut triangles_of_part = vec![Vec::new(); parts.group_count];
    for (t, &f) in tri.indices.as_chunks::<3>().0.iter().zip(&tri.triangle_faces) {
        if let Some(list) = triangles_of_part.get_mut(us(parts.group_of[us(f)])) {
            list.push(t.map(us));
        }
    }

    // Each part flattened on its own vertices.
    let points = mesh.points();
    let corner_points = mesh.corner_points();
    let mut local = vec![usize::MAX; parts.vertex_count];
    let mut islands = Vec::with_capacity(parts.group_count);
    for (part_faces, part_triangles) in faces_of_part.iter().zip(&triangles_of_part) {
        let mut positions = Vec::new();
        let mut corners = Vec::new();
        for &f in part_faces {
            for c in mesh.corner_range(f) {
                let v = us(parts.vertex_of[c]);
                if local[v] == usize::MAX {
                    local[v] = positions.len();
                    positions.push(points[us(corner_points[c])]);
                }
                corners.push(c);
            }
        }
        let triangles: Vec<[usize; 3]> = part_triangles
            .iter()
            .map(|t| t.map(|c| local[us(parts.vertex_of[c])]))
            .collect();
        let flat = lscm(&positions, &triangles);
        for &c in &corners {
            uvs[c] = flat.get(local[us(parts.vertex_of[c])]).copied().unwrap_or(Vec2::ZERO);
        }
        // Mirrored as a whole (it should not be): turned the right way round.
        let winding: f32 = part_faces.iter().map(|&f| uv_area(&uvs, mesh.corner_range(f))).sum();
        if winding < 0.0 {
            for &c in &corners {
                uvs[c].x = -uvs[c].x;
            }
        }
        // As large in UV as on the surface.
        let uv: f32 = part_faces
            .iter()
            .map(|&f| uv_area(&uvs, mesh.corner_range(f)).abs())
            .sum();
        let surface: f32 = part_faces.iter().map(|&f| mesh.face_area(f)).sum();
        if uv > 0.0 && surface > 0.0 {
            let scale = (surface / uv).sqrt();
            for &c in &corners {
                uvs[c] *= scale;
            }
        }
        upright(&mut uvs, &corners);
        for &c in &corners {
            local[us(parts.vertex_of[c])] = usize::MAX;
        }
        islands.push(corners);
    }
    pack(&mut uvs, &islands, &options.pack);
    uvs
}

/// Turns the corners `corners` about their middle so the box around them is
/// as small as it gets (whole degrees from 0 to 89; turning never mirrors),
/// so packed parts waste little room.
fn upright(uvs: &mut [Vec2], corners: &[usize]) {
    let area = |angle: f32| {
        let turn = Vec2::from_angle(angle);
        let mut min = Vec2::splat(f32::INFINITY);
        let mut max = Vec2::splat(f32::NEG_INFINITY);
        for &c in corners {
            let at = turn.rotate(uvs[c]);
            min = min.min(at);
            max = max.max(at);
        }
        let size = max - min;
        size.x * size.y
    };
    let best = (0u8..90)
        .map(|degrees| f32::from(degrees).to_radians())
        .min_by(|&a, &b| area(a).total_cmp(&area(b)))
        .unwrap_or(0.0);
    if best == 0.0 || area(best) >= area(0.0) * 0.999 {
        return;
    }
    let turn = Vec2::from_angle(best);
    for &c in corners {
        uvs[c] = turn.rotate(uvs[c]);
    }
}

/// Which way a face mostly faces: ±X, ±Y or ±Z (0 – 5).
fn facing(normal: Vec3) -> u8 {
    let a = normal.abs();
    let (axis, value) = if a.x >= a.y && a.x >= a.z {
        (0, normal.x)
    } else if a.y >= a.z {
        (1, normal.y)
    } else {
        (2, normal.z)
    };
    axis * 2 + u8::from(value < 0.0)
}

/// For each group, whether it is a disc (Euler characteristic 1: it lies
/// flat in one piece).
fn disc_groups(mesh: &PolyMesh, grouping: &super::islands::Grouping) -> Vec<bool> {
    let n = grouping.group_count;
    let mut vertices = vec![0i64; n];
    let mut faces = vec![0i64; n];
    let mut edges: Vec<std::collections::HashSet<(u32, u32)>> = vec![std::collections::HashSet::new(); n];
    let mut seen = BitSet::new();
    for f in 0..mesh.face_count() {
        let g = grouping.group_of[f];
        let Some(count) = faces.get_mut(us(g)) else {
            continue;
        };
        *count += 1;
        let range = mesh.corner_range(f);
        for c in range.clone() {
            let v = grouping.vertex_of[c];
            if seen.insert(us(v)) {
                vertices[us(g)] += 1;
            }
            let w = grouping.vertex_of[super::islands::next_corner(c, &range)];
            edges[us(g)].insert((v.min(w), v.max(w)));
        }
    }
    (0..n)
        .map(|g| vertices[g] - i64::try_from(edges[g].len()).unwrap_or(i64::MAX) + faces[g] == 1)
        .collect()
}

/// The least squares conformal map of the triangles `triangles` over the
/// points `positions`: one UV (`v` down) per point, unscaled.
fn lscm(positions: &[Vec3], triangles: &[[usize; 3]]) -> Vec<Vec2> {
    let n = positions.len();
    let p: Vec<DVec3> = positions.iter().map(|q| q.as_dvec3()).collect();
    // Two pinned points far apart: the farthest from the first point, then
    // the farthest from that one.
    let farthest = |from: usize| {
        (0..n)
            .max_by(|&a, &b| {
                p[a].distance_squared(p[from])
                    .total_cmp(&p[b].distance_squared(p[from]))
            })
            .unwrap_or(0)
    };
    let a = farthest(0);
    let b = farthest(a);
    if n < 3 || a == b {
        return planar(&p, a, b);
    }
    let pinned = [(a, DVec2::ZERO), (b, DVec2::new(p[a].distance(p[b]), 0.0))];
    // Unknown number of each free point (u at 2i, v at 2i + 1).
    let mut unknown = vec![usize::MAX; n];
    let mut free = 0;
    for (i, slot) in unknown.iter_mut().enumerate() {
        if i != a && i != b {
            *slot = free;
            free += 1;
        }
    }
    let pin_of = |i: usize| pinned.iter().find(|(j, _)| *j == i).map(|(_, at)| *at);

    // Two rows per triangle (the Cauchy-Riemann equations of its linear map,
    // weighted by the square root of its area).
    let mut system = Sparse::default();
    for t in triangles {
        let [q0, q1, q2] = t.map(|i| p[i]);
        let e1 = q1 - q0;
        let e2 = q2 - q0;
        let normal = e1.cross(e2);
        let double_area = normal.length();
        if double_area <= 1e-14 * e1.length_squared().max(e2.length_squared()).max(1e-30) {
            continue;
        }
        let x = e1 / e1.length();
        let y = (normal / double_area).cross(x);
        let local = [
            DVec2::ZERO,
            DVec2::new(e1.length(), 0.0),
            DVec2::new(e2.dot(x), e2.dot(y)),
        ];
        // Gradients of the three corner functions in the triangle's plane.
        let rot = |d: DVec2| DVec2::new(-d.y, d.x) / double_area;
        let g = [
            rot(local[2] - local[1]),
            rot(local[0] - local[2]),
            rot(local[1] - local[0]),
        ];
        let w = (double_area * 0.5).sqrt();
        let mut rows = [Vec::with_capacity(6), Vec::with_capacity(6)];
        let mut rhs = [0.0f64; 2];
        for (corner, gradient) in t.iter().zip(g) {
            // Row 1: du/dx − dv/dy; row 2: du/dy + dv/dx.
            let (cu1, cv1, cu2, cv2) = (w * gradient.x, -w * gradient.y, w * gradient.y, w * gradient.x);
            if let Some(at) = pin_of(*corner) {
                rhs[0] -= cu1 * at.x + cv1 * at.y;
                rhs[1] -= cu2 * at.x + cv2 * at.y;
            } else {
                let k = unknown[*corner];
                rows[0].extend([(2 * k, cu1), (2 * k + 1, cv1)]);
                rows[1].extend([(2 * k, cu2), (2 * k + 1, cv2)]);
            }
        }
        system.push(&rows[0], rhs[0]);
        system.push(&rows[1], rhs[1]);
    }

    // A first guess: the points on the plane they face most, turned so the
    // pins lie as they will.
    let guess = planar(&p, a, b);
    let mut x = vec![0.0; 2 * free];
    for i in 0..n {
        if unknown[i] != usize::MAX {
            x[2 * unknown[i]] = f64::from(guess[i].x);
            x[2 * unknown[i] + 1] = -f64::from(guess[i].y);
        }
    }
    system.solve(&mut x, 2 * free);

    (0..n)
        .map(|i| {
            let at = pin_of(i).unwrap_or_else(|| DVec2::new(x[2 * unknown[i]], x[2 * unknown[i] + 1]));
            // Solved with v up; Grid4D's v runs down the picture.
            Vec2::new(at.x as f32, -at.y as f32)
        })
        .collect()
}

/// The points projected on the plane of their average facing, turned and
/// moved so `a` lies at the origin and `b` along +u (`v` down).
fn planar(p: &[DVec3], a: usize, b: usize) -> Vec<Vec2> {
    let (Some(&pa), Some(&pb)) = (p.get(a), p.get(b)) else {
        return vec![Vec2::ZERO; p.len()];
    };
    // The pin axis and the direction the points spread most across it span
    // a plane that fits them well enough for a first guess.
    let along = (pb - pa).try_normalize().unwrap_or(DVec3::X);
    let side = p
        .iter()
        .map(|&q| (q - pa) - along * (q - pa).dot(along))
        .max_by(|x, y| x.length_squared().total_cmp(&y.length_squared()))
        .and_then(DVec3::try_normalize)
        .unwrap_or_else(|| along.any_orthonormal_vector());
    p.iter()
        .map(|&q| {
            let d = q - pa;
            Vec2::new(d.dot(along) as f32, -(d.dot(side)) as f32)
        })
        .collect()
}

/// A sparse matrix by rows, and its right-hand side.
#[derive(Default)]
struct Sparse {
    starts: Vec<usize>,
    columns: Vec<usize>,
    values: Vec<f64>,
    rhs: Vec<f64>,
}

impl Sparse {
    fn push(&mut self, row: &[(usize, f64)], rhs: f64) {
        self.starts.push(self.columns.len());
        for &(c, v) in row {
            self.columns.push(c);
            self.values.push(v);
        }
        self.rhs.push(rhs);
    }

    fn row(&self, r: usize) -> std::ops::Range<usize> {
        self.starts[r]..self.starts.get(r + 1).copied().unwrap_or(self.columns.len())
    }

    /// Least squares: improves `x` (of `size` unknowns) towards the `x` that
    /// makes `A·x − rhs` smallest, by conjugate gradients on the normal
    /// equations (CGLS) with each column scaled to unit length.
    fn solve(&self, x: &mut [f64], size: usize) {
        if size == 0 {
            return;
        }
        let rows = self.rhs.len();
        let mut norm = vec![0.0f64; size];
        for (&c, &v) in self.columns.iter().zip(&self.values) {
            norm[c] += v * v;
        }
        let scale: Vec<f64> = norm
            .iter()
            .map(|&s| if s > 0.0 { 1.0 / s.sqrt() } else { 0.0 })
            .collect();
        // In scaled unknowns y = x / scale: A' = A · diag(scale).
        let mut y: Vec<f64> = x
            .iter()
            .zip(&scale)
            .map(|(&xi, &s)| if s > 0.0 { xi / s } else { 0.0 })
            .collect();
        let times = |v: &[f64], out: &mut Vec<f64>| {
            out.clear();
            out.extend((0..rows).map(|r| {
                self.row(r)
                    .map(|k| self.values[k] * scale[self.columns[k]] * v[self.columns[k]])
                    .sum::<f64>()
            }));
        };
        let transposed = |v: &[f64], out: &mut Vec<f64>| {
            out.clear();
            out.resize(size, 0.0);
            for (r, &vr) in v.iter().enumerate() {
                for k in self.row(r) {
                    let c = self.columns[k];
                    out[c] += self.values[k] * scale[c] * vr;
                }
            }
        };
        let mut residual = Vec::new();
        times(&y, &mut residual);
        for (r, b) in residual.iter_mut().zip(&self.rhs) {
            *r = b - *r;
        }
        let mut s = Vec::new();
        transposed(&residual, &mut s);
        let mut direction = s.clone();
        let mut gamma: f64 = s.iter().map(|v| v * v).sum();
        // Done when the gradient is small against the problem's own size
        // (the gradient from nothing), not against the first guess's: a good
        // guess must not make the solver chase rounding.
        let mut size_of_problem = Vec::new();
        transposed(&self.rhs, &mut size_of_problem);
        let reference: f64 = size_of_problem.iter().map(|v| v * v).sum::<f64>().sqrt();
        let start = if reference > 0.0 { reference } else { gamma.sqrt() };
        let mut q = Vec::new();
        let limit = (size * 10).max(500);
        for _ in 0..limit {
            if gamma.sqrt() <= start * 1e-6 || gamma == 0.0 {
                break;
            }
            times(&direction, &mut q);
            let qq: f64 = q.iter().map(|v| v * v).sum();
            if qq <= 0.0 {
                break;
            }
            let alpha = gamma / qq;
            for (yi, di) in y.iter_mut().zip(&direction) {
                *yi += alpha * di;
            }
            for (ri, qi) in residual.iter_mut().zip(&q) {
                *ri -= alpha * qi;
            }
            transposed(&residual, &mut s);
            let next: f64 = s.iter().map(|v| v * v).sum();
            let beta = next / gamma;
            gamma = next;
            for (di, si) in direction.iter_mut().zip(&s) {
                *di = si + beta * *di;
            }
        }
        for ((xi, yi), s) in x.iter_mut().zip(&y).zip(&scale) {
            *xi = yi * s;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        primitives,
        uv::{flipped_faces, islands::layout, stretch_measure},
    };

    /// The edges of `mesh` between the points `pairs`.
    fn edges(topology: &Topology, pairs: &[[u32; 2]]) -> BitSet {
        pairs
            .iter()
            .map(|&[a, b]| {
                let key = [a.min(b), a.max(b)];
                topology.edges().iter().position(|e| *e == key).unwrap()
            })
            .collect()
    }

    /// A tree of edges reaching all of `mesh`'s points (breadth first from
    /// point 0).
    fn spanning_tree(mesh: &PolyMesh, topology: &Topology) -> BitSet {
        let mut reached = BitSet::new();
        reached.insert(0);
        let mut queue = vec![0usize];
        let mut tree = BitSet::new();
        while let Some(p) = queue.pop() {
            for &e in topology.point_edges(p) {
                let [a, b] = topology.edges()[us(e)];
                let other = if us(a) == p { us(b) } else { us(a) };
                if reached.insert(other) {
                    tree.insert(us(e));
                    queue.insert(0, other);
                }
            }
        }
        assert_eq!(reached.len(), mesh.point_count());
        tree
    }

    fn with_uvs(mesh: &PolyMesh, uvs: Vec<Vec2>) -> PolyMesh {
        let mut out = mesh.clone();
        out.set_uvs(Some(uvs)).unwrap();
        out
    }

    #[test]
    fn a_cube_cut_along_seven_edges_unfolds_into_one_island_without_flips() {
        let cube = primitives::cube(Vec3::splat(2.0), [1, 1, 1]);
        let topology = Topology::build(&cube);
        let cuts = spanning_tree(&cube, &topology);
        assert_eq!(cuts.len(), 7);
        let uvs = unwrap(&cube, &topology, &BitSet::new(), &cuts, &UnwrapOptions::default());
        let unwrapped = with_uvs(&cube, uvs.clone());
        let layout = layout(&unwrapped, &topology);
        assert_eq!(layout.island_count, 1, "one net");
        assert_eq!(flipped_faces(&unwrapped), 0);
        assert!(
            uvs.iter()
                .all(|uv| (-1e-4..=1.0001).contains(&uv.x) && (-1e-4..=1.0001).contains(&uv.y))
        );
        // A cube's net is flat: every side keeps its square shape.
        assert!(
            stretch_measure(&unwrapped, &topology) < 1e-3,
            "{}",
            stretch_measure(&unwrapped, &topology)
        );
    }

    #[test]
    fn an_open_cylinder_cut_along_one_edge_flattens_with_little_distortion() {
        let full = primitives::cylinder(1.0, 3.0, 24, 4);
        // The side only: faces that do not face up or down.
        let side = full.with_faces_where(|f| full.face_normal(f).y.abs() < 0.5);
        let topology = Topology::build(&side);
        // One vertical line of edges: from the points at angle 0.
        let seam: Vec<[u32; 2]> = topology
            .edges()
            .iter()
            .copied()
            .filter(|&[a, b]| {
                let (pa, pb) = (side.points()[us(a)], side.points()[us(b)]);
                pa.x > 0.999 && pb.x > 0.999 && pa.z.abs() < 1e-4 && pb.z.abs() < 1e-4
            })
            .collect();
        assert_eq!(seam.len(), 4);
        let cuts = edges(&topology, &seam);
        let uvs = unwrap(&side, &topology, &BitSet::new(), &cuts, &UnwrapOptions::default());
        let unwrapped = with_uvs(&side, uvs);
        assert_eq!(layout(&unwrapped, &topology).island_count, 1);
        assert_eq!(flipped_faces(&unwrapped), 0);
        let stretch = super::super::face_stretch(&unwrapped, &topology);
        // log2(1.05) ≈ 0.07: every face within 5 % of the island's area per unit.
        assert!(stretch.iter().all(|s| s.abs() < 0.07), "{stretch:?}");
    }

    #[test]
    fn a_closed_part_without_seams_is_split_and_selections_unwrap_alone() {
        let sphere = primitives::sphere(1.0, 16);
        let topology = Topology::build(&sphere);
        let uvs = unwrap(
            &sphere,
            &topology,
            &BitSet::new(),
            &BitSet::new(),
            &UnwrapOptions::default(),
        );
        let unwrapped = with_uvs(&sphere, uvs);
        assert_eq!(flipped_faces(&unwrapped), 0);
        assert!(layout(&unwrapped, &topology).island_count >= 2, "split to lie flat");

        // Only the first face: the others keep their UVs.
        let plane = primitives::plane(2.0, 2.0, 2, 2);
        let topology = Topology::build(&plane);
        let first: BitSet = [0].into_iter().collect();
        let uvs = unwrap(&plane, &topology, &first, &BitSet::new(), &UnwrapOptions::default());
        let before = plane.uvs().unwrap();
        for c in plane.corner_range(1).start..plane.corner_count() {
            assert_eq!(uvs[c], before[c]);
        }
        assert_eq!(flipped_faces(&with_uvs(&plane, uvs)), 0);
    }
}
