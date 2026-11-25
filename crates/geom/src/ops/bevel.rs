//! Bevel: edges become strips of polygons, points are cut off.

use std::collections::{BTreeMap, HashMap, HashSet};

use glam::{Vec2, Vec3};

use super::{Builder, Corner, Rebuilt, corner_faces, next_corner};
use crate::{BitSet, MeshError, PolyMesh, Topology, us};

/// Bevel options (see `docs/reference/modeling-tools.md`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BevelOptions {
    /// How far the new points move from the original edges or points.
    pub offset: f32,
    /// Rows across a bevelled edge, along a rounded profile (0: a flat chamfer).
    pub subdivision: u32,
    /// The profile's bulge with subdivision: 1 round, 0 flat, negative hollow.
    pub depth: f32,
    /// Clamp the offset so that new points never pass each other.
    pub limit: bool,
    /// Round (Subdivision and Depth) or the Custom profile.
    pub shape: BevelShape,
    /// The Custom profile's points.
    pub profile: BevelProfile,
    /// How corners where several bevelled edges meet are filled.
    pub mitres: Mitres,
}

/// A bevel's cross-section.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BevelShape {
    /// Subdivision rows bulged by Depth.
    #[default]
    Round,
    /// Rows through the points of [`BevelProfile`].
    Custom,
}

impl BevelShape {
    pub const ALL: [Self; 2] = [Self::Round, Self::Custom];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Round => "Round",
            Self::Custom => "Custom",
        }
    }
}

/// How a corner where three or more bevelled edges meet is filled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Mitres {
    /// One polygon over the hole.
    #[default]
    Default,
    /// Triangles to a point pulled towards the corner, so the profile turns
    /// the corner.
    Patch,
}

impl Mitres {
    pub const ALL: [Self; 2] = [Self::Default, Self::Patch];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Patch => "Patch",
        }
    }
}

/// A Custom bevel profile: up to [`BevelProfile::MAX`] points from (1, 0) to
/// (0, 1) in the corner's own axes — (s, t) is the original edge's point
/// plus `s` times the way to the first face's side and `t` times the way to
/// the second's. Kept in a fixed array so options stay plain values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BevelProfile {
    points: [Vec2; BevelProfile::MAX],
    count: u8,
}

/// The ready-made Custom profiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProfilePreset {
    /// A straight cut.
    Chamfer,
    /// A step into the corner.
    Step,
    /// A hollow quarter circle.
    Cove,
}

impl ProfilePreset {
    pub const ALL: [Self; 3] = [Self::Chamfer, Self::Step, Self::Cove];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Chamfer => "Chamfer",
            Self::Step => "Step",
            Self::Cove => "Cove",
        }
    }
}

impl BevelProfile {
    /// The most points a profile holds.
    pub const MAX: usize = 16;

    /// A profile through `points` (the first `MAX`; fewer than two: a chamfer).
    #[must_use]
    pub fn new(points: &[Vec2]) -> Self {
        let mut profile = Self {
            points: [Vec2::ZERO; Self::MAX],
            count: 0,
        };
        for (slot, &point) in profile.points.iter_mut().zip(points.iter().filter(|p| p.is_finite())) {
            *slot = point;
            profile.count += 1;
        }
        if profile.count < 2 {
            return Self::preset(ProfilePreset::Chamfer);
        }
        profile
    }

    #[must_use]
    pub fn preset(preset: ProfilePreset) -> Self {
        match preset {
            ProfilePreset::Chamfer => Self::new(&[Vec2::X, Vec2::Y]),
            ProfilePreset::Step => Self::new(&[
                Vec2::X,
                Vec2::new(0.5, 0.0),
                Vec2::new(0.5, 0.5),
                Vec2::new(0.0, 0.5),
                Vec2::Y,
            ]),
            ProfilePreset::Cove => {
                let arc: Vec<Vec2> = (0..=6)
                    .map(|k| {
                        let a = std::f32::consts::FRAC_PI_2 * k as f32 / 6.0;
                        // The ends exactly (cos 90° is not quite 0 in f32).
                        match k {
                            0 => Vec2::X,
                            6 => Vec2::Y,
                            _ => Vec2::new(a.cos(), a.sin()),
                        }
                    })
                    .collect();
                Self::new(&arc)
            }
        }
    }

    /// Its points, in order.
    #[must_use]
    pub fn points(&self) -> &[Vec2] {
        &self.points[..usize::from(self.count)]
    }
}

impl Default for BevelProfile {
    fn default() -> Self {
        Self::preset(ProfilePreset::Chamfer)
    }
}

/// Longest a mitred point may move, as a multiple of the offset.
const MAX_MITRE: f32 = 4.0;

/// Bevels `edges` (unique edge indices; only those between two faces) into strips
/// and cuts off `points`. Every point at a bevelled edge or in `points` is replaced
/// by new points: where a face's corner has one bevelled edge, its point slides the
/// offset along the other edge; where both are bevelled it moves into the face,
/// the offset away from both; corners with neither (and bevelled points) are cut
/// between the two edges. The hole left at each replaced point is filled by one
/// face. The strips' long edges are the result's new edges.
pub fn bevel(
    mesh: &PolyMesh,
    topology: &Topology,
    edges: &BitSet,
    points: &BitSet,
    options: &BevelOptions,
) -> Result<Rebuilt, MeshError> {
    let corner_faces = corner_faces(mesh);
    let bevelled: BitSet = edges.iter().filter(|&e| topology.edge_faces(e).len() == 2).collect();
    let mut affected: BitSet = points
        .iter()
        .filter(|&p| p < mesh.point_count() && !topology.point_faces(p).is_empty())
        .collect();
    for e in bevelled.iter() {
        let [a, b] = topology.edges()[e];
        affected.insert(us(a));
        affected.insert(us(b));
    }
    let mut builder = Builder::new(mesh);
    if affected.is_empty() || options.offset <= 0.0 {
        for f in 0..mesh.face_count() {
            builder.keep(f);
        }
        return builder.finish();
    }
    let position = |p: u32| mesh.points()[us(p)];
    let mut offset = options.offset;
    if options.limit {
        // A point slides along an edge: it must stay short of the far end, or of
        // the middle when the far end slides too.
        for v in affected.iter() {
            for &e in topology.point_edges(v) {
                if bevelled.contains(us(e)) {
                    continue;
                }
                let [a, b] = topology.edges()[us(e)];
                let length = position(a).distance(position(b));
                let other = if us(a) == v { b } else { a };
                let room = if affected.contains(us(other)) {
                    length * 0.5
                } else {
                    length
                };
                offset = offset.min(room * 0.99);
            }
        }
    }

    // What replaces each corner at an affected point, and which point each new
    // point stands in for (to find the holes afterwards).
    let mut on_edge: HashMap<(usize, u32), u32> = HashMap::new();
    let mut owner: HashMap<u32, u32> = HashMap::new();
    let mut replace: HashMap<usize, Vec<Corner>> = HashMap::new();
    for f in 0..mesh.face_count() {
        let range = mesh.corner_range(f);
        for c in range.clone() {
            let v = mesh.corner_points()[c];
            if !affected.contains(us(v)) {
                continue;
            }
            let prev = if c == range.start { range.end - 1 } else { c - 1 };
            let next = next_corner(mesh, f, c);
            let (Some(e_in), Some(e_out)) = (topology.corner_edge(prev), topology.corner_edge(c)) else {
                continue;
            };
            let (e_in, e_out) = (us(e_in), us(e_out));
            let (p_prev, p_next) = (mesh.corner_points()[prev], mesh.corner_points()[next]);
            let mut slide = |builder: &mut Builder<'_>, edge: usize, toward: u32, toward_corner: usize| {
                let to = position(toward) - position(v);
                let length = to.length().max(f32::EPSILON);
                let point = *on_edge.entry((edge, v)).or_insert_with(|| {
                    let q = builder.add_point(position(v) + to / length * offset);
                    owner.insert(q, v);
                    q
                });
                Corner::Point(point, builder.uv(c).lerp(builder.uv(toward_corner), offset / length))
            };
            let corners = match (bevelled.contains(e_in), bevelled.contains(e_out)) {
                (false, false) => vec![
                    slide(&mut builder, e_in, p_prev, prev),
                    slide(&mut builder, e_out, p_next, next),
                ],
                (true, false) => vec![slide(&mut builder, e_out, p_next, next)],
                (false, true) => vec![slide(&mut builder, e_in, p_prev, prev)],
                (true, true) => {
                    let (u_in, u_out) = (
                        (position(p_prev) - position(v)).normalize_or_zero(),
                        (position(p_next) - position(v)).normalize_or_zero(),
                    );
                    let half = u_in.angle_between(u_out) * 0.5;
                    let bisector = (u_in + u_out).normalize_or_zero();
                    // Straight through (180°): into the face, left of the outgoing edge.
                    let direction = if bisector == Vec3::ZERO {
                        mesh.face_normal(f).cross(u_out).normalize_or_zero()
                    } else {
                        bisector
                    };
                    let distance = (offset / half.sin().max(1.0 / MAX_MITRE)).min(offset * MAX_MITRE);
                    let q = builder.add_point(position(v) + direction * distance);
                    owner.insert(q, v);
                    vec![Corner::Point(q, builder.uv(c))]
                }
            };
            replace.insert(c, corners);
        }
    }

    // The original faces, their corners at affected points replaced.
    let mut corners = Vec::new();
    for f in 0..mesh.face_count() {
        corners.clear();
        for c in mesh.corner_range(f) {
            match replace.get(&c) {
                Some(new) => corners.extend_from_slice(new),
                None => corners.push(Corner::Old(c)),
            }
        }
        builder.push(&corners, f);
    }

    // A strip along every bevelled edge: corner c1 runs p → q in its face, c2 runs
    // q → p in the other; each side point is the one point replacing that corner.
    let single = |c: usize| replace.get(&c).and_then(|r| (r.len() == 1).then(|| r[0]));
    for e in bevelled.iter() {
        let &[c1, c2] = topology.edge_corners(e) else { continue };
        let (c1, c2) = (us(c1), us(c2));
        let (n1, n2) = (
            next_corner(mesh, us(corner_faces[c1]), c1),
            next_corner(mesh, us(corner_faces[c2]), c2),
        );
        let (p, q) = (mesh.corner_points()[c1], mesh.corner_points()[n1]);
        if mesh.corner_points()[c2] != q || mesh.corner_points()[n2] != p {
            continue; // inconsistently wound neighbours
        }
        let (Some(side1_p), Some(side1_q), Some(side2_q), Some(side2_p)) =
            (single(c1), single(n1), single(c2), single(n2))
        else {
            continue;
        };
        let at_p = profile(&mut builder, &mut owner, side1_p, side2_p, p, options);
        let at_q = profile(&mut builder, &mut owner, side1_q, side2_q, q, options);
        for k in 0..at_p.len() - 1 {
            builder.push_new(&[at_q[k], at_p[k], at_p[k + 1], at_q[k + 1]]);
        }
        for (a, b) in [(side1_p, side1_q), (side2_p, side2_q)] {
            let (a, b) = (builder.point(a), builder.point(b));
            builder.new_edge(a, b);
        }
    }

    // Fill the hole at every replaced point: the edges between its new points that
    // no face has from the other side, chained into loops.
    let mut directed: HashSet<(u32, u32)> = HashSet::new();
    for w in builder.starts.windows(2) {
        let face = &builder.corners[w[0]..w[1]];
        for (i, &a) in face.iter().enumerate() {
            let b = face[(i + 1) % face.len()];
            if owner.get(&a).is_some_and(|v| owner.get(&b) == Some(v)) {
                directed.insert((a, b));
            }
        }
    }
    let mut holes: BTreeMap<u32, BTreeMap<u32, u32>> = BTreeMap::new();
    let mut pinched: HashSet<u32> = HashSet::new();
    for &(a, b) in &directed {
        if !directed.contains(&(b, a)) {
            let v = owner[&a];
            if holes.entry(v).or_default().insert(b, a).is_some() {
                pinched.insert(v);
            }
        }
    }
    for (v, next) in &holes {
        if pinched.contains(v) {
            continue;
        }
        let mut visited: HashSet<u32> = HashSet::new();
        for &start in next.keys() {
            if visited.contains(&start) {
                continue;
            }
            let mut cycle = vec![start];
            visited.insert(start);
            let mut at = start;
            while let Some(&to) = next.get(&at) {
                if to == start {
                    break;
                }
                if !visited.insert(to) {
                    cycle.clear(); // ran into another chain: not a closed loop
                    break;
                }
                cycle.push(to);
                at = to;
            }
            let closed = next.get(&at) == Some(&start);
            if closed && cycle.len() >= 3 && options.mitres == Mitres::Patch && cycle.len() > 3 {
                // Triangles to a point pulled from the hole's middle towards
                // the original corner, as far as the profile bulges.
                let middle = cycle.iter().map(|&p| builder.position(p)).sum::<Vec3>() / cycle.len() as f32;
                let pull = match options.shape {
                    BevelShape::Round => options.depth * 0.5,
                    BevelShape::Custom => 0.3,
                };
                let centre = builder.add_point(middle + (position(*v) - middle) * pull);
                for (i, &a) in cycle.iter().enumerate() {
                    let b = cycle[(i + 1) % cycle.len()];
                    builder.push_new(&[
                        Corner::Point(a, Vec2::ZERO),
                        Corner::Point(b, Vec2::ZERO),
                        Corner::Point(centre, Vec2::ZERO),
                    ]);
                }
            } else if closed && cycle.len() >= 3 {
                let patch: Vec<Corner> = cycle.iter().map(|&p| Corner::Point(p, Vec2::ZERO)).collect();
                builder.push_new(&patch);
            }
        }
    }
    builder.finish()
}

/// The points across a bevelled edge at its point `corner`, from `from` (the side
/// in the first face) to `to`: along a quadratic curve whose control point is
/// pulled from the chord's middle towards the original point by `depth`.
fn profile(
    builder: &mut Builder<'_>,
    owner: &mut HashMap<u32, u32>,
    from: Corner,
    to: Corner,
    corner: u32,
    options: &BevelOptions,
) -> Vec<Corner> {
    let (a, b) = (
        builder.position(builder.point(from)),
        builder.position(builder.point(to)),
    );
    if options.shape == BevelShape::Custom {
        // In the corner's axes: (1, 0) is `from`, (0, 1) is `to`.
        let origin = builder.position(corner);
        let (along_a, along_b) = (a - origin, b - origin);
        let points = options.profile.points();
        let mut column = Vec::with_capacity(points.len());
        column.push(from);
        for p in &points[1..points.len() - 1] {
            let q = builder.add_point(origin + along_a * p.x + along_b * p.y);
            owner.insert(q, corner);
            column.push(Corner::Point(q, Vec2::ZERO));
        }
        column.push(to);
        return column;
    }
    let middle = (a + b) * 0.5;
    let control = middle + (builder.position(corner) - middle) * options.depth;
    let steps = options.subdivision + 1;
    let mut column = Vec::with_capacity(usize::try_from(steps).unwrap_or(0) + 1);
    column.push(from);
    for k in 1..steps {
        let t = f32::from(u16::try_from(k).unwrap_or(u16::MAX)) / f32::from(u16::try_from(steps).unwrap_or(u16::MAX));
        let point = a * (1.0 - t) * (1.0 - t) + control * 2.0 * (1.0 - t) * t + b * t * t;
        let q = builder.add_point(point);
        owner.insert(q, corner);
        column.push(Corner::Point(q, Vec2::ZERO));
    }
    column.push(to);
    column
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes;

    const OPTIONS: BevelOptions = BevelOptions {
        offset: 0.2,
        subdivision: 0,
        depth: 1.0,
        shape: BevelShape::Round,
        profile: BevelProfile {
            points: [Vec2::ZERO; BevelProfile::MAX],
            count: 0,
        },
        mitres: Mitres::Default,
        limit: true,
    };

    fn set(items: &[usize]) -> BitSet {
        items.iter().copied().collect()
    }

    fn edge(topology: &Topology, a: u32, b: u32) -> usize {
        topology
            .edges()
            .iter()
            .position(|&e| e == [a.min(b), a.max(b)])
            .unwrap()
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

    #[test]
    fn one_cube_edge_becomes_a_strip() {
        let cube = shapes::cube(2.0);
        let topology = Topology::build(&cube);
        // Points 0 (-,-,-) and 1 (+,-,-): a bottom edge along x.
        let e = set(&[edge(&topology, 0, 1)]);
        let out = bevel(&cube, &topology, &e, &BitSet::new(), &OPTIONS).unwrap();
        assert_eq!((out.mesh.point_count(), out.mesh.face_count()), (10, 7));
        let result = assert_clean(&out.mesh);
        assert!(result.is_closed());
        assert_eq!(result.euler_characteristic(), 2);
        assert_eq!(out.new_edges.len(), 2, "the strip's two long edges");

        let round = bevel(
            &cube,
            &topology,
            &e,
            &BitSet::new(),
            &BevelOptions {
                subdivision: 2,
                ..OPTIONS
            },
        )
        .unwrap();
        // 3 strip quads, and at each end a thin polygon between the curved end and
        // the flat side (in the side's plane).
        assert_eq!((round.mesh.point_count(), round.mesh.face_count()), (14, 6 + 3 + 2));
        assert!(assert_clean(&round.mesh).is_closed());
        // The profile bulges out: its middle points lie beyond the flat chamfer's plane.
        let chamfer_distance = (1.0 - 0.2_f32) / 2f32.sqrt() + 0.2 / 2f32.sqrt();
        let bulge = round.mesh.points()[10..]
            .iter()
            .map(|p| (Vec3::new(0.0, -1.0, -1.0).normalize()).dot(*p))
            .fold(f32::MIN, f32::max);
        assert!(bulge > chamfer_distance + 1e-3, "{bulge} vs {chamfer_distance}");
    }

    #[test]
    fn three_edges_at_a_corner_leave_a_triangle() {
        let cube = shapes::cube(2.0);
        let topology = Topology::build(&cube);
        let corner = set(&[edge(&topology, 0, 1), edge(&topology, 0, 2), edge(&topology, 0, 4)]);
        let out = bevel(&cube, &topology, &corner, &BitSet::new(), &OPTIONS).unwrap();
        assert_eq!((out.mesh.point_count(), out.mesh.face_count()), (13, 10));
        assert!(out.mesh.faces().any(|f| f.len() == 3), "the corner triangle");
        assert!(assert_clean(&out.mesh).is_closed());
    }

    #[test]
    fn a_bevelled_point_is_cut_off() {
        let cube = shapes::cube(2.0);
        let out = bevel(&cube, &Topology::build(&cube), &BitSet::new(), &set(&[0]), &OPTIONS).unwrap();
        assert_eq!((out.mesh.point_count(), out.mesh.face_count()), (10, 7));
        assert!(assert_clean(&out.mesh).is_closed());
        let triangle = out.mesh.faces().find(|f| f.len() == 3).unwrap();
        for &p in triangle {
            let p = out.mesh.points()[us(p)];
            assert!(
                (p.distance(Vec3::splat(-1.0)) - 0.2).abs() < 1e-5,
                "0.2 from the corner: {p}"
            );
        }
    }

    #[test]
    fn edges_ending_inside_a_plane_get_a_triangle_at_each_end() {
        let plane = shapes::grid(3, 3);
        let topology = Topology::build(&plane);
        // Points 5 (1,1) and 6 (2,1): both inside, four edges each.
        let out = bevel(
            &plane,
            &topology,
            &set(&[edge(&topology, 5, 6)]),
            &BitSet::new(),
            &OPTIONS,
        )
        .unwrap();
        assert_eq!((out.mesh.point_count(), out.mesh.face_count()), (16 - 2 + 6, 9 + 1 + 2));
        assert_clean(&out.mesh);
        // Ending at the border: no triangles, the border just moves.
        let strip = shapes::grid(3, 1);
        let topology = Topology::build(&strip);
        let out = bevel(
            &strip,
            &topology,
            &set(&[edge(&topology, 1, 5)]),
            &BitSet::new(),
            &OPTIONS,
        )
        .unwrap();
        assert_eq!((out.mesh.point_count(), out.mesh.face_count()), (10, 4));
        assert_clean(&out.mesh);
    }

    #[test]
    fn limit_keeps_new_points_from_passing_each_other() {
        let cube = shapes::cube(2.0);
        let topology = Topology::build(&cube);
        let e = set(&[edge(&topology, 0, 1)]);
        let out = bevel(
            &cube,
            &topology,
            &e,
            &BitSet::new(),
            &BevelOptions { offset: 5.0, ..OPTIONS },
        )
        .unwrap();
        assert_clean(&out.mesh);
        for p in out.mesh.points() {
            assert!(p.abs().max_element() <= 1.0 + 1e-5, "inside the cube: {p}");
        }
    }

    #[test]
    fn border_edges_and_nothing_selected_change_nothing() {
        let quad = shapes::grid(1, 1);
        let topology = Topology::build(&quad);
        let out = bevel(&quad, &topology, &set(&[0, 1, 2, 3]), &BitSet::new(), &OPTIONS).unwrap();
        assert_eq!(out.mesh, quad);
        let none = bevel(&quad, &topology, &BitSet::new(), &BitSet::new(), &OPTIONS).unwrap();
        assert_eq!(none.mesh, quad);
    }

    #[test]
    fn a_custom_profile_gives_one_row_per_segment() {
        let cube = shapes::cube(2.0);
        let topology = Topology::build(&cube);
        let edge = edge(&topology, 0, 1);
        for preset in ProfilePreset::ALL {
            let profile = BevelProfile::preset(preset);
            let options = BevelOptions {
                shape: BevelShape::Custom,
                profile,
                subdivision: 5,
                ..OPTIONS
            };
            let out = bevel(&cube, &topology, &set(&[edge]), &BitSet::new(), &options).unwrap();
            let rows = profile.points().len() - 1;
            let ends = if rows > 1 { 2 } else { 0 };
            assert_eq!(
                out.mesh.face_count(),
                6 + rows + ends,
                "{preset:?}: the strip's rows follow its points"
            );
            assert_clean(&out.mesh);
        }
    }

    #[test]
    fn patch_mitres_fill_a_cube_corner_without_a_gap() {
        let cube = shapes::cube(2.0);
        let topology = Topology::build(&cube);
        let all: BitSet = (0..topology.edge_count()).collect();
        let options = BevelOptions {
            subdivision: 2,
            mitres: Mitres::Patch,
            ..OPTIONS
        };
        let out = bevel(&cube, &topology, &all, &BitSet::new(), &options).unwrap();
        assert!(assert_clean(&out.mesh).is_closed(), "no gap at the corners");
        let default = bevel(
            &cube,
            &topology,
            &all,
            &BitSet::new(),
            &BevelOptions {
                subdivision: 2,
                ..OPTIONS
            },
        )
        .unwrap();
        assert!(
            out.mesh.face_count() > default.mesh.face_count(),
            "the corners are patched with triangles"
        );
    }
}
