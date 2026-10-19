//! Component selections: which points, edges and polygons of a mesh are flagged
//! (selected, or hidden), converting a selection between component kinds, and
//! the Select menu's operations (grow, shrink, connected, invert).

use crate::{BitSet, PolyMesh, Topology, us};

/// The three kinds of mesh component, one per component mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Component {
    Point,
    Edge,
    Polygon,
}

impl Component {
    pub const ALL: [Self; 3] = [Self::Point, Self::Edge, Self::Polygon];

    /// Plural name, as in "Select Points".
    #[must_use]
    pub const fn plural(self) -> &'static str {
        match self {
            Self::Point => "Points",
            Self::Edge => "Edges",
            Self::Polygon => "Polygons",
        }
    }
}

/// One flag per point, edge and polygon of a mesh.
///
/// Edges are flagged **by corner**: corner `c` stands for the edge from `c` to
/// the next corner of its face, so an edge shared by two faces counts as flagged
/// when either of its corners is ([`edges_from_corners`]). Corners, unlike unique
/// edge indices, do not depend on rebuilding the topology.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComponentSet {
    pub points: BitSet,
    /// By corner (see the type's docs).
    pub edges: BitSet,
    pub polygons: BitSet,
}

impl ComponentSet {
    #[must_use]
    pub fn get(&self, component: Component) -> &BitSet {
        match component {
            Component::Point => &self.points,
            Component::Edge => &self.edges,
            Component::Polygon => &self.polygons,
        }
    }

    pub fn get_mut(&mut self, component: Component) -> &mut BitSet {
        match component {
            Component::Point => &mut self.points,
            Component::Edge => &mut self.edges,
            Component::Polygon => &mut self.polygons,
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.points.is_empty() && self.edges.is_empty() && self.polygons.is_empty()
    }
}

/// The unique edges (indices into [`Topology::edges`]) flagged in `corners`.
#[must_use]
pub fn edges_from_corners(corners: &BitSet, topology: &Topology) -> BitSet {
    corners.iter().filter_map(|c| topology.corner_edge(c)).map(us).collect()
}

/// Flags every corner of each unique edge in `edges`.
#[must_use]
pub fn corners_from_edges(edges: &BitSet, topology: &Topology) -> BitSet {
    edges
        .iter()
        .flat_map(|e| topology.edge_corners(e))
        .map(|&c| us(c))
        .collect()
}

/// Converts the `from` selection in `set` to a selection of `to` components,
/// like switching component mode with Shift (`tolerant`) or Ctrl held:
///
/// * tolerant — a component is selected if it **touches** any selected one (a
///   point selects every edge and polygon that uses it);
/// * strict — only if **all** of it is covered (a polygon only when all of its
///   points, or edges, are selected; a point only when all of its edges, or
///   polygons, are).
///
/// Edges are returned by corner, ready to store in a [`ComponentSet`].
#[must_use]
pub fn convert(
    mesh: &PolyMesh,
    topology: &Topology,
    set: &ComponentSet,
    from: Component,
    to: Component,
    tolerant: bool,
) -> BitSet {
    // `selected` of `total` parts covered: does the whole count as selected?
    let covers = |selected: usize, total: usize| {
        if tolerant {
            selected > 0
        } else {
            total > 0 && selected == total
        }
    };
    let count = |members: &mut dyn Iterator<Item = bool>| {
        members.fold((0, 0), |(selected, total), on| (selected + usize::from(on), total + 1))
    };
    match (from, to) {
        (Component::Point, Component::Point)
        | (Component::Edge, Component::Edge)
        | (Component::Polygon, Component::Polygon) => set.get(from).clone(),
        (Component::Point, Component::Edge) => {
            let edges = topology
                .edges()
                .iter()
                .enumerate()
                .filter(|(_, ends)| {
                    let (s, t) = count(&mut ends.iter().map(|&p| set.points.contains(us(p))));
                    covers(s, t)
                })
                .map(|(e, _)| e)
                .collect();
            corners_from_edges(&edges, topology)
        }
        (Component::Point, Component::Polygon) => (0..mesh.face_count())
            .filter(|&f| {
                let (s, t) = count(&mut mesh.face(f).iter().map(|&p| set.points.contains(us(p))));
                covers(s, t)
            })
            .collect(),
        (Component::Edge, Component::Point) => {
            let edges = edges_from_corners(&set.edges, topology);
            let mut selected = vec![0usize; mesh.point_count()];
            let mut total = vec![0usize; mesh.point_count()];
            for (e, ends) in topology.edges().iter().enumerate() {
                for &p in ends {
                    total[us(p)] += 1;
                    selected[us(p)] += usize::from(edges.contains(e));
                }
            }
            (0..mesh.point_count())
                .filter(|&p| covers(selected[p], total[p]))
                .collect()
        }
        (Component::Edge, Component::Polygon) => {
            let edges = edges_from_corners(&set.edges, topology);
            (0..mesh.face_count())
                .filter(|&f| {
                    let mut on = mesh
                        .corner_range(f)
                        .map(|c| topology.corner_edge(c).is_some_and(|e| edges.contains(us(e))));
                    let (s, t) = count(&mut on);
                    covers(s, t)
                })
                .collect()
        }
        (Component::Polygon, Component::Point) => (0..mesh.point_count())
            .filter(|&p| {
                let mut on = topology.point_faces(p).iter().map(|&f| set.polygons.contains(us(f)));
                let (s, t) = count(&mut on);
                covers(s, t)
            })
            .collect(),
        (Component::Polygon, Component::Edge) => {
            let edges = (0..topology.edge_count())
                .filter(|&e| {
                    let mut on = topology.edge_faces(e).iter().map(|&f| set.polygons.contains(us(f)));
                    let (s, t) = count(&mut on);
                    covers(s, t)
                })
                .collect();
            corners_from_edges(&edges, topology)
        }
    }
}

/// `hidden` plus the points and edges whose polygons are all hidden (hiding
/// polygons hides their points and edges too). Edges by corner.
#[must_use]
pub fn hidden_closure(mesh: &PolyMesh, topology: &Topology, hidden: &ComponentSet) -> ComponentSet {
    let mut closure = hidden.clone();
    if hidden.polygons.is_empty() {
        return closure;
    }
    let all_hidden = |faces: &[u32]| !faces.is_empty() && faces.iter().all(|&f| hidden.polygons.contains(us(f)));
    closure
        .points
        .extend((0..mesh.point_count()).filter(|&p| all_hidden(topology.point_faces(p))));
    let edges: BitSet = (0..topology.edge_count())
        .filter(|&e| all_hidden(topology.edge_faces(e)))
        .collect();
    closure.edges.extend(corners_from_edges(&edges, topology).iter());
    closure
}

/// How many components of a kind there are (unique edges for edges).
fn count(mesh: &PolyMesh, topology: &Topology, component: Component) -> usize {
    match component {
        Component::Point => mesh.point_count(),
        Component::Edge => topology.edge_count(),
        Component::Polygon => mesh.face_count(),
    }
}

/// A stored selection in element space: edges by unique edge, the rest as is.
fn elements(set: &BitSet, component: Component, topology: &Topology) -> BitSet {
    match component {
        Component::Edge => edges_from_corners(set, topology),
        Component::Point | Component::Polygon => set.clone(),
    }
}

/// Back from element space to storage (edges by corner).
fn stored(set: BitSet, component: Component, topology: &Topology) -> BitSet {
    match component {
        Component::Edge => corners_from_edges(&set, topology),
        Component::Point | Component::Polygon => set,
    }
}

/// Appends the neighbours of `element`: points through edges, edges through
/// their points, polygons through shared edges (`by_edge`) or shared points.
fn neighbours(
    mesh: &PolyMesh,
    topology: &Topology,
    component: Component,
    element: usize,
    by_edge: bool,
    out: &mut Vec<usize>,
) {
    match component {
        Component::Point => {
            for &e in topology.point_edges(element) {
                let [a, b] = topology.edges()[us(e)];
                out.push(us(if us(a) == element { b } else { a }));
            }
        }
        Component::Edge => {
            for &p in topology.edges().get(element).into_iter().flatten() {
                out.extend(
                    topology
                        .point_edges(us(p))
                        .iter()
                        .map(|&e| us(e))
                        .filter(|&e| e != element),
                );
            }
        }
        Component::Polygon if by_edge => {
            for c in mesh.corner_range(element) {
                if let Some(e) = topology.corner_edge(c) {
                    out.extend(
                        topology
                            .edge_faces(us(e))
                            .iter()
                            .map(|&f| us(f))
                            .filter(|&f| f != element),
                    );
                }
            }
        }
        Component::Polygon => {
            for &p in mesh.face(element) {
                out.extend(
                    topology
                        .point_faces(us(p))
                        .iter()
                        .map(|&f| us(f))
                        .filter(|&f| f != element),
                );
            }
        }
    }
}

/// Grow Selection: `set` plus every neighbour of its members (for polygons,
/// through shared edges when `edges_only`, else through shared points).
#[must_use]
pub fn grow(mesh: &PolyMesh, topology: &Topology, set: &BitSet, component: Component, edges_only: bool) -> BitSet {
    let members = elements(set, component, topology);
    let mut grown = members.clone();
    let mut around = Vec::new();
    for element in members.iter() {
        around.clear();
        neighbours(mesh, topology, component, element, edges_only, &mut around);
        grown.extend(around.iter().copied());
    }
    stored(grown, component, topology)
}

/// Shrink Selection: drops the members on the selection's border — those with
/// a neighbour outside it (same neighbourhood as [`grow`]). A selection with no
/// border, such as every polygon of a closed mesh, stays as it is.
#[must_use]
pub fn shrink(mesh: &PolyMesh, topology: &Topology, set: &BitSet, component: Component, edges_only: bool) -> BitSet {
    let members = elements(set, component, topology);
    let mut around = Vec::new();
    let kept = members
        .iter()
        .filter(|&element| {
            around.clear();
            neighbours(mesh, topology, component, element, edges_only, &mut around);
            around.iter().all(|&n| members.contains(n))
        })
        .collect();
    stored(kept, component, topology)
}

/// Select Connected: every component reachable from the selection through
/// shared geometry (polygons through shared points), as one flood fill.
#[must_use]
pub fn connected(mesh: &PolyMesh, topology: &Topology, set: &BitSet, component: Component) -> BitSet {
    let mut reached = elements(set, component, topology);
    let mut pending: Vec<usize> = reached.iter().collect();
    let mut around = Vec::new();
    while let Some(element) = pending.pop() {
        around.clear();
        neighbours(mesh, topology, component, element, false, &mut around);
        for &n in &around {
            if reached.insert(n) {
                pending.push(n);
            }
        }
    }
    stored(reached, component, topology)
}

/// Invert: every component not in `set` and not in `hidden` (both stored
/// sets of `component`; edges are inverted as edges, not corner by corner).
#[must_use]
pub fn invert(mesh: &PolyMesh, topology: &Topology, set: &BitSet, component: Component, hidden: &BitSet) -> BitSet {
    let (members, hidden) = (
        elements(set, component, topology),
        elements(hidden, component, topology),
    );
    let inverted = (0..count(mesh, topology, component))
        .filter(|&e| !members.contains(e) && !hidden.contains(e))
        .collect();
    stored(inverted, component, topology)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes;

    fn cube() -> (PolyMesh, Topology) {
        let mesh = shapes::cube(2.0);
        let topology = Topology::build(&mesh);
        (mesh, topology)
    }

    fn sorted(set: &BitSet) -> Vec<usize> {
        set.iter().collect()
    }

    #[test]
    fn one_point_converts_to_its_faces_only_when_tolerant() {
        let (mesh, topology) = cube();
        let set = ComponentSet {
            points: [0].into_iter().collect(),
            ..ComponentSet::default()
        };
        let tolerant = convert(&mesh, &topology, &set, Component::Point, Component::Polygon, true);
        let mut expected: Vec<usize> = topology.point_faces(0).iter().map(|&f| us(f)).collect();
        expected.sort_unstable();
        assert_eq!(sorted(&tolerant), expected, "every face using the point");
        assert_eq!(tolerant.len(), 3);
        let strict = convert(&mesh, &topology, &set, Component::Point, Component::Polygon, false);
        assert!(strict.is_empty(), "no face has all of its points selected");
    }

    #[test]
    fn a_face_converts_to_points_and_edges() {
        let (mesh, topology) = cube();
        let set = ComponentSet {
            polygons: [0].into_iter().collect(),
            ..ComponentSet::default()
        };
        let points = convert(&mesh, &topology, &set, Component::Polygon, Component::Point, true);
        let mut corners: Vec<usize> = mesh.face(0).iter().map(|&p| us(p)).collect();
        corners.sort_unstable();
        assert_eq!(sorted(&points), corners);
        assert!(
            convert(&mesh, &topology, &set, Component::Polygon, Component::Point, false).is_empty(),
            "each point also belongs to unselected faces"
        );
        let edges = convert(&mesh, &topology, &set, Component::Polygon, Component::Edge, true);
        assert_eq!(edges_from_corners(&edges, &topology).len(), 4);
        assert_eq!(edges.len(), 8, "both corners of each of the 4 edges");
        assert!(convert(&mesh, &topology, &set, Component::Polygon, Component::Edge, false).is_empty());

        let all = ComponentSet {
            polygons: (0..6).collect(),
            ..ComponentSet::default()
        };
        let every_point = convert(&mesh, &topology, &all, Component::Polygon, Component::Point, false);
        assert_eq!(every_point.len(), 8, "strict: all faces of every point are selected");
    }

    #[test]
    fn a_face_outline_converts_between_points_edges_and_faces() {
        let (mesh, topology) = cube();
        let outline = ComponentSet {
            points: mesh.face(0).iter().map(|&p| us(p)).collect(),
            ..ComponentSet::default()
        };
        let strict_edges = convert(&mesh, &topology, &outline, Component::Point, Component::Edge, false);
        assert_eq!(
            edges_from_corners(&strict_edges, &topology).len(),
            4,
            "the face's border"
        );
        let tolerant_edges = convert(&mesh, &topology, &outline, Component::Point, Component::Edge, true);
        assert_eq!(
            edges_from_corners(&tolerant_edges, &topology).len(),
            8,
            "border plus the 4 side edges"
        );

        let edges = ComponentSet {
            edges: strict_edges,
            ..ComponentSet::default()
        };
        let strict_faces = convert(&mesh, &topology, &edges, Component::Edge, Component::Polygon, false);
        assert_eq!(sorted(&strict_faces), [0]);
        let tolerant_faces = convert(&mesh, &topology, &edges, Component::Edge, Component::Polygon, true);
        assert_eq!(tolerant_faces.len(), 5, "the face and its four neighbours");
        let points = convert(&mesh, &topology, &edges, Component::Edge, Component::Point, true);
        assert_eq!(points.len(), 4);
        assert!(
            convert(&mesh, &topology, &edges, Component::Edge, Component::Point, false).is_empty(),
            "every point has a third, unselected edge"
        );
    }

    /// A 2 × 2 patch of faces in the middle of a 6 × 6 grid.
    fn patch(grid: &PolyMesh) -> BitSet {
        (0..grid.face_count())
            .filter(|&f| {
                let c = grid.face_center(f);
                (2.0..4.0).contains(&c.x) && (2.0..4.0).contains(&c.z)
            })
            .collect()
    }

    #[test]
    fn grow_then_shrink_restores_an_interior_patch() {
        let grid = shapes::grid(6, 6);
        let topology = Topology::build(&grid);
        let start = patch(&grid);
        assert_eq!(start.len(), 4);
        for edges_only in [true, false] {
            let grown = grow(&grid, &topology, &start, Component::Polygon, edges_only);
            assert_eq!(
                grown.len(),
                if edges_only { 4 + 8 } else { 16 },
                "edges_only = {edges_only}"
            );
            assert_eq!(shrink(&grid, &topology, &grown, Component::Polygon, edges_only), start);
        }
    }

    #[test]
    fn a_selection_without_border_does_not_shrink() {
        let (mesh, topology) = cube();
        let all: BitSet = (0..6).collect();
        assert_eq!(shrink(&mesh, &topology, &all, Component::Polygon, true), all);
        let every_point: BitSet = (0..8).collect();
        assert_eq!(
            shrink(&mesh, &topology, &every_point, Component::Point, true),
            every_point
        );
    }

    #[test]
    fn points_and_edges_grow_along_edges() {
        let grid = shapes::grid(4, 4);
        let topology = Topology::build(&grid);
        let centre = grid.points().iter().position(|p| p.x == 2.0 && p.z == 2.0).unwrap();
        let one: BitSet = [centre].into_iter().collect();
        assert_eq!(
            grow(&grid, &topology, &one, Component::Point, true).len(),
            5,
            "itself and 4 neighbours"
        );
        let edge = us(topology.point_edges(centre)[0]);
        let corners = corners_from_edges(&[edge].into_iter().collect(), &topology);
        let grown = grow(&grid, &topology, &corners, Component::Edge, true);
        assert_eq!(
            edges_from_corners(&grown, &topology).len(),
            7,
            "itself and 3 more at each end"
        );
    }

    #[test]
    fn connected_floods_one_island_only() {
        // Two cubes in one mesh, sharing nothing.
        let a = shapes::cube(1.0);
        let mut points = a.points().to_vec();
        points.extend(a.points().iter().map(|p| *p + glam::Vec3::X * 5.0));
        let faces: Vec<Vec<u32>> = a
            .faces()
            .map(<[u32]>::to_vec)
            .chain(a.faces().map(|f| f.iter().map(|&p| p + 8).collect()))
            .collect();
        let two = PolyMesh::from_faces(points, faces).unwrap();
        let topology = Topology::build(&two);
        let seed: BitSet = [7].into_iter().collect(); // a face of the second cube
        assert_eq!(
            sorted(&connected(&two, &topology, &seed, Component::Polygon)),
            [6, 7, 8, 9, 10, 11]
        );
        let point: BitSet = [0].into_iter().collect();
        assert_eq!(connected(&two, &topology, &point, Component::Point).len(), 8);
    }

    #[test]
    fn invert_skips_hidden_components_and_treats_edges_as_edges() {
        let (mesh, topology) = cube();
        let selected: BitSet = [0, 1].into_iter().collect();
        let hidden: BitSet = [5].into_iter().collect();
        assert_eq!(
            sorted(&invert(&mesh, &topology, &selected, Component::Polygon, &hidden)),
            [2, 3, 4]
        );
        // One corner of an edge selects the edge: inverting must drop the whole edge.
        let corner = us(topology.edge_corners(0)[0]);
        let one_corner: BitSet = [corner].into_iter().collect();
        let inverted = invert(&mesh, &topology, &one_corner, Component::Edge, &BitSet::new());
        let edges = edges_from_corners(&inverted, &topology);
        assert_eq!(edges.len(), 11);
        assert!(!edges.contains(0));
    }

    #[test]
    fn hiding_polygons_hides_their_points_and_edges() {
        let (mesh, topology) = cube();
        let hidden = ComponentSet {
            polygons: [0, 2].into_iter().collect(), // +X and +Y share an edge
            ..ComponentSet::default()
        };
        let closure = hidden_closure(&mesh, &topology, &hidden);
        assert!(closure.points.is_empty(), "every point still has a visible face");
        assert_eq!(
            edges_from_corners(&closure.edges, &topology).len(),
            1,
            "the shared edge"
        );
        let all = ComponentSet {
            polygons: (0..6).collect(),
            ..ComponentSet::default()
        };
        let closure = hidden_closure(&mesh, &topology, &all);
        assert_eq!(
            (
                closure.points.len(),
                edges_from_corners(&closure.edges, &topology).len()
            ),
            (8, 12)
        );
    }

    #[test]
    fn edges_round_trip_through_corners() {
        let (_, topology) = cube();
        let edges: BitSet = [0, 5, 11].into_iter().collect();
        let corners = corners_from_edges(&edges, &topology);
        assert_eq!(corners.len(), 6, "two corners per edge of a closed mesh");
        assert_eq!(edges_from_corners(&corners, &topology), edges);
        let one_corner: BitSet = [corners.iter().next().unwrap()].into_iter().collect();
        assert_eq!(
            edges_from_corners(&one_corner, &topology).len(),
            1,
            "either corner selects the edge"
        );
    }
}
