//! Delete, Melt and Dissolve: remove components, merging the polygons around them.

use std::collections::HashMap;

use super::{Builder, Corner, Rebuilt, corner_faces, next_corner};
use crate::{BitSet, Component, MeshError, PolyMesh, Topology, u32_index, us};

/// Deletes `selected` of kind `component` (points, unique edge indices or faces):
/// the faces themselves; for edges, the faces using them; for points, the faces
/// using them and the points. Points no face uses any more go too.
pub fn delete(
    mesh: &PolyMesh,
    topology: &Topology,
    component: Component,
    selected: &BitSet,
) -> Result<Rebuilt, MeshError> {
    let doomed: BitSet = match component {
        Component::Point => selected
            .iter()
            .flat_map(|p| topology.point_faces(p))
            .map(|&f| us(f))
            .collect(),
        Component::Edge => selected
            .iter()
            .flat_map(|e| topology.edge_faces(e))
            .map(|&f| us(f))
            .collect(),
        Component::Polygon => selected.clone(),
    };
    let mut builder = Builder::new(mesh);
    for f in (0..mesh.face_count()).filter(|&f| !doomed.contains(f)) {
        builder.keep(f);
    }
    if component == Component::Point {
        builder.removed = selected.clone();
    }
    builder.finish()
}

/// Melts `selected` of kind `component` into the faces around them: each
/// connected group of selected faces becomes one face; the two faces of each
/// selected edge become one (its points stay); the faces around each selected
/// point become one, without the point. A region whose outline is not a single
/// loop (a hole, or touching itself at a point) is left alone, as are border edges.
pub fn melt(
    mesh: &PolyMesh,
    topology: &Topology,
    component: Component,
    selected: &BitSet,
) -> Result<Rebuilt, MeshError> {
    let corner_faces = corner_faces(mesh);
    let mut regions = Regions::new(mesh.face_count());
    match component {
        Component::Polygon => {
            for f in selected.iter().filter(|&f| f < mesh.face_count()) {
                regions.include(f);
                for c in mesh.corner_range(f) {
                    let Some(edge) = topology.corner_edge(c) else { continue };
                    for &g in topology.edge_faces(us(edge)) {
                        if selected.contains(us(g)) {
                            regions.join(f, us(g));
                        }
                    }
                }
            }
        }
        Component::Edge => {
            for e in selected.iter() {
                if let &[f, g] = topology.edge_faces(e) {
                    regions.include(us(f));
                    regions.join(us(f), us(g));
                }
            }
        }
        Component::Point => {
            for p in selected.iter() {
                let faces = topology.point_faces(p);
                for &f in faces {
                    regions.include(us(f));
                    regions.join(us(faces[0]), us(f));
                }
            }
        }
    }
    let drop_points = (component == Component::Point).then_some(selected);
    // A face on its own melts only to lose a point (Points mode).
    let min_faces = if component == Component::Point { 1 } else { 2 };

    let mut outlines: HashMap<usize, Vec<usize>> = HashMap::new();
    for (root, faces) in regions.groups() {
        if faces.len() >= min_faces
            && let Some(outline) = outline(mesh, topology, &corner_faces, &faces, &regions, drop_points)
        {
            outlines.insert(root, outline);
        }
    }
    let mut builder = Builder::new(mesh);
    let mut corners = Vec::new();
    for f in 0..mesh.face_count() {
        match regions.root(f).and_then(|r| outlines.get(&r).map(|o| (r, o))) {
            // The merged face stands where the region's first face was.
            Some((root, outline)) if regions.first(root) == f => {
                corners.clear();
                corners.extend(outline.iter().map(|&c| Corner::Old(c)));
                builder.push(&corners, f);
            }
            Some(_) => {}
            None => builder.keep(f),
        }
    }
    if let Some(points) = drop_points {
        // Only where the point's faces melted; a point of a failed region stays.
        builder.removed = points
            .iter()
            .filter(|&p| {
                let faces = topology.point_faces(p);
                !faces.is_empty()
                    && faces
                        .iter()
                        .all(|&f| regions.root(us(f)).is_some_and(|r| outlines.contains_key(&r)))
            })
            .collect();
    }
    builder.finish()
}

/// [`melt`], and for edges also removes the points the melted edges ended at that
/// are left between just two edges (they no longer shape anything).
pub fn dissolve(
    mesh: &PolyMesh,
    topology: &Topology,
    component: Component,
    selected: &BitSet,
) -> Result<Rebuilt, MeshError> {
    let melted = melt(mesh, topology, component, selected)?;
    if component != Component::Edge {
        return Ok(melted);
    }
    let mut new_index = vec![u32::MAX; mesh.point_count()];
    for (p, &origin) in melted.point_origin.iter().enumerate() {
        if let Some(slot) = new_index.get_mut(us(origin)) {
            *slot = u32_index(p);
        }
    }
    let result = &melted.mesh;
    let after = Topology::build(result);
    let mut doomed: BitSet = selected
        .iter()
        .filter_map(|e| topology.edges().get(e))
        .flat_map(|&[a, b]| [a, b])
        .filter_map(|p| new_index.get(us(p)).copied().filter(|&q| q != u32::MAX))
        .filter(|&q| after.point_edges(us(q)).len() == 2)
        .map(us)
        .collect();
    // Every face must keep three corners: spare the points of a face that would not.
    loop {
        let spare: Vec<usize> = (0..result.face_count())
            .filter(|&f| result.face(f).iter().filter(|&&p| !doomed.contains(us(p))).count() < 3)
            .flat_map(|f| result.face(f).iter().map(|&p| us(p)))
            .filter(|&p| doomed.contains(p))
            .collect();
        if spare.is_empty() {
            break;
        }
        for p in spare {
            doomed.remove(p);
        }
    }
    let mut builder = Builder::new(result);
    let mut corners = Vec::new();
    for f in 0..result.face_count() {
        corners.clear();
        corners.extend(
            result
                .corner_range(f)
                .filter(|&c| !doomed.contains(us(result.corner_points()[c])))
                .map(Corner::Old),
        );
        builder.push(&corners, f);
    }
    builder.removed = doomed;
    let cleaned = builder.finish()?;
    Ok(melted.then(cleaned))
}

/// The outline of the region made of `faces` as one loop of source corners in
/// winding order, without the corners at `drop_points`; `None` unless it is a
/// single simple loop of at least three corners.
fn outline(
    mesh: &PolyMesh,
    topology: &Topology,
    corner_faces: &[u32],
    faces: &[usize],
    regions: &Regions,
    drop_points: Option<&BitSet>,
) -> Option<Vec<usize>> {
    let region = regions.root(faces[0]);
    let inside = |c: u32| regions.root(us(corner_faces[us(c)])) == region;
    // Outline corners: their edge has no other corner in the region.
    let mut leaving: HashMap<u32, usize> = HashMap::new();
    let mut count = 0;
    for &f in faces {
        for c in mesh.corner_range(f) {
            let edge = topology.corner_edge(c)?;
            let twin = topology.edge_corners(us(edge)).iter().any(|&k| us(k) != c && inside(k));
            if !twin {
                // Two outline corners leaving one point: the region pinches there.
                if leaving.insert(mesh.corner_points()[c], c).is_some() {
                    return None;
                }
                count += 1;
            }
        }
    }
    let &start = leaving.values().min()?;
    let mut loop_corners = Vec::with_capacity(count);
    let mut c = start;
    loop {
        loop_corners.push(c);
        let next = mesh.corner_points()[next_corner(mesh, us(corner_faces[c]), c)];
        c = *leaving.get(&next)?;
        if c == start || loop_corners.len() > count {
            break;
        }
    }
    if loop_corners.len() != count {
        return None; // more than one loop: the region has a hole
    }
    if let Some(points) = drop_points {
        loop_corners.retain(|&c| !points.contains(us(mesh.corner_points()[c])));
    }
    (loop_corners.len() >= 3).then_some(loop_corners)
}

/// Faces grouped by union-find; only faces that were `include`d belong to one.
struct Regions {
    parent: Vec<usize>,
    member: BitSet,
}

impl Regions {
    fn new(faces: usize) -> Self {
        Self {
            parent: (0..faces).collect(),
            member: BitSet::new(),
        }
    }

    fn include(&mut self, face: usize) {
        if face < self.parent.len() {
            self.member.insert(face);
        }
    }

    fn find(&self, mut face: usize) -> usize {
        while self.parent[face] != face {
            face = self.parent[face];
        }
        face
    }

    fn join(&mut self, a: usize, b: usize) {
        if a >= self.parent.len() || b >= self.parent.len() {
            return;
        }
        self.include(a);
        self.include(b);
        let (ra, rb) = (self.find(a), self.find(b));
        // The lower index becomes the root, so a region's root is its first face.
        let (low, high) = (ra.min(rb), ra.max(rb));
        self.parent[high] = low;
    }

    /// The region of `face`, `None` if it is in none.
    fn root(&self, face: usize) -> Option<usize> {
        self.member.contains(face).then(|| self.find(face))
    }

    /// The region's first face (its root, see `join`).
    fn first(&self, root: usize) -> usize {
        root
    }

    /// Every region's root and faces, in face order.
    fn groups(&self) -> Vec<(usize, Vec<usize>)> {
        let mut groups: Vec<(usize, Vec<usize>)> = Vec::new();
        let mut slot: HashMap<usize, usize> = HashMap::new();
        for f in self.member.iter() {
            let root = self.find(f);
            let i = *slot.entry(root).or_insert_with(|| {
                groups.push((root, Vec::new()));
                groups.len() - 1
            });
            groups[i].1.push(f);
        }
        groups
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;
    use crate::{ComponentSet, shapes};

    fn set(items: &[usize]) -> BitSet {
        items.iter().copied().collect()
    }

    /// Manifold and consistently wound, with no point left unused.
    fn assert_clean(mesh: &PolyMesh) -> Topology {
        let topology = Topology::build(mesh);
        assert!(topology.is_manifold(), "manifold and consistently oriented");
        for p in 0..mesh.point_count() {
            assert!(!topology.point_faces(p).is_empty(), "point {p} is unused");
        }
        topology
    }

    fn edge(topology: &Topology, a: u32, b: u32) -> usize {
        topology
            .edges()
            .iter()
            .position(|&e| e == [a.min(b), a.max(b)])
            .unwrap()
    }

    #[test]
    fn deleting_a_cube_corner_removes_its_three_faces_and_the_point() {
        let cube = shapes::cube(2.0);
        let out = delete(&cube, &Topology::build(&cube), Component::Point, &set(&[0])).unwrap();
        assert_eq!((out.mesh.point_count(), out.mesh.face_count()), (7, 3));
        assert_clean(&out.mesh);
        assert!(!out.point_origin.contains(&0));
        assert!(out.mesh.points().iter().all(|&p| p != Vec3::splat(-1.0)));
    }

    #[test]
    fn deleting_edges_takes_their_faces_and_polygons_go_alone() {
        let cube = shapes::cube(2.0);
        let topology = Topology::build(&cube);
        let out = delete(&cube, &topology, Component::Edge, &set(&[edge(&topology, 0, 1)])).unwrap();
        assert_eq!(
            (out.mesh.point_count(), out.mesh.face_count()),
            (8, 4),
            "both faces of the edge"
        );
        let plane = shapes::grid(2, 1);
        let out = delete(&plane, &Topology::build(&plane), Component::Polygon, &set(&[0])).unwrap();
        assert_eq!((out.mesh.point_count(), out.mesh.face_count()), (4, 1));
        assert_eq!(out.face_origin, [1]);
        assert_clean(&out.mesh);
    }

    #[test]
    fn melting_a_patch_makes_one_face_without_its_inner_point() {
        let plane = shapes::grid(2, 2);
        let out = melt(
            &plane,
            &Topology::build(&plane),
            Component::Polygon,
            &set(&[0, 1, 2, 3]),
        )
        .unwrap();
        assert_eq!((out.mesh.point_count(), out.mesh.face_count()), (8, 1));
        assert_eq!(out.mesh.face(0).len(), 8);
        assert!(
            out.mesh.face_normal(0).abs_diff_eq(Vec3::Y, 1e-5),
            "the winding is kept"
        );
        assert_clean(&out.mesh);
        // The same through the middle point.
        let out = melt(&plane, &Topology::build(&plane), Component::Point, &set(&[4])).unwrap();
        assert_eq!((out.mesh.point_count(), out.mesh.face_count()), (8, 1));
    }

    #[test]
    fn regions_with_a_hole_do_not_melt() {
        let plane = shapes::grid(3, 3);
        let ring = set(&[0, 1, 2, 3, 5, 6, 7, 8]);
        let out = melt(&plane, &Topology::build(&plane), Component::Polygon, &ring).unwrap();
        assert_eq!(out.mesh, plane, "eight faces around a hole stay as they are");
    }

    #[test]
    fn melting_an_edge_merges_its_faces_and_keeps_its_points() {
        let cube = shapes::cube(2.0);
        let topology = Topology::build(&cube);
        let out = melt(&cube, &topology, Component::Edge, &set(&[edge(&topology, 0, 1)])).unwrap();
        assert_eq!((out.mesh.point_count(), out.mesh.face_count()), (8, 5));
        assert!(out.mesh.faces().any(|f| f.len() == 6));
        assert_clean(&out.mesh);
        let plane = shapes::grid(1, 1);
        let border = melt(&plane, &Topology::build(&plane), Component::Edge, &set(&[0])).unwrap();
        assert_eq!(border.mesh, plane, "a border edge has nothing to merge with");
    }

    #[test]
    fn dissolving_an_edge_also_removes_the_points_left_between_two_edges() {
        let plane = shapes::grid(3, 1);
        let topology = Topology::build(&plane);
        // Points 1 (1,0) and 5 (1,1): the edge between the first two faces.
        let melted = melt(&plane, &topology, Component::Edge, &set(&[edge(&topology, 1, 5)])).unwrap();
        assert_eq!((melted.mesh.point_count(), melted.mesh.face_count()), (8, 2));
        let out = dissolve(&plane, &topology, Component::Edge, &set(&[edge(&topology, 1, 5)])).unwrap();
        assert_eq!((out.mesh.point_count(), out.mesh.face_count()), (6, 2));
        assert_eq!(out.mesh.face(0).len(), 4, "a clean quad again");
        assert_clean(&out.mesh);
        assert!(!out.point_origin.contains(&1) && !out.point_origin.contains(&5));
    }

    #[test]
    fn a_border_point_melts_out_of_its_face() {
        let quad = shapes::grid(1, 1);
        let out = melt(&quad, &Topology::build(&quad), Component::Point, &set(&[0])).unwrap();
        assert_eq!((out.mesh.point_count(), out.mesh.face_count()), (3, 1));
    }

    #[test]
    fn selections_follow_their_components() {
        let plane = shapes::grid(3, 1);
        let topology = Topology::build(&plane);
        let out = delete(&plane, &topology, Component::Polygon, &set(&[0])).unwrap();
        let selection = ComponentSet {
            points: set(&[7]),
            edges: crate::selection::corners_from_edges(&set(&[edge(&topology, 2, 3)]), &topology),
            polygons: set(&[2]),
        };
        let carried = out.carry(&selection, &topology);
        assert_eq!(carried.polygons.iter().collect::<Vec<_>>(), [1], "face 2 is now face 1");
        let point = carried.points.iter().next().unwrap();
        assert_eq!(out.point_origin[point], 7);
        let corner = carried.edges.iter().next().unwrap();
        let ends = [
            out.mesh.corner_points()[corner],
            out.mesh.corner_points()[next_corner(&out.mesh, 1, corner)],
        ];
        let mut origins = ends.map(|p| out.point_origin[us(p)]);
        origins.sort_unstable();
        assert_eq!(origins, [2, 3]);
    }
}
