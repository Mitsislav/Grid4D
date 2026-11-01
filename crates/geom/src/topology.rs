use crate::{PolyMesh, u32_index, us};

/// Adjacency of a [`PolyMesh`]: its unique edges and which faces use each edge
/// and each point.
///
/// Built on demand (modelling tools, validation, smoothing), not stored in the
/// mesh: the face-vertex arrays stay the single source of truth. Adjacency lists
/// are packed (offsets + one flat list), so building allocates a fixed handful of
/// vectors regardless of mesh size.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Topology {
    /// Unique undirected edges `[a, b]` with `a < b`, in order of first use.
    edges: Vec<[u32; 2]>,
    /// Edge index of each corner `c`: the edge from corner `c` to the next corner
    /// of the same face.
    corner_edges: Vec<u32>,
    edge_face_offsets: Vec<u32>,
    edge_face_list: Vec<u32>,
    edge_corner_offsets: Vec<u32>,
    edge_corner_list: Vec<u32>,
    point_face_offsets: Vec<u32>,
    point_face_list: Vec<u32>,
    point_edge_offsets: Vec<u32>,
    point_edge_list: Vec<u32>,
    /// Points used by at least one face.
    used_points: usize,
    face_count: usize,
    /// No directed edge `a → b` is used by two faces: neighbouring faces agree
    /// on their winding.
    consistently_oriented: bool,
}

impl Topology {
    #[must_use]
    pub fn build(mesh: &PolyMesh) -> Self {
        // Each corner's directed edge, to the next corner of its face.
        let points = mesh.corner_points();
        let mut ends: Vec<[u32; 2]> = Vec::with_capacity(mesh.corner_count());
        for f in 0..mesh.face_count() {
            let range = mesh.corner_range(f);
            for c in range.clone() {
                let next = if c + 1 == range.end { range.start } else { c + 1 };
                ends.push([points[c], points[next]]);
            }
        }
        // Corners by their edge's lower point, in corner order: an edge's
        // earlier corners are found among a point's few, without hashing.
        let (low_offsets, low_list) = pack(mesh.point_count(), |emit| {
            for (c, &[a, b]) in ends.iter().enumerate() {
                emit(us(a.min(b)), u32_index(c));
            }
        });

        let mut edges = Vec::new();
        let mut corner_edges: Vec<u32> = Vec::with_capacity(ends.len());
        let mut consistently_oriented = true;
        for (c, &[a, b]) in ends.iter().enumerate() {
            let (low, high) = (a.min(b), a.max(b));
            let mut edge = None;
            for &earlier in slice(&low_offsets, &low_list, us(low)) {
                let earlier = us(earlier);
                if earlier >= c {
                    break;
                }
                let [ea, eb] = ends[earlier];
                if ea.max(eb) == high {
                    edge = edge.or(Some(corner_edges[earlier]));
                    // The same directed edge in two faces: they disagree on
                    // their winding.
                    consistently_oriented &= ea != a;
                }
            }
            corner_edges.push(edge.unwrap_or_else(|| {
                edges.push([low, high]);
                u32_index(edges.len() - 1)
            }));
        }

        // Faces per edge.
        let (edge_face_offsets, edge_face_list) = pack(edges.len(), |emit| {
            for (f, range) in (0..mesh.face_count()).map(|f| (f, mesh.corner_range(f))) {
                for c in range {
                    emit(us(corner_edges[c]), u32_index(f));
                }
            }
        });
        // Corners per edge (the inverse of `corner_edges`).
        let (edge_corner_offsets, edge_corner_list) = pack(edges.len(), |emit| {
            for (c, &e) in corner_edges.iter().enumerate() {
                emit(us(e), u32_index(c));
            }
        });
        // Faces per point.
        let (point_face_offsets, point_face_list) = pack(mesh.point_count(), |emit| {
            for (f, face) in mesh.faces().enumerate() {
                for &p in face {
                    emit(us(p), u32_index(f));
                }
            }
        });
        // Edges per point.
        let (point_edge_offsets, point_edge_list) = pack(mesh.point_count(), |emit| {
            for (e, &[a, b]) in edges.iter().enumerate() {
                emit(us(a), u32_index(e));
                emit(us(b), u32_index(e));
            }
        });
        let used_points = point_face_offsets.windows(2).filter(|w| w[1] > w[0]).count();

        Self {
            edges,
            corner_edges,
            edge_face_offsets,
            edge_face_list,
            edge_corner_offsets,
            edge_corner_list,
            point_face_offsets,
            point_face_list,
            point_edge_offsets,
            point_edge_list,
            used_points,
            face_count: mesh.face_count(),
            consistently_oriented,
        }
    }

    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Unique edges as `[a, b]` point pairs with `a < b`.
    #[must_use]
    pub fn edges(&self) -> &[[u32; 2]] {
        &self.edges
    }

    /// The edge running from corner `corner` to the next corner of its face.
    #[must_use]
    pub fn corner_edge(&self, corner: usize) -> Option<u32> {
        self.corner_edges.get(corner).copied()
    }

    /// Corners whose edge is `edge` — one per face using it (empty if out of range).
    #[must_use]
    pub fn edge_corners(&self, edge: usize) -> &[u32] {
        slice(&self.edge_corner_offsets, &self.edge_corner_list, edge)
    }

    /// Faces that use edge `edge` (empty if out of range).
    #[must_use]
    pub fn edge_faces(&self, edge: usize) -> &[u32] {
        slice(&self.edge_face_offsets, &self.edge_face_list, edge)
    }

    /// Unique edges that end at point `point` (empty if out of range or unused).
    #[must_use]
    pub fn point_edges(&self, point: usize) -> &[u32] {
        slice(&self.point_edge_offsets, &self.point_edge_list, point)
    }

    /// Faces that use point `point` (empty if out of range or unused).
    #[must_use]
    pub fn point_faces(&self, point: usize) -> &[u32] {
        slice(&self.point_face_offsets, &self.point_face_list, point)
    }

    /// Edges used by exactly one face: the mesh's open border.
    #[must_use]
    pub fn boundary_edge_count(&self) -> usize {
        (0..self.edges.len()).filter(|&e| self.edge_faces(e).len() == 1).count()
    }

    /// Every edge is shared by exactly two faces: the surface has no border.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        !self.edges.is_empty() && (0..self.edges.len()).all(|e| self.edge_faces(e).len() == 2)
    }

    /// No edge has more than two faces, and neighbouring faces are wound the same
    /// way — what modelling tools and smoothing assume.
    #[must_use]
    pub fn is_manifold(&self) -> bool {
        self.consistently_oriented && (0..self.edges.len()).all(|e| self.edge_faces(e).len() <= 2)
    }

    #[must_use]
    pub fn is_consistently_oriented(&self) -> bool {
        self.consistently_oriented
    }

    /// V − E + F over the points that faces use: 2 for a closed sphere-like
    /// surface, 0 for a torus, 1 for a disc.
    #[must_use]
    pub fn euler_characteristic(&self) -> i64 {
        let count = |n: usize| i64::try_from(n).unwrap_or(i64::MAX);
        count(self.used_points) - count(self.edges.len()) + count(self.face_count)
    }
}

/// Builds packed adjacency lists for `len` keys: `fill` calls `emit(key, value)`
/// for every entry; it is run twice (count, then place).
fn pack(len: usize, fill: impl Fn(&mut dyn FnMut(usize, u32))) -> (Vec<u32>, Vec<u32>) {
    let mut offsets = vec![0u32; len + 1];
    fill(&mut |key, _| offsets[key + 1] += 1);
    for i in 0..len {
        offsets[i + 1] += offsets[i];
    }
    let mut cursor: Vec<u32> = offsets[..len].to_vec();
    let mut list = vec![0u32; us(offsets[len])];
    fill(&mut |key, value| {
        list[us(cursor[key])] = value;
        cursor[key] += 1;
    });
    (offsets, list)
}

fn slice<'a>(offsets: &[u32], list: &'a [u32], key: usize) -> &'a [u32] {
    match (offsets.get(key), offsets.get(key + 1)) {
        (Some(&start), Some(&end)) => &list[us(start)..us(end)],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;
    use crate::shapes;

    /// The edges, corner edges and orientation as the first version found
    /// them, with hash maps: the packed search must give exactly the same.
    fn by_hashing(mesh: &PolyMesh) -> (Vec<[u32; 2]>, Vec<u32>, bool) {
        use std::collections::{HashMap, HashSet};
        let mut index: HashMap<[u32; 2], u32> = HashMap::new();
        let mut directed = HashSet::new();
        let (mut edges, mut corner_edges, mut oriented) = (Vec::new(), Vec::new(), true);
        for face in mesh.faces() {
            for (i, &a) in face.iter().enumerate() {
                let b = face[(i + 1) % face.len()];
                oriented &= directed.insert([a, b]);
                let key = [a.min(b), a.max(b)];
                let edge = *index.entry(key).or_insert_with(|| {
                    edges.push(key);
                    u32_index(edges.len() - 1)
                });
                corner_edges.push(edge);
            }
        }
        (edges, corner_edges, oriented)
    }

    #[test]
    fn edges_are_found_as_by_hashing() {
        let mut meshes: Vec<PolyMesh> = shapes::closed().into_iter().map(|(_, mesh)| mesh).collect();
        meshes.extend([
            shapes::open_box(),
            shapes::grid(7, 3),
            shapes::l_shape(),
            shapes::star(),
        ]);
        // A face turned the other way, and faces sharing an edge three times.
        let mut flipped = shapes::grid(2, 1);
        flipped.add_face(&[0, 3, 4, 1]).unwrap();
        meshes.push(flipped);
        // Random faces over a few points: repeated and non-manifold edges.
        let mut seed = 7u32;
        for _ in 0..20 {
            let mut mesh = PolyMesh::new();
            for k in 0..12u8 {
                mesh.add_point(Vec3::new(f32::from(k), 0.0, 0.0)).unwrap();
            }
            for _ in 0..30 {
                let mut corners = Vec::new();
                while corners.len() < 3 + usize::try_from(seed % 3).unwrap() {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    let point = seed % 12;
                    if !corners.contains(&point) {
                        corners.push(point);
                    }
                }
                mesh.add_face(&corners).unwrap();
            }
            meshes.push(mesh);
        }
        for mesh in &meshes {
            let topology = Topology::build(mesh);
            let (edges, corner_edges, oriented) = by_hashing(mesh);
            assert_eq!(topology.edges, edges);
            assert_eq!(topology.corner_edges, corner_edges);
            assert_eq!(topology.consistently_oriented, oriented);
        }
        assert!(meshes.iter().any(|m| !Topology::build(m).consistently_oriented));
    }

    fn summary(mesh: &PolyMesh) -> (usize, usize, i64, bool, bool) {
        let t = Topology::build(mesh);
        (
            t.edge_count(),
            t.boundary_edge_count(),
            t.euler_characteristic(),
            t.is_closed(),
            t.is_manifold(),
        )
    }

    #[test]
    fn closed_shapes_have_the_expected_genus() {
        // (edges, boundary edges, Euler characteristic, closed, manifold)
        assert_eq!(summary(&shapes::cube(1.0)), (12, 0, 2, true, true));
        assert_eq!(summary(&shapes::cylinder(12)), (36, 0, 2, true, true));
        assert_eq!(
            summary(&shapes::torus(16, 8)),
            (256, 0, 0, true, true),
            "a torus has genus 1"
        );
    }

    #[test]
    fn open_shapes_have_borders() {
        assert_eq!(summary(&shapes::open_box()), (12, 4, 1, false, true));
        let grid = shapes::grid(4, 3);
        assert_eq!(summary(&grid), (31, 14, 1, false, true), "a disc; border = 2 × (4 + 3)");
        assert_eq!(summary(&shapes::l_shape()), (6, 6, 1, false, true));
    }

    #[test]
    fn adjacency_is_complete() {
        let cube = shapes::cube(1.0);
        let t = Topology::build(&cube);
        for p in 0..8 {
            assert_eq!(t.point_faces(p).len(), 3, "every cube corner touches three faces");
            assert_eq!(t.point_edges(p).len(), 3, "…and three edges");
            assert!(
                t.point_edges(p)
                    .iter()
                    .all(|&e| t.edges()[us(e)].contains(&u32_index(p)))
            );
        }
        for (e, &[a, b]) in t.edges().iter().enumerate() {
            assert!(a < b);
            for &f in t.edge_faces(e) {
                let face = cube.face(us(f));
                assert!(face.contains(&a) && face.contains(&b), "edge {e} listed on face {f}");
            }
        }
        for c in 0..cube.corner_count() {
            let [a, b] = t.edges()[us(t.corner_edge(c).unwrap())];
            assert!(cube.corner_points()[c] == a || cube.corner_points()[c] == b);
        }
        for e in 0..t.edge_count() {
            assert_eq!(t.edge_corners(e).len(), 2, "a closed edge has a corner in each face");
            assert!(
                t.edge_corners(e)
                    .iter()
                    .all(|&c| t.corner_edge(us(c)) == Some(u32_index(e)))
            );
        }
        assert!(t.point_faces(99).is_empty());
    }

    #[test]
    fn a_flipped_face_breaks_orientation() {
        let cube = shapes::cube(1.0);
        let faces: Vec<Vec<u32>> = cube
            .faces()
            .enumerate()
            .map(|(f, c)| {
                let mut c = c.to_vec();
                if f == 0 {
                    c.reverse();
                }
                c
            })
            .collect();
        let flipped = PolyMesh::from_faces(cube.points().to_vec(), faces).unwrap();
        let t = Topology::build(&flipped);
        assert!(t.is_closed(), "still closed by edge count");
        assert!(!t.is_consistently_oriented());
        assert!(!t.is_manifold());
    }

    #[test]
    fn three_faces_on_one_edge_are_non_manifold() {
        let points = vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z, Vec3::NEG_Y];
        // Three triangles share edge 0–1 (orientations made consistent where possible).
        let mesh = PolyMesh::from_faces(points, [[0u32, 1, 2], [1, 0, 3], [0, 4, 1]]).unwrap();
        let t = Topology::build(&mesh);
        assert_eq!(t.edge_faces(0).len(), 3);
        assert!(!t.is_manifold());
    }

    #[test]
    fn unused_points_do_not_count() {
        let mut mesh = shapes::cube(1.0);
        mesh.add_point(Vec3::splat(5.0)).unwrap();
        assert_eq!(Topology::build(&mesh).euler_characteristic(), 2);
        let empty = Topology::build(&PolyMesh::new());
        assert_eq!((empty.edge_count(), empty.euler_characteristic()), (0, 0));
        assert!(!empty.is_closed() && empty.is_manifold());
    }
}
