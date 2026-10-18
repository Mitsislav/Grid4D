use std::collections::{HashMap, HashSet};

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
    point_face_offsets: Vec<u32>,
    point_face_list: Vec<u32>,
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
        let mut edge_index: HashMap<[u32; 2], u32> = HashMap::with_capacity(mesh.corner_count());
        let mut directed: HashSet<[u32; 2]> = HashSet::with_capacity(mesh.corner_count());
        let mut edges = Vec::new();
        let mut corner_edges = Vec::with_capacity(mesh.corner_count());
        let mut consistently_oriented = true;

        for face in mesh.faces() {
            for (i, &a) in face.iter().enumerate() {
                let b = face[(i + 1) % face.len()];
                consistently_oriented &= directed.insert([a, b]);
                let key = [a.min(b), a.max(b)];
                let edge = *edge_index.entry(key).or_insert_with(|| {
                    edges.push(key);
                    u32_index(edges.len() - 1)
                });
                corner_edges.push(edge);
            }
        }

        // Faces per edge.
        let (edge_face_offsets, edge_face_list) = pack(edges.len(), |emit| {
            for (f, range) in (0..mesh.face_count()).map(|f| (f, mesh.corner_range(f))) {
                for c in range {
                    emit(us(corner_edges[c]), u32_index(f));
                }
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
        let used_points = point_face_offsets.windows(2).filter(|w| w[1] > w[0]).count();

        Self {
            edges,
            corner_edges,
            edge_face_offsets,
            edge_face_list,
            point_face_offsets,
            point_face_list,
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

    /// Faces that use edge `edge` (empty if out of range).
    #[must_use]
    pub fn edge_faces(&self, edge: usize) -> &[u32] {
        slice(&self.edge_face_offsets, &self.edge_face_list, edge)
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
