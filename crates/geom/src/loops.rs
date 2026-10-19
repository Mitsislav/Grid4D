//! Loops and rings: the chains of edges (and the strips of polygons between
//! them) that loop and ring selection follow.
//!
//! * A **loop** runs *along* edges, from vertex to vertex: at a regular vertex
//!   (four faces, four edges) it continues with the edge on the far side; along a
//!   border it follows the border.
//! * A **ring** runs *across* faces: from an edge of a quad it continues with the
//!   quad's opposite edge, then into the next quad — every vertical edge of a
//!   cylinder, say. The quads crossed form a polygon strip.

use glam::Vec3;

use crate::{BitSet, PolyMesh, Topology, us};

/// Where loops and rings stop besides borders and non-manifold edges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WalkOptions {
    /// Stop at components that are already selected (use them as fences).
    pub stop_at_selections: bool,
    /// Loop: stop at triangles and n-gons. Ring: cross only quads (otherwise it
    /// also turns around triangle fans).
    pub stop_at_non_quads: bool,
    /// Loop: stop at points that do not have four faces (two on a border); when
    /// off, go on as straight as possible through poles surrounded by quads.
    /// Ring: do not turn around a point surrounded only by triangles (a sphere pole).
    pub stop_at_poles: bool,
}

/// The edge loop through unique edge `start` (indices into [`Topology::edges`]).
/// With `stop_at_selections`, the walk stops before edges in `stop_edges` and at
/// points in `stop_points`. Always stops at borders it runs into, non-manifold
/// edges and where the loop closes.
#[must_use]
pub fn edge_loop(
    mesh: &PolyMesh,
    topology: &Topology,
    start: usize,
    options: WalkOptions,
    stop_edges: &BitSet,
    stop_points: &BitSet,
) -> BitSet {
    let mut result = BitSet::new();
    let Some(&[a, b]) = topology.edges().get(start) else {
        return result;
    };
    result.insert(start);
    for (mut edge, mut at) in [(start, b), (start, a)] {
        loop {
            if options.stop_at_selections && stop_points.contains(us(at)) {
                break;
            }
            let Some(next) = next_loop_edge(mesh, topology, edge, at, options) else {
                break;
            };
            if result.contains(next) || (options.stop_at_selections && stop_edges.contains(next)) {
                break; // closed, or fenced off
            }
            result.insert(next);
            at = other_end(topology, next, at);
            edge = next;
        }
    }
    result
}

/// The edge after `edge` in its loop, leaving through point `at`.
fn next_loop_edge(mesh: &PolyMesh, topology: &Topology, edge: usize, at: u32, options: WalkOptions) -> Option<usize> {
    let faces = topology.edge_faces(edge);
    let around = topology.point_edges(us(at));
    let is_border = |e: usize| topology.edge_faces(e).len() == 1;
    let quads = |e: usize| topology.edge_faces(e).iter().all(|&f| mesh.face(us(f)).len() == 4);
    match faces.len() {
        // Along a border: the other border edge at the point, if there is exactly one.
        1 => {
            let mut border = around.iter().map(|&e| us(e)).filter(|&e| e != edge && is_border(e));
            let next = border.next()?;
            border.next().is_none().then_some(next)
        }
        2 => {
            // The edges at `at` on the far side: they share no face with `edge`.
            let mut far = around
                .iter()
                .map(|&e| us(e))
                .filter(|&e| e != edge && !topology.edge_faces(e).iter().any(|f| faces.contains(f)));
            let point_faces = topology.point_faces(us(at));
            let regular = point_faces.len() == 4 && around.len() == 4 && !around.iter().any(|&e| is_border(us(e)));
            let next = if regular {
                let next = far.next()?;
                if far.next().is_some() {
                    return None;
                }
                next
            } else {
                // A pole. Stop, or go on as straight as possible — only through
                // quads, and never along a border.
                if options.stop_at_poles
                    || around.iter().any(|&e| is_border(us(e)))
                    || point_faces.iter().any(|&f| mesh.face(us(f)).len() != 4)
                {
                    return None;
                }
                let points = mesh.points();
                let here = points[us(at)];
                let incoming = (here - points[us(other_end(topology, edge, at))]).normalize_or_zero();
                let straightness =
                    |e: usize| incoming.dot((points[us(other_end(topology, e, at))] - here).normalize_or_zero());
                let best = far.max_by(|&x, &y| straightness(x).total_cmp(&straightness(y)))?;
                if straightness(best) <= 0.0 {
                    return None;
                }
                best
            };
            if options.stop_at_non_quads && !(quads(edge) && quads(next)) {
                return None;
            }
            Some(next)
        }
        _ => None, // loose or non-manifold
    }
}

fn other_end(topology: &Topology, edge: usize, point: u32) -> u32 {
    let [a, b] = topology.edges()[edge];
    if a == point { b } else { a }
}

/// An edge ring and the polygons it crosses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ring {
    /// Unique edge indices.
    pub edges: BitSet,
    /// The polygons between consecutive ring edges: a polygon strip.
    pub faces: BitSet,
}

/// The edge ring through unique edge `start`, in both directions. With
/// `stop_at_selections` it stops before edges in `stop_edges` and polygons in
/// `stop_faces`. Always stops at borders, n-gons and where the ring closes.
#[must_use]
pub fn edge_ring(
    mesh: &PolyMesh,
    topology: &Topology,
    start: usize,
    options: WalkOptions,
    stop_edges: &BitSet,
    stop_faces: &BitSet,
) -> Ring {
    let mut ring = Ring::default();
    if start >= topology.edge_count() {
        return ring;
    }
    ring.edges.insert(start);
    for &first in topology.edge_faces(start) {
        let (mut edge, mut face, mut pivot) = (start, us(first), None);
        loop {
            if options.stop_at_selections && stop_faces.contains(face) {
                break;
            }
            let Some(next) = across(mesh, topology, edge, face, &mut pivot, options) else {
                break;
            };
            ring.faces.insert(face);
            if ring.edges.contains(next) || (options.stop_at_selections && stop_edges.contains(next)) {
                break; // closed, or fenced off
            }
            ring.edges.insert(next);
            let faces = topology.edge_faces(next);
            if faces.len() != 2 {
                break; // a border
            }
            face = us(if us(faces[0]) == face { faces[1] } else { faces[0] });
            edge = next;
        }
    }
    ring
}

/// The ring's next edge across `face`, entered through `edge`. `pivot` is the
/// point a triangle fan is being walked around.
fn across(
    mesh: &PolyMesh,
    topology: &Topology,
    edge: usize,
    face: usize,
    pivot: &mut Option<u32>,
    options: WalkOptions,
) -> Option<usize> {
    let [a, b] = topology.edges()[edge];
    let touches = |e: usize, p: u32| topology.edges()[e].contains(&p);
    let mut edges = mesh.corner_range(face).filter_map(|c| topology.corner_edge(c)).map(us);
    match mesh.face(face).len() {
        // The opposite edge: it shares no point with `edge`.
        4 => edges.find(|&e| !touches(e, a) && !touches(e, b)),
        3 if !options.stop_at_non_quads => {
            let centre = match *pivot {
                Some(p) => p,
                None => {
                    // The fan's centre: the end of `edge` surrounded only by triangles.
                    let fan = |p: u32| topology.point_faces(us(p)).iter().all(|&f| mesh.face(us(f)).len() == 3);
                    match (fan(a), fan(b)) {
                        (true, false) => a,
                        (false, true) => b,
                        _ => return None,
                    }
                }
            };
            if options.stop_at_poles {
                return None;
            }
            *pivot = Some(centre);
            edges.find(|&e| e != edge && touches(e, centre))
        }
        _ => None,
    }
}

/// Direction of unique edge `edge`, for tests and callers choosing a strip.
#[must_use]
pub fn edge_vector(mesh: &PolyMesh, topology: &Topology, edge: usize) -> Vec3 {
    topology
        .edges()
        .get(edge)
        .map_or(Vec3::ZERO, |&[a, b]| mesh.points()[us(b)] - mesh.points()[us(a)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{primitives, shapes};

    /// The first unique edge whose ends satisfy `pick`.
    fn find_edge(mesh: &PolyMesh, topology: &Topology, pick: impl Fn(Vec3, Vec3) -> bool) -> usize {
        topology
            .edges()
            .iter()
            .position(|&[a, b]| pick(mesh.points()[us(a)], mesh.points()[us(b)]))
            .unwrap()
    }

    fn none() -> BitSet {
        BitSet::new()
    }

    #[test]
    fn a_sphere_equator_loop_goes_all_the_way_round() {
        let sphere = primitives::sphere(1.0, 16);
        let topology = Topology::build(&sphere);
        let equator = find_edge(&sphere, &topology, |a, b| a.y.abs() < 1e-5 && b.y.abs() < 1e-5);
        let edges = edge_loop(&sphere, &topology, equator, WalkOptions::default(), &none(), &none());
        assert_eq!(edges.len(), 16);
    }

    #[test]
    fn a_meridian_loop_stops_at_the_poles() {
        let sphere = primitives::sphere(1.0, 16); // 8 rows: 6 quad rows + 2 triangle fans
        let topology = Topology::build(&sphere);
        let meridian = find_edge(&sphere, &topology, |a, b| {
            (a.y.abs() < 1e-5) != (b.y.abs() < 1e-5) && a.x.abs() < 1e-5 && b.x.abs() < 1e-5 && a.z > 0.0
        });
        let edges = edge_loop(&sphere, &topology, meridian, WalkOptions::default(), &none(), &none());
        assert_eq!(edges.len(), 8, "pole to pole, then stop");
        let quads_only = WalkOptions {
            stop_at_non_quads: true,
            ..WalkOptions::default()
        };
        let edges = edge_loop(&sphere, &topology, meridian, quads_only, &none(), &none());
        assert_eq!(edges.len(), 6, "not into the triangle fans");
    }

    #[test]
    fn plane_loops_stop_at_the_border_and_border_loops_follow_it() {
        let grid = shapes::grid(4, 3);
        let topology = Topology::build(&grid);
        // An interior edge along X (z = 1).
        let row = find_edge(&grid, &topology, |a, b| a.z == 1.0 && b.z == 1.0 && a.x.min(b.x) == 1.0);
        let edges = edge_loop(&grid, &topology, row, WalkOptions::default(), &none(), &none());
        assert_eq!(edges.len(), 4, "the whole row, border to border");
        let border = find_edge(&grid, &topology, |a, b| a.z == 0.0 && b.z == 0.0);
        let edges = edge_loop(&grid, &topology, border, WalkOptions::default(), &none(), &none());
        assert_eq!(edges.len(), 14, "the whole border");
    }

    #[test]
    fn selections_fence_loops_off() {
        let grid = shapes::grid(6, 2);
        let topology = Topology::build(&grid);
        let row: Vec<usize> = (0..topology.edge_count())
            .filter(|&e| {
                let [a, b] = topology.edges()[e];
                grid.points()[us(a)].z == 1.0 && grid.points()[us(b)].z == 1.0
            })
            .collect();
        let start = row
            .iter()
            .copied()
            .find(|&e| edge_vector(&grid, &topology, e).x != 0.0)
            .unwrap();
        let fence: BitSet = row.iter().copied().filter(|&e| e != start).take(1).collect();
        let fenced = WalkOptions {
            stop_at_selections: true,
            ..WalkOptions::default()
        };
        let edges = edge_loop(&grid, &topology, start, fenced, &fence, &none());
        assert!(edges.len() < 6 && !edges.iter().any(|e| fence.contains(e)));
        assert_eq!(
            edge_loop(&grid, &topology, start, WalkOptions::default(), &fence, &none()).len(),
            6
        );
    }

    #[test]
    fn rings_cross_quads_and_optionally_triangle_fans() {
        let sphere = primitives::sphere(1.0, 16);
        let topology = Topology::build(&sphere);
        let meridian = find_edge(&sphere, &topology, |a, b| {
            (a.y.abs() < 1e-5) != (b.y.abs() < 1e-5) && a.x.abs() < 1e-5 && b.x.abs() < 1e-5 && a.z > 0.0
        });
        let ring = edge_ring(&sphere, &topology, meridian, WalkOptions::default(), &none(), &none());
        assert_eq!((ring.edges.len(), ring.faces.len()), (16, 16), "once around the band");

        let spoke = find_edge(&sphere, &topology, |a, b| {
            (a.y - 1.0).abs() < 1e-5 || (b.y - 1.0).abs() < 1e-5
        });
        let ring = edge_ring(&sphere, &topology, spoke, WalkOptions::default(), &none(), &none());
        assert_eq!(ring.edges.len(), 16, "around the pole's triangle fan");
        let poles = WalkOptions {
            stop_at_poles: true,
            ..WalkOptions::default()
        };
        assert_eq!(
            edge_ring(&sphere, &topology, spoke, poles, &none(), &none())
                .edges
                .len(),
            1
        );
    }

    #[test]
    fn plane_rings_stop_at_the_border() {
        let grid = shapes::grid(4, 3);
        let topology = Topology::build(&grid);
        // An edge along Z: its ring runs along X, crossing the 4 quads of a row.
        let along_z = find_edge(&grid, &topology, |a, b| a.x == 2.0 && b.x == 2.0 && a.z.min(b.z) == 1.0);
        let ring = edge_ring(&grid, &topology, along_z, WalkOptions::default(), &none(), &none());
        assert_eq!((ring.edges.len(), ring.faces.len()), (5, 4));
    }

    #[test]
    fn torus_loops_and_rings_close() {
        let torus = shapes::torus(12, 6);
        let topology = Topology::build(&torus);
        for e in [0, 1, 2] {
            let edges = edge_loop(&torus, &topology, e, WalkOptions::default(), &none(), &none());
            assert!(edges.len() == 12 || edges.len() == 6, "edge {e}: {}", edges.len());
            let ring = edge_ring(&torus, &topology, e, WalkOptions::default(), &none(), &none());
            assert_eq!(
                ring.edges.len(),
                ring.faces.len(),
                "a closed ring crosses one quad per edge"
            );
        }
    }
}
