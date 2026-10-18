//! Component selections: which points, edges and polygons of a mesh are flagged
//! (selected, or hidden), and converting a selection between component kinds.

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
