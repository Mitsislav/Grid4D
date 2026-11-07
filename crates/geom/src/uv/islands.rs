//! How a mesh's corners join in UV space (see
//! `docs/reference/manager-uv-editor.md`): **UV vertices** (the corners of one
//! point joined across edges whose two sides agree in UV), **islands** (faces
//! joined that way), sharp edges for seams, and how much the UVs stretch the
//! surface.

use glam::{Vec2, Vec3};

use crate::{BitSet, PolyMesh, Topology, u32_index, us};

/// Corners closer than this in UV are at one place.
pub(crate) const SAME_UV: f32 = 1e-5;

/// A union-find over `n` items.
pub(crate) struct Groups {
    parent: Vec<u32>,
}

impl Groups {
    pub(crate) fn new(n: usize) -> Self {
        Self {
            parent: (0..n).map(u32_index).collect(),
        }
    }

    pub(crate) fn find(&mut self, mut i: usize) -> usize {
        while us(self.parent[i]) != i {
            let grand = self.parent[us(self.parent[i])];
            self.parent[i] = grand;
            i = us(grand);
        }
        i
    }

    pub(crate) fn join(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[ra.max(rb)] = u32_index(ra.min(rb));
        }
    }

    /// Numbers the groups of the items `keep` says, in order of first item;
    /// `u32::MAX` for the others. Returns the numbers and how many there are.
    pub(crate) fn number(&mut self, keep: impl Fn(usize) -> bool) -> (Vec<u32>, usize) {
        let n = self.parent.len();
        let mut of_root = vec![u32::MAX; n];
        let mut out = vec![u32::MAX; n];
        let mut count = 0;
        for (i, slot) in out.iter_mut().enumerate() {
            if !keep(i) {
                continue;
            }
            let root = self.find(i);
            if of_root[root] == u32::MAX {
                of_root[root] = u32_index(count);
                count += 1;
            }
            *slot = of_root[root];
        }
        (out, count)
    }
}

/// The face each corner of `mesh` belongs to.
pub(crate) fn corner_faces(mesh: &PolyMesh) -> Vec<u32> {
    let mut out = vec![0; mesh.corner_count()];
    for f in 0..mesh.face_count() {
        for c in mesh.corner_range(f) {
            out[c] = u32_index(f);
        }
    }
    out
}

/// The corner after `corner` in its face (`range`: the face's corners).
pub(crate) fn next_corner(corner: usize, range: &std::ops::Range<usize>) -> usize {
    if corner + 1 < range.end {
        corner + 1
    } else {
        range.start
    }
}

/// Corners and faces grouped across edges `joins` accepts.
pub(crate) struct Grouping {
    /// The vertex of each corner (`u32::MAX` outside the faces grouped).
    pub vertex_of: Vec<u32>,
    pub vertex_count: usize,
    /// The group of each face (`u32::MAX` outside).
    pub group_of: Vec<u32>,
    pub group_count: usize,
    /// Corners whose edge (to the next corner) joins no other face: borders.
    pub border: BitSet,
}

/// Groups the faces `inside` accepts: across each edge two of them share,
/// when `joins(edge, [this face's corners at the edge's two points], [the
/// other's])` says so, their faces join and so do their corners at each point.
pub(crate) fn group(
    mesh: &PolyMesh,
    topology: &Topology,
    inside: impl Fn(usize) -> bool,
    joins: impl Fn(usize, [usize; 2], [usize; 2]) -> bool,
) -> Grouping {
    let faces = corner_faces(mesh);
    let points = mesh.corner_points();
    let mut corners = Groups::new(mesh.corner_count());
    let mut groups = Groups::new(mesh.face_count());
    let mut joined = BitSet::new();
    for edge in 0..topology.edge_count() {
        let list = topology.edge_corners(edge);
        let Some((&first, rest)) = list.split_first() else {
            continue;
        };
        let first = us(first);
        let f1 = us(faces[first]);
        if !inside(f1) {
            continue;
        }
        let r1 = mesh.corner_range(f1);
        // This face's corners at the edge's start and end points.
        let a1 = [first, next_corner(first, &r1)];
        for &other in rest {
            let other = us(other);
            let f2 = us(faces[other]);
            if f2 == f1 || !inside(f2) {
                continue;
            }
            let r2 = mesh.corner_range(f2);
            let ends = [other, next_corner(other, &r2)];
            // The other face's corners at the same two points.
            let a2 = if points[ends[0]] == points[a1[0]] {
                ends
            } else {
                [ends[1], ends[0]]
            };
            if !joins(edge, a1, a2) {
                continue;
            }
            groups.join(f1, f2);
            corners.join(a1[0], a2[0]);
            corners.join(a1[1], a2[1]);
            joined.insert(first);
            joined.insert(other);
        }
    }
    let (vertex_of, vertex_count) = corners.number(|c| inside(us(faces[c])));
    let (group_of, group_count) = groups.number(&inside);
    let mut border = BitSet::new();
    for (c, &f) in faces.iter().enumerate() {
        if inside(us(f)) && !joined.contains(c) {
            border.insert(c);
        }
    }
    Grouping {
        vertex_of,
        vertex_count,
        group_of,
        group_count,
        border,
    }
}

/// How a mesh's corners and faces join in UV space.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UvLayout {
    /// The UV vertex of each corner: the corners of one point that are joined
    /// across edges whose two sides agree in UV.
    pub vertex_of: Vec<u32>,
    pub vertex_count: usize,
    /// The island of each face: faces joined across such edges.
    pub island_of: Vec<u32>,
    pub island_count: usize,
}

impl UvLayout {
    /// The corners of each island, in corner order.
    #[must_use]
    pub fn island_corners(&self, mesh: &PolyMesh) -> Vec<Vec<usize>> {
        let mut out = vec![Vec::new(); self.island_count];
        for f in 0..mesh.face_count() {
            if let Some(list) = self.island_of.get(f).and_then(|&i| out.get_mut(us(i))) {
                list.extend(mesh.corner_range(f));
            }
        }
        out
    }
}

/// The UV vertices and islands of `mesh` (without UVs: one island per
/// connected part, a vertex per point of it).
#[must_use]
pub fn layout(mesh: &PolyMesh, topology: &Topology) -> UvLayout {
    let uvs = mesh.uvs();
    let grouping = group(
        mesh,
        topology,
        |_| true,
        |_, a, b| {
            uvs.is_none_or(|uvs| uvs[a[0]].distance(uvs[b[0]]) <= SAME_UV && uvs[a[1]].distance(uvs[b[1]]) <= SAME_UV)
        },
    );
    UvLayout {
        vertex_of: grouping.vertex_of,
        vertex_count: grouping.vertex_count,
        island_of: grouping.group_of,
        island_count: grouping.group_count,
    }
}

/// The edges of `mesh` (by [`Topology`] index) between two faces that meet at
/// more than `angle_deg` degrees.
#[must_use]
pub fn sharp_edges(mesh: &PolyMesh, topology: &Topology, angle_deg: f32) -> BitSet {
    let cos = angle_deg.to_radians().cos();
    let normals: Vec<Vec3> = (0..mesh.face_count()).map(|f| mesh.face_normal(f)).collect();
    (0..topology.edge_count())
        .filter(|&e| match topology.edge_faces(e) {
            [a, b] => normals[us(*a)].dot(normals[us(*b)]) < cos,
            _ => false,
        })
        .collect()
}

/// Twice the signed area of a face's UV polygon (positive: unmirrored, see
/// [`winding`](super::winding)).
pub(crate) fn uv_area(uvs: &[Vec2], range: std::ops::Range<usize>) -> f32 {
    super::winding(&uvs[range]) * 0.5
}

/// How much the UVs stretch each face against its island: the base-2
/// logarithm of the face's UV area per unit of surface over its island's
/// (0: as the island on average, +1: twice as large in UV, −1: half). Faces
/// without area, or a mesh without UVs, give 0.
#[must_use]
pub fn face_stretch(mesh: &PolyMesh, topology: &Topology) -> Vec<f32> {
    let Some(uvs) = mesh.uvs() else {
        return vec![0.0; mesh.face_count()];
    };
    let layout = layout(mesh, topology);
    let mut sums = vec![(0.0f64, 0.0f64); layout.island_count];
    let areas: Vec<(f32, f32)> = (0..mesh.face_count())
        .map(|f| (uv_area(uvs, mesh.corner_range(f)).abs(), mesh.face_area(f)))
        .collect();
    for (f, &(uv, surface)) in areas.iter().enumerate() {
        if let Some(sum) = sums.get_mut(us(layout.island_of[f])) {
            sum.0 += f64::from(uv);
            sum.1 += f64::from(surface);
        }
    }
    areas
        .iter()
        .enumerate()
        .map(|(f, &(uv, surface))| {
            let Some(&(island_uv, island_surface)) = sums.get(us(layout.island_of[f])) else {
                return 0.0;
            };
            if uv <= 0.0 || surface <= 0.0 || island_uv <= 0.0 || island_surface <= 0.0 {
                return 0.0;
            }
            let ratio = f64::from(uv) / f64::from(surface) / (island_uv / island_surface);
            ratio.log2() as f32
        })
        .collect()
}

/// One number for how unevenly the UVs stretch the surface: the mean squared
/// difference between each polygon edge's UV length and its surface length
/// times its island's scale (the square root of UV area per unit of surface),
/// relative to the latter. 0 for a perfect, undistorted layout.
#[must_use]
pub fn stretch_measure(mesh: &PolyMesh, topology: &Topology) -> f32 {
    let Some(uvs) = mesh.uvs() else {
        return 0.0;
    };
    let layout = layout(mesh, topology);
    let mut sums = vec![(0.0f64, 0.0f64); layout.island_count];
    for f in 0..mesh.face_count() {
        if let Some(sum) = sums.get_mut(us(layout.island_of[f])) {
            sum.0 += f64::from(uv_area(uvs, mesh.corner_range(f)).abs());
            sum.1 += f64::from(mesh.face_area(f));
        }
    }
    let points = mesh.points();
    let corner_points = mesh.corner_points();
    let (mut total, mut count) = (0.0f64, 0usize);
    for f in 0..mesh.face_count() {
        let Some(&(island_uv, island_surface)) = sums.get(us(layout.island_of[f])) else {
            continue;
        };
        if island_uv <= 0.0 || island_surface <= 0.0 {
            continue;
        }
        let scale = (island_uv / island_surface).sqrt();
        let range = mesh.corner_range(f);
        for c in range.clone() {
            let n = next_corner(c, &range);
            let surface = f64::from(points[us(corner_points[c])].distance(points[us(corner_points[n])])) * scale;
            if surface <= 0.0 {
                continue;
            }
            let uv = f64::from(uvs[c].distance(uvs[n]));
            total += ((uv - surface) / surface).powi(2);
            count += 1;
        }
    }
    if count == 0 { 0.0 } else { (total / count as f64) as f32 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    #[test]
    fn a_cube_s_box_uvs_make_six_islands_and_its_corners_join_within_them() {
        let cube = primitives::cube(Vec3::splat(2.0), [1, 1, 1]);
        let topology = Topology::build(&cube);
        let layout = layout(&cube, &topology);
        assert_eq!(layout.island_count, 6, "one per side");
        assert_eq!(layout.vertex_count, 24, "four corners per side, none shared");
        let corners = layout.island_corners(&cube);
        assert!(corners.iter().all(|c| c.len() == 4));
        // Without UVs, one island and a vertex per point.
        let mut plain = cube;
        plain.set_uvs(None).unwrap();
        let layout = super::layout(&plain, &topology);
        assert_eq!((layout.island_count, layout.vertex_count), (1, 8));
    }

    #[test]
    fn a_plane_s_grid_is_one_island_and_sharp_edges_are_found() {
        let plane = primitives::plane(2.0, 2.0, 3, 3);
        let topology = Topology::build(&plane);
        let layout = layout(&plane, &topology);
        assert_eq!((layout.island_count, layout.vertex_count), (1, 16));
        assert!(sharp_edges(&plane, &topology, 60.0).is_empty());
        let cube = primitives::cube(Vec3::splat(2.0), [1, 1, 1]);
        let topology = Topology::build(&cube);
        assert_eq!(sharp_edges(&cube, &topology, 60.0).len(), 12);
        assert_eq!(sharp_edges(&cube, &topology, 95.0).len(), 0);
    }

    #[test]
    fn stretch_is_zero_for_an_even_layout_and_grows_when_uvs_are_squeezed() {
        let mut plane = primitives::plane(2.0, 2.0, 2, 2);
        let topology = Topology::build(&plane);
        assert!(stretch_measure(&plane, &topology) < 1e-6);
        assert!(face_stretch(&plane, &topology).iter().all(|s| s.abs() < 1e-5));
        // Squeeze the left column of faces to half their width in UV.
        let uvs: Vec<Vec2> = plane
            .uvs()
            .unwrap()
            .iter()
            .map(|uv| {
                if uv.x <= 0.5 {
                    Vec2::new(uv.x * 0.5, uv.y)
                } else {
                    Vec2::new(0.25 + (uv.x - 0.5) * 1.5, uv.y)
                }
            })
            .collect();
        plane.set_uvs(Some(uvs)).unwrap();
        assert!(stretch_measure(&plane, &topology) > 0.01);
        let stretch = face_stretch(&plane, &topology);
        assert!(
            stretch.iter().any(|&s| s < -0.5) && stretch.iter().any(|&s| s > 0.3),
            "{stretch:?}"
        );
    }
}
