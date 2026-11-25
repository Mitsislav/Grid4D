//! Slide: points and edge loops moved along the edges next to them, so the
//! surface keeps its shape (spec `docs/reference/modeling-tools-2.md`).

use glam::Vec3;

use crate::{
    BitSet, MeshError, PolyMesh, Topology,
    loops::{self, WalkOptions},
    ops::{Rebuilt, loop_cut},
    u32_index, us,
};

/// How a slide moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlideOptions {
    /// How far along the neighbouring edges: 1 reaches the neighbour (−1 the
    /// one on the other side, for a loop).
    pub offset: f32,
    /// Stop at the neighbouring points.
    pub clamp: bool,
    /// Move along the curve through the neighbours, not the straight edge.
    pub keep_curvature: bool,
    /// Leave the loop where it is and slide a new one (a loop inserted).
    pub duplicate: bool,
}

impl Default for SlideOptions {
    fn default() -> Self {
        Self {
            offset: 0.0,
            clamp: true,
            keep_curvature: false,
            duplicate: false,
        }
    }
}

/// A point of a slide and the neighbours it slides towards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rail {
    pub point: u32,
    /// Towards positive offsets.
    pub ahead: Option<u32>,
    /// Towards negative offsets (a loop's other side; a point's opposite
    /// neighbour, if it has one).
    pub behind: Option<u32>,
}

/// The rails of single points: each point slides towards the neighbour whose
/// edge points most nearly along `direction` (object space), and back towards
/// the one most nearly against it.
#[must_use]
pub fn point_rails(mesh: &PolyMesh, topology: &Topology, points: &BitSet, direction: Vec3) -> Vec<Rail> {
    let direction = direction.normalize_or_zero();
    points
        .iter()
        .filter(|&p| p < mesh.point_count())
        .map(|p| {
            let at = mesh.points()[p];
            let neighbours = topology.point_edges(p).iter().filter_map(|&e| {
                let [a, b] = *topology.edges().get(us(e))?;
                let q = if us(a) == p { b } else { a };
                Some((q, (mesh.points()[us(q)] - at).normalize_or_zero().dot(direction)))
            });
            let mut ahead: Option<(u32, f32)> = None;
            let mut behind: Option<(u32, f32)> = None;
            for (q, along) in neighbours {
                if ahead.is_none_or(|(_, best)| along > best) {
                    ahead = Some((q, along));
                }
                if behind.is_none_or(|(_, best)| along < best) {
                    behind = Some((q, along));
                }
            }
            Rail {
                point: u32_index(p),
                ahead: ahead.map(|(q, _)| q),
                behind: behind
                    .filter(|&(q, _)| Some(q) != ahead.map(|(a, _)| a))
                    .map(|(q, _)| q),
            }
        })
        .collect()
}

/// The rails of the points of the edges `edges` (unique edge indices): each
/// point's edge across the loop on one side (`ahead`) and on the other
/// (`behind`). The sides are the two sides of the loop; "ahead" is the side of
/// the first face of the loop's lowest edge.
#[must_use]
pub fn loop_rails(mesh: &PolyMesh, topology: &Topology, edges: &BitSet) -> Vec<Rail> {
    let in_loop = |e: usize| edges.contains(e);
    let mut on_loop = BitSet::new();
    for e in edges.iter().filter(|&e| e < topology.edge_count()) {
        for p in topology.edges()[e] {
            on_loop.insert(us(p));
        }
    }
    // Faces touching the loop, grouped: faces sharing an edge off the loop
    // at a loop point lie on one side; faces sharing a loop edge on opposite
    // sides.
    let faces: Vec<usize> = {
        let mut set = BitSet::new();
        for p in on_loop.iter() {
            for &f in topology.point_faces(p) {
                set.insert(us(f));
            }
        }
        set.iter().collect()
    };
    let mut side: std::collections::HashMap<usize, bool> = std::collections::HashMap::new();
    let mut queue: Vec<usize> = Vec::new();
    let first = edges
        .iter()
        .find(|&e| e < topology.edge_count())
        .and_then(|e| topology.edge_faces(e).first().map(|&f| us(f)));
    let mut seeds: Vec<usize> = first.into_iter().collect();
    seeds.extend(faces.iter().copied());
    for seed in seeds {
        if side.contains_key(&seed) {
            continue;
        }
        side.insert(seed, true);
        queue.push(seed);
        while let Some(f) = queue.pop() {
            let here = side[&f];
            let corners = mesh.face(f);
            for (k, &a) in corners.iter().enumerate() {
                let b = corners[(k + 1) % corners.len()];
                if !on_loop.contains(us(a)) && !on_loop.contains(us(b)) {
                    continue;
                }
                let Some(e) = edge_between(topology, a, b) else {
                    continue;
                };
                let crossing = in_loop(e);
                for &g in topology.edge_faces(e) {
                    let g = us(g);
                    if g == f || faces.binary_search(&g).is_err() {
                        continue;
                    }
                    let wanted = if crossing { !here } else { here };
                    if let std::collections::hash_map::Entry::Vacant(slot) = side.entry(g) {
                        slot.insert(wanted);
                        queue.push(g);
                    }
                }
            }
        }
    }
    on_loop
        .iter()
        .map(|p| {
            let rail = |ahead: bool| {
                let candidates: Vec<u32> = topology
                    .point_edges(p)
                    .iter()
                    .filter(|&&e| !in_loop(us(e)))
                    .filter(|&&e| {
                        let faces = topology.edge_faces(us(e));
                        !faces.is_empty() && faces.iter().all(|&f| side.get(&us(f)) == Some(&ahead))
                    })
                    .map(|&e| {
                        let [a, b] = topology.edges()[us(e)];
                        if us(a) == p { b } else { a }
                    })
                    .collect();
                // Several (a point with more edges on that side): the one
                // nearest their average direction.
                let at = mesh.points()[p];
                let direction = candidates
                    .iter()
                    .map(|&q| (mesh.points()[us(q)] - at).normalize_or_zero())
                    .sum::<Vec3>();
                candidates.into_iter().max_by(|&x, &y| {
                    let along = |q: u32| (mesh.points()[us(q)] - at).normalize_or_zero().dot(direction);
                    along(x).total_cmp(&along(y))
                })
            };
            Rail {
                point: u32_index(p),
                ahead: rail(true),
                behind: rail(false),
            }
        })
        .collect()
}

/// The unique edge between points `a` and `b`.
fn edge_between(topology: &Topology, a: u32, b: u32) -> Option<usize> {
    topology.point_edges(us(a)).iter().map(|&e| us(e)).find(|&e| {
        topology
            .edges()
            .get(e)
            .is_some_and(|&[x, y]| (x == a && y == b) || (x == b && y == a))
    })
}

/// The points of `mesh` with every rail's point slid by `options.offset`
/// (towards `ahead` when positive, `behind` when negative; a point without
/// that neighbour stays).
#[must_use]
pub fn slide(mesh: &PolyMesh, rails: &[Rail], options: &SlideOptions) -> Vec<Vec3> {
    let mut points = mesh.points().to_vec();
    let offset = if options.clamp {
        options.offset.clamp(-1.0, 1.0)
    } else {
        options.offset
    };
    for rail in rails {
        let p = us(rail.point);
        let Some(&at) = mesh.points().get(p) else { continue };
        let ahead = rail.ahead.and_then(|q| mesh.points().get(us(q)).copied());
        let behind = rail.behind.and_then(|q| mesh.points().get(us(q)).copied());
        let moved = match (options.keep_curvature, ahead, behind) {
            // The parabola through behind (−1), here (0) and ahead (1).
            (true, Some(a), Some(b)) => at + (a - b) * (offset * 0.5) + (a + b - at * 2.0) * (offset * offset * 0.5),
            _ if offset >= 0.0 => ahead.map_or(at, |a| at + (a - at) * offset),
            _ => behind.map_or(at, |b| at + (b - at) * -offset),
        };
        if moved.is_finite() {
            points[p] = moved;
        }
    }
    points
}

/// Duplicate: a new loop beside the edges `edges`, at `offset` along their
/// rails (towards `ahead` when positive), the original loop kept. The new
/// loop's edges are [`Rebuilt::new_edges`]. Cuts the whole ring the rails
/// belong to.
pub fn slide_duplicate(
    mesh: &PolyMesh,
    topology: &Topology,
    rails: &[Rail],
    offset: f32,
) -> Result<Rebuilt, MeshError> {
    let ahead = offset >= 0.0;
    let t = offset.abs().clamp(0.0, 1.0);
    let rail = rails.iter().find_map(|r| {
        let q = if ahead { r.ahead } else { r.behind }?;
        Some((r.point, q))
    });
    let Some((p, q)) = rail else {
        return loop_cut(mesh, topology, &loops::Ring::default(), 0, &[]);
    };
    let Some(start) = edge_between(topology, p, q) else {
        return loop_cut(mesh, topology, &loops::Ring::default(), 0, &[]);
    };
    let ring = loops::edge_ring(
        mesh,
        topology,
        start,
        WalkOptions::default(),
        &BitSet::new(),
        &BitSet::new(),
    );
    // The cut is measured from the start edge's first point.
    let from_first = if topology.edges()[start][0] == p { t } else { 1.0 - t };
    loop_cut(mesh, topology, &ring, start, &[from_first])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    /// A 100 × 100 plane of 4 × 4 quads and the edges of its middle row
    /// (z = 0, across x).
    fn plane_and_loop() -> (PolyMesh, Topology, BitSet) {
        let plane = primitives::plane(100.0, 100.0, 4, 4);
        let topology = Topology::build(&plane);
        let middle: BitSet = topology
            .edges()
            .iter()
            .enumerate()
            .filter(|(_, [a, b])| {
                let (a, b) = (plane.points()[us(*a)], plane.points()[us(*b)]);
                a.z.abs() < 1e-3 && b.z.abs() < 1e-3
            })
            .map(|(e, _)| e)
            .collect();
        (plane, topology, middle)
    }

    #[test]
    fn a_loop_slid_halfway_lies_halfway_on_every_rail() {
        let (plane, topology, middle) = plane_and_loop();
        let rails = loop_rails(&plane, &topology, &middle);
        assert_eq!(rails.len(), 5);
        let ahead_z = plane.points()[us(rails[0].ahead.unwrap())].z;
        for rail in &rails {
            assert!(rail.ahead.is_some() && rail.behind.is_some(), "both sides");
            assert_eq!(plane.points()[us(rail.ahead.unwrap())].z, ahead_z, "one side for all");
        }
        let moved = slide(
            &plane,
            &rails,
            &SlideOptions {
                offset: 0.5,
                ..SlideOptions::default()
            },
        );
        for rail in &rails {
            let p = moved[us(rail.point)];
            assert!((p.z - ahead_z * 0.5).abs() < 1e-4, "halfway: {p}");
        }
        // Clamped at the neighbours; free beyond them.
        let far = SlideOptions {
            offset: -3.0,
            ..SlideOptions::default()
        };
        let clamped = slide(&plane, &rails, &far);
        assert!((clamped[us(rails[0].point)].z + ahead_z).abs() < 1e-4);
        let free = slide(&plane, &rails, &SlideOptions { clamp: false, ..far });
        assert!((free[us(rails[0].point)].z + ahead_z * 3.0).abs() < 1e-3);
    }

    #[test]
    fn keep_curvature_follows_the_arc() {
        // Points on a circle: the middle one slides along the parabola, off
        // the straight edge.
        let points = vec![
            Vec3::new(-1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
        ];
        let mesh = PolyMesh::from_faces(points, [vec![0u32, 2, 1]]).unwrap();
        let rail = Rail {
            point: 1,
            ahead: Some(2),
            behind: Some(0),
        };
        let options = SlideOptions {
            offset: 0.5,
            keep_curvature: true,
            ..SlideOptions::default()
        };
        let curved = slide(&mesh, &[rail], &options)[1];
        let straight = slide(
            &mesh,
            &[rail],
            &SlideOptions {
                keep_curvature: false,
                ..options
            },
        )[1];
        assert!(curved.y > straight.y, "{curved} above {straight}");
        let end = slide(&mesh, &[rail], &SlideOptions { offset: 1.0, ..options })[1];
        assert!(end.distance(Vec3::X) < 1e-5, "reaches the neighbour");
    }

    #[test]
    fn a_point_slides_along_the_edge_nearest_the_direction() {
        let (plane, topology, _) = plane_and_loop();
        let centre = plane.points().iter().position(|p| p.length() < 1e-3).unwrap();
        let points: BitSet = [centre].into_iter().collect();
        let rails = point_rails(&plane, &topology, &points, Vec3::new(1.0, 0.0, 0.2));
        let ahead = plane.points()[us(rails[0].ahead.unwrap())];
        assert!((ahead - Vec3::new(25.0, 0.0, 0.0)).length() < 1e-3);
        assert!((plane.points()[us(rails[0].behind.unwrap())] - Vec3::new(-25.0, 0.0, 0.0)).length() < 1e-3);
    }

    #[test]
    fn duplicate_inserts_a_loop_and_keeps_the_original() {
        let (plane, topology, middle) = plane_and_loop();
        let rails = loop_rails(&plane, &topology, &middle);
        let ahead_z = plane.points()[us(rails[0].ahead.unwrap())].z;
        let rebuilt = slide_duplicate(&plane, &topology, &rails, 0.25).unwrap();
        assert_eq!(rebuilt.mesh.face_count(), plane.face_count() + 4);
        assert_eq!(rebuilt.new_edges.len(), 4, "the new loop's edges");
        for [a, b] in &rebuilt.new_edges {
            for p in [a, b] {
                assert!((rebuilt.mesh.points()[us(*p)].z - ahead_z * 0.25).abs() < 1e-4);
            }
        }
        for rail in &rails {
            assert_eq!(
                rebuilt.mesh.points()[us(rail.point)],
                plane.points()[us(rail.point)],
                "kept"
            );
        }
    }
}
