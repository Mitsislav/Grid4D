//! Connect, Loop Cut and Line Cut: new edges across faces.

use std::collections::HashMap;

use glam::Vec3;

use super::{Builder, Corner, Rebuilt, corner_faces, next_corner};
use crate::{BitSet, Component, MeshError, PolyMesh, Topology, loops::Ring, u32_index, us};

/// Connects `selected` points (or the middles of `selected` unique edges) that
/// share a face: in every face the chosen corners are joined in their order around
/// it; two split the face in two, three or more cut out their own polygon plus the
/// pieces around it. Faces with one chosen corner only gain it (edges mode).
pub fn connect(
    mesh: &PolyMesh,
    topology: &Topology,
    component: Component,
    selected: &BitSet,
) -> Result<Rebuilt, MeshError> {
    let mut builder = Builder::new(mesh);
    let mut middles: HashMap<usize, u32> = HashMap::new();
    if component == Component::Edge {
        for e in selected.iter() {
            if let Some(&[a, b]) = topology.edges().get(e) {
                let middle = (builder.position(a) + builder.position(b)) * 0.5;
                middles.insert(e, builder.add_point(middle));
            }
        }
    }
    let (mut corners, mut chosen) = (Vec::new(), Vec::new());
    for f in 0..mesh.face_count() {
        corners.clear();
        chosen.clear();
        for c in mesh.corner_range(f) {
            corners.push(Corner::Old(c));
            match component {
                Component::Point if selected.contains(us(mesh.corner_points()[c])) => chosen.push(corners.len() - 1),
                Component::Edge => {
                    if let Some(&middle) = topology.corner_edge(c).and_then(|e| middles.get(&us(e))) {
                        corners.push(builder.on_edge(f, c, middle, 0.5));
                        chosen.push(corners.len() - 1);
                    }
                }
                Component::Point | Component::Polygon => {}
            }
        }
        split(&mut builder, &corners, &chosen, f);
    }
    builder.finish()
}

/// Cuts across `ring` (an edge ring and its strip of quads, see
/// [`loops::edge_ring`](crate::loops::edge_ring)) at `fractions` (each in
/// `(0, 1)`) along every ring edge, measured from the same side of the strip:
/// on `start` from its first point. Each quad of the strip splits into
/// `fractions.len() + 1` quads; other faces on a ring edge gain the new points.
pub fn loop_cut(
    mesh: &PolyMesh,
    topology: &Topology,
    ring: &Ring,
    start: usize,
    fractions: &[f32],
) -> Result<Rebuilt, MeshError> {
    let corner_faces = corner_faces(mesh);
    let mut fractions: Vec<f32> = fractions.iter().copied().filter(|t| *t > 0.0 && *t < 1.0).collect();
    fractions.sort_by(f32::total_cmp);
    let mut builder = Builder::new(mesh);
    if fractions.is_empty() || !ring.edges.contains(start) {
        return keep_all(builder, mesh);
    }
    // The reference end of every ring edge, carried across each quad: corner c
    // runs p0 → p1 and the opposite corner p2 → p3, with p0 facing p3.
    let quad_opposite = |c: usize| {
        let f = us(corner_faces[c]);
        (ring.faces.contains(f) && mesh.face(f).len() == 4).then(|| next_corner(mesh, f, next_corner(mesh, f, c)))
    };
    let mut side: HashMap<usize, u32> = HashMap::new();
    side.insert(start, topology.edges()[start][0]);
    let mut queue = vec![start];
    while let Some(e) = queue.pop() {
        for &c in topology.edge_corners(e) {
            let c = us(c);
            let Some(o) = quad_opposite(c) else { continue };
            let Some(opposite) = topology.corner_edge(o).map(us) else {
                continue;
            };
            if !ring.edges.contains(opposite) || side.contains_key(&opposite) {
                continue;
            }
            let f = us(corner_faces[c]);
            let p0 = mesh.corner_points()[c];
            let (p2, p3) = (mesh.corner_points()[o], mesh.corner_points()[next_corner(mesh, f, o)]);
            side.insert(opposite, if side[&e] == p0 { p3 } else { p2 });
            queue.push(opposite);
        }
    }
    // The new points of every ring edge, from its reference end.
    let mut cuts: HashMap<usize, (u32, Vec<u32>)> = HashMap::new();
    for e in ring.edges.iter() {
        let [a, b] = topology.edges()[e];
        let from = side.get(&e).copied().unwrap_or(a);
        let to = if from == a { b } else { a };
        let (pa, pb) = (builder.position(from), builder.position(to));
        let points = fractions.iter().map(|&t| builder.add_point(pa.lerp(pb, t))).collect();
        cuts.insert(e, (from, points));
    }
    // A corner's new points in its own direction, with their fractions from its point.
    let along = |c: usize| -> Option<Vec<(u32, f32)>> {
        let (from, points) = cuts.get(&us(topology.corner_edge(c)?))?;
        let forward = *from == mesh.corner_points()[c];
        let mut list: Vec<(u32, f32)> = points.iter().copied().zip(fractions.iter().copied()).collect();
        if !forward {
            list.reverse();
            for item in &mut list {
                item.1 = 1.0 - item.1;
            }
        }
        Some(list)
    };

    let mut corners = Vec::new();
    for f in 0..mesh.face_count() {
        let range = mesh.corner_range(f);
        // A strip quad: its two ring edges face each other.
        let split = ring.faces.contains(f) && range.len() == 4;
        let pair = split
            .then(|| {
                range
                    .clone()
                    .find(|&c| cuts.contains_key(&topology.corner_edge(c).map_or(usize::MAX, us)))
            })
            .flatten()
            .and_then(|c| {
                let o = next_corner(mesh, f, next_corner(mesh, f, c));
                Some((c, o, along(c)?, along(o)?))
            });
        if let Some((c0, c2, a, b)) = pair {
            let (c1, c3) = (next_corner(mesh, f, c0), next_corner(mesh, f, c2));
            // Points from q0 towards q1, and from q3 towards q2 (facing them).
            let left: Vec<Corner> = std::iter::once(Corner::Old(c0))
                .chain(a.iter().map(|&(p, t)| builder.on_edge(f, c0, p, t)))
                .chain(std::iter::once(Corner::Old(c1)))
                .collect();
            let right: Vec<Corner> = std::iter::once(Corner::Old(c3))
                .chain(b.iter().rev().map(|&(p, t)| builder.on_edge(f, c2, p, t)))
                .chain(std::iter::once(Corner::Old(c2)))
                .collect();
            for k in 0..left.len() - 1 {
                builder.push(&[left[k], left[k + 1], right[k + 1], right[k]], f);
            }
            for k in 1..left.len() - 1 {
                let (p, q) = (builder.point(left[k]), builder.point(right[k]));
                builder.new_edge(p, q);
            }
            continue;
        }
        corners.clear();
        for c in range {
            corners.push(Corner::Old(c));
            for (p, t) in along(c).unwrap_or_default() {
                corners.push(builder.on_edge(f, c, p, t));
            }
        }
        builder.push(&corners, f);
    }
    builder.finish()
}

/// A Line Cut: the plane through `eye` and the rays `from` and `to` (directions
/// through the line's ends), all in the mesh's space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineCut {
    pub eye: Vec3,
    pub from: Vec3,
    pub to: Vec3,
    /// Beyond the line's ends too (still only in front of the eye).
    pub infinite: bool,
    /// Split the faces it crosses; otherwise only add the points on their edges.
    pub connect: bool,
}

/// Cuts the mesh along `cut`, where `visible` accepts the crossing (a position in
/// the mesh's space). Every crossed edge gets a point (a crossing at a point uses
/// it); faces crossed twice are split between the crossings, more crossings are
/// paired along the line.
pub fn line_cut(
    mesh: &PolyMesh,
    topology: &Topology,
    cut: &LineCut,
    visible: impl Fn(Vec3) -> bool,
) -> Result<Rebuilt, MeshError> {
    let mut builder = Builder::new(mesh);
    let span = cut.from.cross(cut.to);
    let normal = span.normalize_or_zero();
    if normal == Vec3::ZERO {
        return keep_all(builder, mesh);
    }
    let scale = mesh.bounds().map_or(1.0, |(min, max)| (max - min).length().max(1e-6));
    let eps = scale * 1e-6;
    let denominator = span.dot(normal);
    // Where along the line a position lies (0 at `from`, 1 at `to`), if the cut reaches it.
    let along = |x: Vec3| -> Option<f32> {
        let w = x - cut.eye;
        let alpha = w.cross(cut.to).dot(normal) / denominator;
        let beta = cut.from.cross(w).dot(normal) / denominator;
        let reached = if cut.infinite {
            alpha + beta > 0.0
        } else {
            alpha >= -1e-6 && beta >= -1e-6 && alpha + beta > 0.0
        };
        (reached && visible(x)).then(|| beta / (alpha + beta))
    };
    let distance = |p: u32| (mesh.points()[us(p)] - cut.eye).dot(normal);

    // Points the plane passes through, and new points on the edges it crosses.
    let mut on_point: HashMap<u32, f32> = HashMap::new();
    for p in 0..mesh.point_count() {
        let p = u32_index(p);
        if distance(p).abs() <= eps
            && let Some(s) = along(mesh.points()[us(p)])
        {
            on_point.insert(p, s);
        }
    }
    let mut on_edge: HashMap<usize, (u32, f32, f32)> = HashMap::new();
    for (e, &[a, b]) in topology.edges().iter().enumerate() {
        let (da, db) = (distance(a), distance(b));
        if da.abs() <= eps || db.abs() <= eps || (da > 0.0) == (db > 0.0) {
            continue;
        }
        let t = da / (da - db);
        let x = mesh.points()[us(a)].lerp(mesh.points()[us(b)], t);
        if let Some(s) = along(x) {
            on_edge.insert(e, (builder.add_point(x), t, s));
        }
    }

    let (mut corners, mut crossings) = (Vec::new(), Vec::new());
    for f in 0..mesh.face_count() {
        corners.clear();
        crossings.clear();
        for c in mesh.corner_range(f) {
            let p = mesh.corner_points()[c];
            corners.push(Corner::Old(c));
            if let Some(&s) = on_point.get(&p) {
                crossings.push((corners.len() - 1, s));
            }
            let Some(e) = topology.corner_edge(c).map(us) else {
                continue;
            };
            let Some(&(q, t, s)) = on_edge.get(&e) else { continue };
            let forward = topology.edges()[e][0] == p;
            corners.push(builder.on_edge(f, c, q, if forward { t } else { 1.0 - t }));
            crossings.push((corners.len() - 1, s));
        }
        if !cut.connect || crossings.len() < 2 {
            builder.push(&corners, f);
            continue;
        }
        crossings.sort_by(|a, b| a.1.total_cmp(&b.1));
        // Split successively: each pair's chord lies within one piece.
        let mut pieces: Vec<Vec<usize>> = vec![(0..corners.len()).collect()];
        for &[(x, _), (y, _)] in crossings.as_chunks::<2>().0 {
            let Some(i) = pieces.iter().position(|piece| piece.contains(&x) && piece.contains(&y)) else {
                continue;
            };
            let piece = pieces.swap_remove(i);
            let positions: Vec<usize> = [x, y]
                .iter()
                .filter_map(|v| piece.iter().position(|w| w == v))
                .collect();
            let [ix, iy] = positions[..] else { continue };
            let (first, second) = arcs(&piece, ix, iy);
            if first.len() < 3 || second.len() < 3 {
                pieces.push(piece); // the chord is an existing side
                continue;
            }
            let (p, q) = (builder.point(corners[x]), builder.point(corners[y]));
            builder.new_edge(p, q);
            pieces.push(first);
            pieces.push(second);
        }
        for piece in pieces {
            let piece: Vec<Corner> = piece.iter().map(|&i| corners[i]).collect();
            builder.push(&piece, f);
        }
    }
    builder.finish()
}

/// The two arcs of the cycle `items` between positions `i` and `j`, both inclusive.
fn arcs(items: &[usize], i: usize, j: usize) -> (Vec<usize>, Vec<usize>) {
    let n = items.len();
    let walk = |from: usize, to: usize| {
        let len = (to + n - from) % n + 1;
        (0..len).map(|k| items[(from + k) % n]).collect::<Vec<usize>>()
    };
    (walk(i, j), walk(j, i))
}

/// Pushes face `origin` as `corners` cut along the chords between consecutive
/// `chosen` positions (around the face): the arcs between them, plus the polygon of
/// the chosen corners when there are three or more.
fn split(builder: &mut Builder<'_>, corners: &[Corner], chosen: &[usize], origin: usize) {
    let n = corners.len();
    let chord = |i: usize| {
        let gap = (chosen[(i + 1) % chosen.len()] + n - chosen[i]) % n;
        gap != 1 && gap != n - 1
    };
    if chosen.len() < 2 || !(0..chosen.len()).any(chord) {
        builder.push(corners, origin); // nothing to join that is not a side already
        return;
    }
    let mut arc = Vec::new();
    for (i, &from) in chosen.iter().enumerate() {
        let to = chosen[(i + 1) % chosen.len()];
        let gap = (to + n - from) % n;
        if gap + 1 >= 3 {
            arc.clear();
            arc.extend((0..=gap).map(|k| corners[(from + k) % n]));
            builder.push(&arc, origin);
        }
        // Two chosen corners share one chord; record it once.
        let chord_seen = chosen.len() == 2 && i == 1;
        if gap != 1 && gap != n - 1 && !chord_seen {
            let (p, q) = (builder.point(corners[from]), builder.point(corners[to]));
            builder.new_edge(p, q);
        }
    }
    if chosen.len() >= 3 {
        let inner: Vec<Corner> = chosen.iter().map(|&i| corners[i]).collect();
        builder.push(&inner, origin);
    }
}

fn keep_all(mut builder: Builder<'_>, mesh: &PolyMesh) -> Result<Rebuilt, MeshError> {
    for f in 0..mesh.face_count() {
        builder.keep(f);
    }
    builder.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        loops::{WalkOptions, edge_ring},
        shapes,
    };

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

    fn ring(mesh: &PolyMesh, topology: &Topology, start: usize) -> Ring {
        let options = WalkOptions {
            stop_at_non_quads: true,
            ..WalkOptions::default()
        };
        edge_ring(mesh, topology, start, options, &BitSet::new(), &BitSet::new())
    }

    #[test]
    fn connecting_opposite_points_splits_a_quad() {
        let quad = shapes::grid(1, 1);
        // The face runs 0 → 2 → 3 → 1: points 0 and 3 are opposite.
        let out = connect(&quad, &Topology::build(&quad), Component::Point, &set(&[0, 3])).unwrap();
        assert_eq!((out.mesh.point_count(), out.mesh.face_count()), (4, 2));
        assert!(out.mesh.faces().all(|f| f.len() == 3));
        assert_eq!(out.face_origin, [0, 0], "both pieces come from the quad");
        assert_eq!(out.new_edges.len(), 1);
        assert_eq!(out.new_edge_corners().len(), 2, "the chord, once from each side");
        assert_clean(&out.mesh);
        let neighbours = connect(&quad, &Topology::build(&quad), Component::Point, &set(&[0, 2])).unwrap();
        assert_eq!(neighbours.mesh, quad, "neighbours need no new edge");
    }

    #[test]
    fn connecting_three_points_cuts_out_their_polygon() {
        let hexagon = PolyMesh::from_faces(
            (0..6u8)
                .map(|i| {
                    let a = std::f32::consts::TAU * f32::from(i) / 6.0;
                    Vec3::new(a.cos(), 0.0, -a.sin())
                })
                .collect(),
            [[0u32, 1, 2, 3, 4, 5]],
        )
        .unwrap();
        let out = connect(&hexagon, &Topology::build(&hexagon), Component::Point, &set(&[0, 2, 4])).unwrap();
        assert_eq!(out.mesh.face_count(), 4, "a triangle in the middle and three around it");
        assert_eq!(out.new_edges.len(), 3);
        assert_clean(&out.mesh);
    }

    #[test]
    fn connecting_an_edge_ring_cuts_a_loop_through_the_middle() {
        let cylinder = shapes::cylinder(8);
        let topology = Topology::build(&cylinder);
        let side = ring(&cylinder, &topology, edge(&topology, 0, 8));
        assert_eq!(side.edges.len(), 8, "the vertical side edges");
        let out = connect(&cylinder, &topology, Component::Edge, &side.edges).unwrap();
        assert_eq!((out.mesh.point_count(), out.mesh.face_count()), (16 + 8, 2 + 8 * 2));
        let result = assert_clean(&out.mesh);
        assert!(result.is_closed());
        assert_eq!(out.new_edges.len(), 8);
    }

    #[test]
    fn a_loop_cut_splits_the_strip_of_quads() {
        let cube = shapes::cube(2.0);
        let topology = Topology::build(&cube);
        // Points 0 (-,-,-) and 2 (-,+,-): a vertical edge; its ring runs round the sides.
        let start = edge(&topology, 0, 2);
        let strip = ring(&cube, &topology, start);
        assert_eq!((strip.edges.len(), strip.faces.len()), (4, 4));
        let out = loop_cut(&cube, &topology, &strip, start, &[0.5]).unwrap();
        assert_eq!((out.mesh.point_count(), out.mesh.face_count()), (12, 10));
        assert!(out.mesh.faces().all(|f| f.len() == 4), "all quads");
        assert!(
            out.mesh.points()[8..].iter().all(|p| p.y.abs() < 1e-6),
            "the loop runs round the middle"
        );
        let result = assert_clean(&out.mesh);
        assert!(result.is_closed());
        assert_eq!(out.new_edges.len(), 4);

        let three = loop_cut(&cube, &topology, &strip, start, &[0.25, 0.5, 0.75]).unwrap();
        assert_eq!((three.mesh.point_count(), three.mesh.face_count()), (8 + 12, 6 + 12));
        assert_clean(&three.mesh);
        // The cut stays on the same side of every quad: a quarter up from the bottom
        // (the first point of the start edge is the lower one).
        let low = loop_cut(&cube, &topology, &strip, start, &[0.25]).unwrap();
        assert!(
            low.mesh.points()[8..].iter().all(|p| (p.y + 0.5).abs() < 1e-6),
            "{:?}",
            &low.mesh.points()[8..]
        );
    }

    #[test]
    fn faces_at_the_ends_of_a_loop_gain_the_new_points() {
        let cylinder = shapes::cylinder(8);
        let topology = Topology::build(&cylinder);
        // Bottom edge of the first side quad: its ring crosses that quad to the top
        // edge, whose other faces are the n-gon caps.
        let start = edge(&topology, 0, 1);
        let strip = ring(&cylinder, &topology, start);
        assert_eq!(strip.faces.len(), 1);
        let out = loop_cut(&cylinder, &topology, &strip, start, &[0.5]).unwrap();
        assert_eq!(out.mesh.face_count(), 10 + 1);
        let result = assert_clean(&out.mesh);
        assert!(result.is_closed(), "the caps took the new points: no crack");
        assert!(
            out.mesh.faces().filter(|f| f.len() == 9).count() == 2,
            "both caps grew a point"
        );
    }

    #[test]
    fn a_line_cut_splits_the_faces_it_crosses() {
        let plane = shapes::grid(2, 1);
        let topology = Topology::build(&plane);
        // From above, a line along z at x = 0.5: the plane x = 0.5.
        let eye = Vec3::new(0.5, 5.0, 0.5);
        let cut = LineCut {
            eye,
            from: Vec3::new(0.5, 0.0, -1.0) - eye,
            to: Vec3::new(0.5, 0.0, 2.0) - eye,
            infinite: false,
            connect: true,
        };
        let out = line_cut(&plane, &topology, &cut, |_| true).unwrap();
        assert_eq!((out.mesh.point_count(), out.mesh.face_count()), (6 + 2, 3));
        assert_eq!(out.face_origin, [0, 0, 1]);
        assert_eq!(out.new_edges.len(), 1);
        assert_clean(&out.mesh);

        // A short line only reaches what lies between its ends.
        let short = LineCut {
            from: Vec3::new(0.5, 0.0, -1.0) - eye,
            to: Vec3::new(0.5, 0.0, -0.5) - eye,
            ..cut
        };
        let none = line_cut(&plane, &topology, &short, |_| true).unwrap();
        assert_eq!(none.mesh, plane);
        let infinite = LineCut {
            infinite: true,
            ..short
        };
        assert_eq!(
            line_cut(&plane, &topology, &infinite, |_| true)
                .unwrap()
                .mesh
                .face_count(),
            3
        );

        let points_only = LineCut { connect: false, ..cut };
        let out = line_cut(&plane, &topology, &points_only, |_| true).unwrap();
        assert_eq!((out.mesh.point_count(), out.mesh.face_count()), (8, 2));
        assert_eq!(out.mesh.face(0).len(), 6, "the points sit on the face's edges");
    }

    #[test]
    fn visible_only_leaves_the_back_alone() {
        let cube = shapes::cube(2.0);
        let topology = Topology::build(&cube);
        // From the front (+z), a horizontal line across the middle.
        let eye = Vec3::new(0.0, 0.0, 6.0);
        let cut = LineCut {
            eye,
            from: Vec3::new(-3.0, 0.0, 1.0) - eye,
            to: Vec3::new(3.0, 0.0, 1.0) - eye,
            infinite: false,
            connect: true,
        };
        let all = line_cut(&cube, &topology, &cut, |_| true).unwrap();
        assert_eq!(all.mesh.face_count(), 6 + 4, "front, back and both sides split");
        assert_clean(&all.mesh);
        // Only the front half is visible from there.
        let front = line_cut(&cube, &topology, &cut, |x| x.z > 0.0).unwrap();
        assert_eq!(front.mesh.face_count(), 6 + 1, "only the front face splits");
        assert_eq!(front.mesh.point_count(), 8 + 2);
        assert_clean(&front.mesh);
    }

    #[test]
    fn a_cut_through_a_point_uses_it() {
        let quad = shapes::grid(1, 1);
        let topology = Topology::build(&quad);
        // The diagonal from point 0 (0,0) to point 3 (1,1), seen from above.
        let eye = Vec3::new(0.5, 5.0, 0.5);
        let cut = LineCut {
            eye,
            from: Vec3::new(-0.5, 0.0, -0.5) - eye,
            to: Vec3::new(1.5, 0.0, 1.5) - eye,
            infinite: false,
            connect: true,
        };
        let out = line_cut(&quad, &topology, &cut, |_| true).unwrap();
        assert_eq!((out.mesh.point_count(), out.mesh.face_count()), (4, 2), "no new points");
        assert_clean(&out.mesh);
    }
}
