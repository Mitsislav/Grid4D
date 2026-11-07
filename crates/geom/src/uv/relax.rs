//! Relaxing UVs (see `docs/reference/manager-uv-editor.md`): each UV vertex
//! is pulled so the UV edges around it take the lengths their surface edges
//! have (at the island's scale), a little each round; an island's border
//! stays put unless asked, and no move ever turns a face over.

use glam::Vec2;

use super::islands::{SAME_UV, group, next_corner, uv_area};
use crate::{BitSet, PolyMesh, Topology, us};

/// How [`relax`] works.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RelaxOptions {
    /// Rounds (1 – 100).
    pub iterations: u32,
    /// Island borders stay where they are.
    pub pin_borders: bool,
}

impl Default for RelaxOptions {
    fn default() -> Self {
        Self {
            iterations: 20,
            pin_borders: true,
        }
    }
}

/// The corner UVs of `mesh` with the UV vertices of the faces `faces` (all
/// when empty) relaxed. A mesh without UVs gets a Box projection's first.
#[must_use]
pub fn relax(mesh: &PolyMesh, topology: &Topology, faces: &BitSet, options: &RelaxOptions) -> Vec<Vec2> {
    let mut uvs = mesh.uvs().map_or_else(|| super::box_uvs(mesh), <[Vec2]>::to_vec);
    let grouping = group(
        mesh,
        topology,
        |_| true,
        |_, a, b| uvs[a[0]].distance(uvs[b[0]]) <= SAME_UV && uvs[a[1]].distance(uvs[b[1]]) <= SAME_UV,
    );
    let n = grouping.vertex_count;
    let points = mesh.points();
    let corner_points = mesh.corner_points();
    let mut at = vec![Vec2::ZERO; n];
    for (c, &v) in grouping.vertex_of.iter().enumerate() {
        at[us(v)] = uvs[c];
    }

    // Each island's scale: UV length per unit of surface length.
    let mut sums = vec![(0.0f32, 0.0f32); grouping.group_count];
    for f in 0..mesh.face_count() {
        let sum = &mut sums[us(grouping.group_of[f])];
        sum.0 += uv_area(&uvs, mesh.corner_range(f)).abs();
        sum.1 += mesh.face_area(f);
    }
    let scale: Vec<f32> = sums
        .iter()
        .map(|&(uv, surface)| if surface > 0.0 { (uv / surface).sqrt() } else { 0.0 })
        .collect();

    // Springs between UV vertices (each polygon edge once per side), the
    // faces around each vertex, and which vertices may move.
    let mut springs: Vec<Vec<(usize, f32)>> = vec![Vec::new(); n];
    let mut around: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut movable = vec![true; n];
    for f in 0..mesh.face_count() {
        let range = mesh.corner_range(f);
        let chosen = faces.is_empty() || faces.contains(f);
        let s = scale[us(grouping.group_of[f])];
        for c in range.clone() {
            let next = next_corner(c, &range);
            let (v, w) = (us(grouping.vertex_of[c]), us(grouping.vertex_of[next]));
            let rest = points[us(corner_points[c])].distance(points[us(corner_points[next])]) * s;
            if !springs[v].iter().any(|&(o, _)| o == w) {
                springs[v].push((w, rest));
                springs[w].push((v, rest));
            }
            around[v].push(f);
            // Vertices of faces not chosen, and both ends of a border edge,
            // stay.
            if !chosen {
                movable[v] = false;
            }
            if options.pin_borders && grouping.border.contains(c) {
                movable[v] = false;
                movable[w] = false;
            }
        }
    }

    let vertex_of = &grouping.vertex_of;
    let face_area = |at: &[Vec2], f: usize| {
        let corners: Vec<Vec2> = mesh.corner_range(f).map(|c| at[us(vertex_of[c])]).collect();
        super::winding(&corners)
    };
    for _ in 0..options.iterations.clamp(1, 100) {
        for v in 0..n {
            if !movable[v] || springs[v].is_empty() {
                continue;
            }
            let mut pull = Vec2::ZERO;
            for &(o, rest) in &springs[v] {
                let d = at[o] - at[v];
                let length = d.length();
                if length > f32::MIN_POSITIVE {
                    pull += d * ((length - rest) / length);
                }
            }
            let step = pull * (0.5 / springs[v].len() as f32);
            let before: Vec<f32> = around[v].iter().map(|&f| face_area(&at, f)).collect();
            let start = at[v];
            let mut moved = false;
            for fraction in [1.0, 0.5, 0.25, 0.125] {
                at[v] = start + step * fraction;
                let keeps = around[v]
                    .iter()
                    .zip(&before)
                    .all(|(&f, &was)| was <= 0.0 || face_area(&at, f) > 0.0);
                if keeps {
                    moved = true;
                    break;
                }
            }
            if !moved {
                at[v] = start;
            }
        }
    }
    for (c, uv) in uvs.iter_mut().enumerate() {
        *uv = at[us(vertex_of[c])];
    }
    uvs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        primitives,
        uv::{flipped_faces, stretch_measure},
    };

    /// A 6 × 6 grid whose inner UV points are pushed about.
    fn jittered() -> PolyMesh {
        let mut plane = primitives::plane(6.0, 6.0, 6, 6);
        let uvs: Vec<Vec2> = plane
            .uvs()
            .unwrap()
            .iter()
            .map(|&uv| {
                let inner = uv.x > 0.01 && uv.x < 0.99 && uv.y > 0.01 && uv.y < 0.99;
                if inner {
                    let wobble = ((uv.x * 37.0).sin() * 0.04, (uv.y * 23.0 + uv.x * 11.0).cos() * 0.04);
                    uv + Vec2::new(wobble.0, wobble.1)
                } else {
                    uv
                }
            })
            .collect();
        plane.set_uvs(Some(uvs)).unwrap();
        plane
    }

    #[test]
    fn relax_lowers_the_stretch_never_flips_and_keeps_borders() {
        let mut plane = jittered();
        let topology = Topology::build(&plane);
        assert_eq!(flipped_faces(&plane), 0);
        let before = stretch_measure(&plane, &topology);
        let original = plane.uvs().unwrap().to_vec();
        let relaxed = relax(&plane, &topology, &BitSet::new(), &RelaxOptions::default());
        plane.set_uvs(Some(relaxed.clone())).unwrap();
        let after = stretch_measure(&plane, &topology);
        assert!(after < before * 0.5, "{before} → {after}");
        assert_eq!(flipped_faces(&plane), 0);
        for (a, b) in original.iter().zip(&relaxed) {
            let border = a.x < 0.01 || a.x > 0.99 || a.y < 0.01 || a.y > 0.99;
            if border {
                assert_eq!(a, b, "borders stay");
            }
        }
    }

    #[test]
    fn relax_moves_only_the_chosen_faces() {
        let plane = jittered();
        let topology = Topology::build(&plane);
        let first: BitSet = [0].into_iter().collect();
        let relaxed = relax(&plane, &topology, &first, &RelaxOptions::default());
        // Face 0's points all touch other faces or the border: nothing moves.
        assert_eq!(relaxed, plane.uvs().unwrap());
        // Without pinned borders the outline moves too.
        let free = RelaxOptions {
            iterations: 5,
            pin_borders: false,
        };
        let moved = relax(&plane, &topology, &BitSet::new(), &free);
        assert!(moved.iter().zip(plane.uvs().unwrap()).any(|(a, b)| a != b));
    }
}
