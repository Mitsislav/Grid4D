//! Surfaces made from splines: the Extrude, Lathe, Sweep and Loft generators'
//! bodies (see `docs/reference/generator-extrude.md`, `generator-lathe.md`,
//! `generator-sweep.md`, `generator-loft.md`). Pure functions of sampled
//! [`Polyline`]s.

use glam::{Vec2, Vec3};

use crate::{PolyMesh, spline::Polyline, triangulate_outlines, u32_index};

/// How an extrusion finds its movement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ExtrudeDirection {
    /// Along the axis closest to the perpendicular of the spline's plane.
    #[default]
    Auto,
    X,
    Y,
    Z,
    /// Along [`Extrusion::movement`]'s direction.
    Custom,
    /// Exactly [`Extrusion::movement`].
    Absolute,
}

impl ExtrudeDirection {
    pub const ALL: [Self; 6] = [Self::Auto, Self::X, Self::Y, Self::Z, Self::Custom, Self::Absolute];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::X => "X",
            Self::Y => "Y",
            Self::Z => "Z",
            Self::Custom => "Custom",
            Self::Absolute => "Absolute",
        }
    }
}

/// How caps are cut into polygons.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CapType {
    /// One polygon per outline without holes; triangles where holes need them.
    #[default]
    NGons,
    /// Every cap triangulated.
    Triangles,
}

impl CapType {
    pub const ALL: [Self; 2] = [Self::NGons, Self::Triangles];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::NGons => "N-Gons",
            Self::Triangles => "Triangles",
        }
    }
}

/// The Extrude generator's options: how far a spline is pushed, and how its
/// solid is closed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extrusion {
    pub direction: ExtrudeDirection,
    /// Length of the movement (cm) for every direction but Absolute; negative
    /// reverses it.
    pub offset: f32,
    /// Custom: the direction; Absolute: the movement itself (cm).
    pub movement: Vec3,
    /// Rings of polygons along the movement (1 – [`Extrusion::MAX_SUBDIVISION`]).
    pub subdivision: u32,
    /// Turns the shell inside out (caps untouched).
    pub reverse_normals: bool,
    pub start_cap: bool,
    pub end_cap: bool,
    pub cap_type: CapType,
}

impl Default for Extrusion {
    fn default() -> Self {
        Self {
            direction: ExtrudeDirection::Auto,
            offset: 20.0,
            movement: Vec3::new(0.0, 0.0, 20.0),
            subdivision: 1,
            reverse_normals: false,
            start_cap: true,
            end_cap: true,
            cap_type: CapType::NGons,
        }
    }
}

impl Extrusion {
    pub const MAX_SUBDIVISION: u32 = 4000;

    /// The movement for `polyline` (in the polyline's space).
    #[must_use]
    pub fn vector(&self, polyline: &Polyline) -> Vec3 {
        match self.direction {
            ExtrudeDirection::Auto => plane_axis(polyline) * self.offset,
            ExtrudeDirection::X => Vec3::X * self.offset,
            ExtrudeDirection::Y => Vec3::Y * self.offset,
            ExtrudeDirection::Z => Vec3::Z * self.offset,
            ExtrudeDirection::Custom => self.movement.normalize_or_zero() * self.offset,
            ExtrudeDirection::Absolute => self.movement,
        }
    }
}

/// The axis (X, Y or Z, positive) closest to the perpendicular of the plane the
/// curve lies in; Z for a straight line.
fn plane_axis(polyline: &Polyline) -> Vec3 {
    let normal: Vec3 = polyline
        .runs()
        .map(|(points, _)| crate::mesh::newell(points.iter().copied()))
        .sum();
    let a = normal.abs();
    if a.max_element() <= f32::EPSILON || (a.z >= a.x && a.z >= a.y) {
        Vec3::Z
    } else if a.x >= a.y {
        Vec3::X
    } else {
        Vec3::Y
    }
}

/// One run of the polyline as a surface uses it.
struct Run<'a> {
    points: &'a [Vec3],
    closed: bool,
    /// Projected onto the plane caps are made in.
    flat: Vec<Vec2>,
    /// Closed runs: how many other closed runs contain it (even: an outline,
    /// odd: a hole).
    depth: usize,
    /// Its points in the order that faces outwards: outlines counter-clockwise
    /// and holes clockwise in the flat plane; open runs as drawn.
    order: Vec<u32>,
}

/// The runs of `polyline` with two points or more, projected by `flat`,
/// nested even-odd and ordered outwards.
fn profile<'a>(polyline: &'a Polyline, flat: impl Fn(Vec3) -> Vec2) -> Vec<Run<'a>> {
    let mut runs: Vec<Run<'a>> = polyline
        .runs()
        .filter(|(points, _)| points.len() >= 2)
        .map(|(points, closed)| Run {
            points,
            closed: closed && points.len() >= 3,
            flat: points.iter().map(|&p| flat(p)).collect(),
            depth: 0,
            order: (0..u32_index(points.len())).collect(),
        })
        .collect();
    for i in 0..runs.len() {
        if !runs[i].closed {
            continue;
        }
        let probe = runs[i].flat[0];
        runs[i].depth = (0..runs.len())
            .filter(|&j| j != i && runs[j].closed && contains(&runs[j].flat, probe))
            .count();
        let counter_clockwise = area(&runs[i].flat) > 0.0;
        if counter_clockwise != runs[i].depth.is_multiple_of(2) {
            runs[i].order.reverse();
        }
    }
    runs
}

/// The cap polygons of `runs`: each outline with its direct holes cut out, as
/// (run, point) pairs, counter-clockwise in the flat plane.
fn caps(runs: &[Run<'_>], cap_type: CapType) -> Vec<Vec<(usize, u32)>> {
    let mut polygons = Vec::new();
    for (i, outline) in runs.iter().enumerate() {
        if !outline.closed || !outline.depth.is_multiple_of(2) {
            continue;
        }
        let holes: Vec<usize> = (0..runs.len())
            .filter(|&j| {
                j != i
                    && runs[j].closed
                    && runs[j].depth == outline.depth + 1
                    && contains(&outline.flat, runs[j].flat[0])
            })
            .collect();
        if holes.is_empty() && cap_type == CapType::NGons {
            polygons.push(outline.order.iter().map(|&p| (i, p)).collect());
            continue;
        }
        let hole_flats: Vec<&[Vec2]> = holes.iter().map(|&h| runs[h].flat.as_slice()).collect();
        let mut owners: Vec<(usize, u32)> = (0..u32_index(outline.points.len())).map(|p| (i, p)).collect();
        for &hole in &holes {
            owners.extend((0..u32_index(runs[hole].points.len())).map(|p| (hole, p)));
        }
        for triangle in triangulate_outlines(&outline.flat, &hole_flats) {
            polygons.push(
                triangle
                    .iter()
                    .filter_map(|&c| owners.get(crate::us(c)).copied())
                    .collect(),
            );
        }
    }
    polygons
}

/// A mesh from `points` and `faces`; repeated corners are dropped from a face
/// (welded points, a triangle across a bridge) and faces left with fewer than
/// three are skipped.
fn assemble(points: Vec<Vec3>, faces: &[Vec<u32>]) -> PolyMesh {
    let mut mesh = PolyMesh::from_faces(points, Vec::<Vec<u32>>::new()).unwrap_or_default();
    let mut corners = Vec::new();
    for face in faces {
        corners.clear();
        for &c in face {
            if !corners.contains(&c) {
                corners.push(c);
            }
        }
        if corners.len() >= 3 {
            let _ = mesh.add_face(&corners);
        }
    }
    mesh
}

/// The solid `polyline` makes when it is pushed along `extrusion`'s movement:
/// a shell of `subdivision` rings per run, and caps on its closed outlines with
/// their holes cut out. Empty for a movement of zero length or a polyline
/// without runs of two points.
#[must_use]
pub fn extrude(polyline: &Polyline, extrusion: &Extrusion) -> PolyMesh {
    let movement = extrusion.vector(polyline);
    let Some(dir) = movement.try_normalize().filter(|_| movement.is_finite()) else {
        return PolyMesh::new();
    };
    // u × v = dir: counter-clockwise in 2D is counter-clockwise around dir.
    let u = dir.any_orthonormal_vector();
    let v = dir.cross(u);
    let rings = extrusion.subdivision.clamp(1, Extrusion::MAX_SUBDIVISION);
    let runs = profile(polyline, |p| Vec2::new(p.dot(u), p.dot(v)));

    let mut points = Vec::new();
    let mut bases = Vec::with_capacity(runs.len());
    let steps = f32::from(u16::try_from(rings).unwrap_or(u16::MAX));
    for run in &runs {
        bases.push(u32_index(points.len()));
        for k in 0..=rings {
            let along = movement * (f32::from(u16::try_from(k).unwrap_or(u16::MAX)) / steps);
            points.extend(run.points.iter().map(|&p| p + along));
        }
    }
    let at = |run: usize, ring: u32, local: u32| bases[run] + ring * u32_index(runs[run].points.len()) + local;
    let mut faces: Vec<Vec<u32>> = Vec::new();
    // The shell: each edge of each run swept ring by ring.
    for (r, run) in runs.iter().enumerate() {
        let n = run.order.len();
        let edges = if run.closed { n } else { n - 1 };
        for e in 0..edges {
            let (a, b) = (run.order[e], run.order[(e + 1) % n]);
            for k in 0..rings {
                let mut quad = vec![at(r, k, a), at(r, k, b), at(r, k + 1, b), at(r, k + 1, a)];
                if extrusion.reverse_normals {
                    quad.reverse();
                }
                faces.push(quad);
            }
        }
    }
    // The caps: counter-clockwise around dir faces forward (the end).
    for polygon in caps(&runs, extrusion.cap_type) {
        if extrusion.end_cap {
            faces.push(polygon.iter().map(|&(run, p)| at(run, rings, p)).collect());
        }
        if extrusion.start_cap {
            faces.push(polygon.iter().rev().map(|&(run, p)| at(run, 0, p)).collect());
        }
    }
    assemble(points, &faces)
}

/// The Lathe generator's options: how far a profile turns around the Y axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lathe {
    /// Radians; a full turn is ±τ.
    pub angle: f32,
    /// Steps along the turn (3 – [`Lathe::MAX_SUBDIVISION`]).
    pub subdivision: u32,
    /// Distance along Y from start to end (cm): a helix.
    pub movement: f32,
    /// The profile's size at the end (1.0 = 100 %), about the origin.
    pub scaling: f32,
    /// Turns the shell inside out (caps untouched).
    pub reverse_normals: bool,
    pub start_cap: bool,
    pub end_cap: bool,
    pub cap_type: CapType,
}

impl Default for Lathe {
    fn default() -> Self {
        Self {
            angle: std::f32::consts::TAU,
            subdivision: 24,
            movement: 0.0,
            scaling: 1.0,
            reverse_normals: false,
            start_cap: true,
            end_cap: true,
            cap_type: CapType::NGons,
        }
    }
}

impl Lathe {
    pub const MIN_SUBDIVISION: u32 = 3;
    pub const MAX_SUBDIVISION: u32 = 4000;

    /// Closes on itself: a full turn that ends where it started.
    #[must_use]
    pub fn is_full_turn(&self) -> bool {
        (self.angle.abs() - std::f32::consts::TAU).abs() < 1e-4 && self.movement == 0.0 && self.scaling == 1.0
    }
}

/// The body `polyline` makes when it turns around the Y axis by `lathe`'s
/// angle (rising by its movement, growing to its scaling). A full turn closes
/// on itself, sharing the points on the axis; an open one gets caps on closed
/// profiles. Empty when nothing moves.
#[must_use]
pub fn lathe(polyline: &Polyline, lathe: &Lathe) -> PolyMesh {
    let (angle, movement, scaling) = (lathe.angle, lathe.movement, lathe.scaling.max(0.0));
    if !(angle.is_finite() && movement.is_finite() && scaling.is_finite())
        || (angle == 0.0 && movement == 0.0 && scaling == 1.0)
    {
        return PolyMesh::new();
    }
    let full = lathe.is_full_turn();
    let steps = lathe.subdivision.clamp(Lathe::MIN_SUBDIVISION, Lathe::MAX_SUBDIVISION);
    let stored = if full { steps } else { steps + 1 };
    // Each point as (distance from the axis, height): the half-plane caps are made in.
    let mut runs = profile(polyline, |p| Vec2::new(p.x.hypot(p.z), p.y));
    // Open profiles face away from the area between them and the axis.
    for run in &mut runs {
        if run.closed {
            continue;
        }
        let n = run.flat.len();
        let swept: f32 = (1..n)
            .map(|i| (run.flat[i - 1].x + run.flat[i].x) * (run.flat[i].y - run.flat[i - 1].y))
            .sum();
        if swept < 0.0 {
            run.order.reverse();
        }
    }
    let size = polyline
        .bounds()
        .map_or(1.0, |(min, max)| (max - min).max_element().max(1.0));
    // Points on the axis stay put while nothing but the angle changes them.
    let welded = |q: Vec2| movement == 0.0 && scaling == 1.0 && q.x <= size * 1e-5;
    let span = f32::from(u16::try_from(steps).unwrap_or(u16::MAX));
    let mut points = Vec::new();
    // Point index by run, ring and point.
    let mut ids: Vec<Vec<u32>> = Vec::with_capacity(runs.len());
    for run in &runs {
        let n = run.points.len();
        let mut table = Vec::with_capacity(n * crate::us(stored));
        for k in 0..stored {
            let t = f32::from(u16::try_from(k).unwrap_or(u16::MAX)) / span;
            let turn = glam::Quat::from_rotation_y(angle * t);
            let grow = 1.0 + (scaling - 1.0) * t;
            for (i, &p) in run.points.iter().enumerate() {
                if k > 0 && welded(run.flat[i]) {
                    let first = table[i];
                    table.push(first);
                    continue;
                }
                table.push(u32_index(points.len()));
                points.push(turn * (p * grow) + Vec3::Y * (movement * t));
            }
        }
        ids.push(table);
    }
    let at = |run: usize, ring: u32, local: u32| {
        let n = u32_index(runs[run].points.len());
        ids[run][crate::us((ring % stored) * n + local)]
    };
    // Counter-clockwise profiles turning the positive way face the axis: flip.
    let flip = (angle > 0.0) != lathe.reverse_normals;
    let mut faces: Vec<Vec<u32>> = Vec::new();
    for (r, run) in runs.iter().enumerate() {
        let n = run.order.len();
        let edges = if run.closed { n } else { n - 1 };
        for e in 0..edges {
            let (a, b) = (run.order[e], run.order[(e + 1) % n]);
            for k in 0..steps {
                let mut quad = vec![at(r, k, a), at(r, k, b), at(r, k + 1, b), at(r, k + 1, a)];
                if flip {
                    quad.reverse();
                }
                faces.push(quad);
            }
        }
    }
    if !full {
        // Counter-clockwise in the half-plane faces back against a positive turn.
        let backwards = angle >= 0.0;
        for polygon in caps(&runs, lathe.cap_type) {
            let start: Vec<u32> = polygon.iter().map(|&(run, p)| at(run, 0, p)).collect();
            let end: Vec<u32> = polygon.iter().map(|&(run, p)| at(run, steps, p)).collect();
            let (mut start, mut end) = (start, end);
            if backwards {
                end.reverse();
            } else {
                start.reverse();
            }
            if lathe.start_cap {
                faces.push(start);
            }
            if lathe.end_cap {
                faces.push(end);
            }
        }
    }
    assemble(points, &faces)
}

/// The Sweep generator's options: how a profile travels along a path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sweep {
    /// The profile's size at the end of the path (1.0 = 100 %).
    pub end_scale: f32,
    /// How far the profile has turned about the path at its end (radians).
    pub end_rotation: f32,
    /// Where along the path (0 – 1, by length) the sweep starts and ends.
    pub start_growth: f32,
    pub end_growth: f32,
    /// Moves the profile without turning it.
    pub parallel_movement: bool,
    /// Stretches the profile across corners so walls keep their thickness.
    pub constant_cross_section: bool,
    /// Turns the profile with the path (parallel transport) rather than
    /// keeping its X axis level.
    pub banking: bool,
    /// Turns the shell inside out (caps untouched).
    pub reverse_normals: bool,
    pub start_cap: bool,
    pub end_cap: bool,
    pub cap_type: CapType,
}

impl Default for Sweep {
    fn default() -> Self {
        Self {
            end_scale: 1.0,
            end_rotation: 0.0,
            start_growth: 0.0,
            end_growth: 1.0,
            parallel_movement: false,
            constant_cross_section: true,
            banking: true,
            reverse_normals: false,
            start_cap: true,
            end_cap: true,
            cap_type: CapType::NGons,
        }
    }
}

/// The most a corner stretches the profile (Constant Cross Section).
const MAX_STRETCH: f32 = 5.0;

/// One place along a swept path.
#[derive(Clone, Copy, Debug)]
struct Station {
    position: Vec3,
    /// Fraction of the whole path's length (end scale and rotation).
    t: f32,
    /// The profile's axes here: X, Y and the path's direction.
    x: Vec3,
    y: Vec3,
    /// Corner stretch: the direction across the corner and the factor.
    across: Vec3,
    stretch: f32,
}

/// The surface `profile` (read in its XY plane) makes as it travels along
/// each segment of `path`: rings of the profile turned with the path, a shell
/// between them and caps where the swept path is open. Empty without a profile
/// or a path of length.
#[must_use]
pub fn sweep(profile_line: &Polyline, path: &Polyline, sweep: &Sweep) -> PolyMesh {
    let runs = profile(profile_line, |p| Vec2::new(p.x, p.y));
    if runs.is_empty() {
        return PolyMesh::new();
    }
    let mut points = Vec::new();
    let mut faces: Vec<Vec<u32>> = Vec::new();
    for (rail, closed) in path.runs() {
        let Some((stations, closed)) = stations(rail, closed, sweep) else {
            continue;
        };
        let base = u32_index(points.len());
        let (end_scale, end_rotation) = (sweep.end_scale.max(0.0), sweep.end_rotation);
        // Each station's ring: every run's points in order.
        for station in &stations {
            let (sin, cos) = (end_rotation * station.t).sin_cos();
            let grow = 1.0 + (end_scale - 1.0) * station.t;
            let forward = station.x.cross(station.y);
            for run in &runs {
                for &p in run.points {
                    let flat = Vec2::new(p.x * cos - p.y * sin, p.x * sin + p.y * cos) * grow;
                    let mut offset = station.x * flat.x + station.y * flat.y;
                    offset += station.across * ((station.stretch - 1.0) * offset.dot(station.across));
                    points.push(station.position + offset + forward * (p.z * grow));
                }
            }
        }
        let ring = u32_index(runs.iter().map(|r| r.points.len()).sum());
        let mut first_of = Vec::with_capacity(runs.len());
        let mut start = 0;
        for run in &runs {
            first_of.push(start);
            start += u32_index(run.points.len());
        }
        let count = u32_index(stations.len());
        let at = |station: u32, run: usize, local: u32| base + (station % count) * ring + first_of[run] + local;
        let steps = if closed { count } else { count - 1 };
        for (r, run) in runs.iter().enumerate() {
            let n = run.order.len();
            let edges = if run.closed { n } else { n - 1 };
            for e in 0..edges {
                let (a, b) = (run.order[e], run.order[(e + 1) % n]);
                for k in 0..steps {
                    let mut quad = vec![at(k, r, a), at(k, r, b), at(k + 1, r, b), at(k + 1, r, a)];
                    if sweep.reverse_normals {
                        quad.reverse();
                    }
                    faces.push(quad);
                }
            }
        }
        if !closed {
            // Counter-clockwise in the profile's plane faces along the path.
            for polygon in caps(&runs, sweep.cap_type) {
                if sweep.end_cap {
                    faces.push(polygon.iter().map(|&(run, p)| at(count - 1, run, p)).collect());
                }
                if sweep.start_cap {
                    faces.push(polygon.iter().rev().map(|&(run, p)| at(0, run, p)).collect());
                }
            }
        }
    }
    assemble(points, &faces)
}

/// The stations along one path segment, trimmed to the growth range, and
/// whether they still close; `None` if nothing of it is swept.
fn stations(rail: &[Vec3], closed: bool, sweep: &Sweep) -> Option<(Vec<Station>, bool)> {
    // Without repeated points (a closed segment's end is its start).
    let mut line: Vec<Vec3> = Vec::with_capacity(rail.len() + 1);
    for &p in rail {
        if line.last().is_none_or(|&q: &Vec3| q.distance_squared(p) > 1e-12) {
            line.push(p);
        }
    }
    let mut closed = closed && line.len() >= 3;
    if closed
        && line
            .first()
            .zip(line.last())
            .is_some_and(|(a, b)| a.distance_squared(*b) <= 1e-12)
    {
        line.pop();
    }
    if line.len() < 2 || !line.iter().all(|p| p.is_finite()) {
        return None;
    }
    let (g0, g1) = (sweep.start_growth.clamp(0.0, 1.0), sweep.end_growth.clamp(0.0, 1.0));
    if g1 - g0 <= 1e-6 {
        return None;
    }
    if closed && (g0 > 0.0 || g1 < 1.0) {
        // Cut open: go round once and stop where the growth says.
        line.push(line[0]);
        closed = false;
    }
    let mut lengths = vec![0.0f32];
    let n = line.len();
    let edges = if closed { n } else { n - 1 };
    for i in 0..edges {
        let next = lengths[i] + line[i].distance(line[(i + 1) % n]);
        lengths.push(next);
    }
    let total = lengths[edges];
    if total <= 1e-6 {
        return None;
    }
    // Positions with their fraction of the length, cut to the growth range.
    let mut kept: Vec<(Vec3, f32)> = Vec::with_capacity(n + 2);
    if closed {
        kept.extend(line.iter().zip(&lengths).map(|(&p, &l)| (p, l / total)));
    } else {
        let (from, to) = (g0 * total, g1 * total);
        let at = |length: f32| {
            let i = lengths.partition_point(|&l| l <= length).clamp(1, n - 1);
            let span = (lengths[i] - lengths[i - 1]).max(f32::MIN_POSITIVE);
            line[i - 1].lerp(line[i], ((length - lengths[i - 1]) / span).clamp(0.0, 1.0))
        };
        kept.push((at(from), g0));
        for (i, &l) in lengths.iter().enumerate().take(n) {
            if l > from + 1e-4 && l < to - 1e-4 {
                kept.push((line[i], l / total));
            }
        }
        kept.push((at(to), g1));
    }
    let m = kept.len();
    if m < 2 {
        return None;
    }
    // Directions of the edges, and the path's direction at each station.
    let dir = |i: usize| (kept[(i + 1) % m].0 - kept[i].0).normalize_or_zero();
    let mut stations = Vec::with_capacity(m);
    let mut forwards = Vec::with_capacity(m);
    let mut last_forward = Vec3::Z;
    for (i, &(position, t)) in kept.iter().enumerate() {
        let incoming = if i > 0 {
            dir(i - 1)
        } else if closed {
            dir(m - 1)
        } else {
            Vec3::ZERO
        };
        let outgoing = if i + 1 < m || closed { dir(i) } else { Vec3::ZERO };
        let forward = (incoming + outgoing)
            .try_normalize()
            .or_else(|| outgoing.try_normalize())
            .or_else(|| incoming.try_normalize())
            .unwrap_or(last_forward);
        last_forward = forward;
        forwards.push(forward);
        let (across, stretch) = if sweep.constant_cross_section
            && !sweep.parallel_movement
            && incoming != Vec3::ZERO
            && outgoing != Vec3::ZERO
        {
            let across = (outgoing - incoming).normalize_or_zero();
            (across, 1.0 / forward.dot(incoming).max(1.0 / MAX_STRETCH))
        } else {
            (Vec3::ZERO, 1.0)
        };
        stations.push(Station {
            position,
            t,
            x: Vec3::X,
            y: Vec3::Y,
            across,
            stretch,
        });
    }
    orient(&mut stations, &forwards, closed, sweep);
    Some((stations, closed))
}

/// Sets each station's X and Y axes, given the path's direction there:
/// unturned (Parallel Movement), level (Banking off) or transported along the
/// path (Banking on).
fn orient(stations: &mut [Station], forwards: &[Vec3], closed: bool, sweep: &Sweep) {
    let m = stations.len();
    if sweep.parallel_movement {
        return; // X and Y as they are
    }
    let level = |forward: Vec3, previous: Vec3| Vec3::Y.cross(forward).try_normalize().unwrap_or(previous);
    if !sweep.banking {
        let mut previous = Vec3::X;
        for (station, &forward) in stations.iter_mut().zip(forwards) {
            let x = level(forward, previous);
            previous = x;
            station.x = x;
            station.y = forward.cross(x);
        }
        return;
    }
    // The start: Y along the mean plane's normal, so X lies in that plane.
    let mut normal = crate::mesh::newell(stations.iter().map(|s| s.position));
    let largest = if normal.x.abs() >= normal.y.abs() && normal.x.abs() >= normal.z.abs() {
        normal.x
    } else if normal.y.abs() >= normal.z.abs() {
        normal.y
    } else {
        normal.z
    };
    if largest < 0.0 {
        normal = -normal;
    }
    let first = forwards[0];
    let mut x = (normal - first * normal.dot(first))
        .try_normalize()
        .map_or_else(|| level(first, Vec3::X), |y| y.cross(first));
    for i in 0..m {
        if i > 0 {
            x = glam::Quat::from_rotation_arc(forwards[i - 1], forwards[i]) * x;
        }
        let forward = forwards[i];
        x = (x - forward * x.dot(forward))
            .try_normalize()
            .unwrap_or_else(|| level(forward, Vec3::X));
        stations[i].x = x;
        stations[i].y = forward.cross(x);
    }
    if closed {
        // Back at the start: spread the leftover twist evenly round the loop.
        let back = glam::Quat::from_rotation_arc(forwards[m - 1], forwards[0]) * stations[m - 1].x;
        let (start, axis) = (stations[0].x, forwards[0]);
        let twist = back.cross(start).dot(axis).atan2(back.dot(start));
        let steps = f32::from(u16::try_from(m).unwrap_or(u16::MAX));
        for (i, station) in stations.iter_mut().enumerate() {
            let k = f32::from(u16::try_from(i).unwrap_or(u16::MAX));
            let forward = forwards[i];
            let turn = glam::Quat::from_axis_angle(forward, twist * k / steps);
            station.x = turn * station.x;
            station.y = forward.cross(station.x);
        }
    }
}

/// The Loft generator's options: how a skin is stretched over sections.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Loft {
    /// Points around each section (closed sections: one fewer).
    pub subdivision_u: u32,
    /// Rings along the skin (at least one per section).
    pub subdivision_v: u32,
    /// A looser surface that approximates the inner sections.
    pub organic: bool,
    /// Rings spread evenly per section pair rather than by distance.
    pub per_segment: bool,
    /// Joins the last section to the first.
    pub looped: bool,
    /// Straight lines between sections.
    pub linear: bool,
    /// Turns the skin inside out (caps untouched).
    pub reverse_normals: bool,
    pub start_cap: bool,
    pub end_cap: bool,
    pub cap_type: CapType,
}

impl Default for Loft {
    fn default() -> Self {
        Self {
            subdivision_u: 30,
            subdivision_v: 10,
            organic: false,
            per_segment: false,
            looped: false,
            linear: false,
            reverse_normals: false,
            start_cap: true,
            end_cap: true,
            cap_type: CapType::NGons,
        }
    }
}

impl Loft {
    pub const MIN_SUBDIVISION: u32 = 2;
    pub const MAX_SUBDIVISION: u32 = 4000;
}

/// The skin over `sections` (their first segments, in order): each resampled
/// to the same number of points, closed ones turned the same way round,
/// joined by curves (or lines) through them, with caps on closed ends. Empty
/// without a section of two points.
#[must_use]
pub fn loft(sections: &[Polyline], loft: &Loft) -> PolyMesh {
    let raw: Vec<(&[Vec3], bool)> = sections
        .iter()
        .filter_map(|section| section.runs().next())
        .filter(|(points, _)| points.len() >= 2 && points.iter().all(|p| p.is_finite()))
        .collect();
    let Some(&(_, first_closed)) = raw.first() else {
        return PolyMesh::new();
    };
    let closed = first_closed && raw[0].0.len() >= 3;
    let u = loft.subdivision_u.clamp(Loft::MIN_SUBDIVISION, Loft::MAX_SUBDIVISION);
    let count = if closed { (u - 1).max(3) } else { u };
    let mut rings: Vec<Vec<Vec3>> = raw
        .iter()
        .map(|(points, _)| resample(points, closed, crate::us(count)))
        .collect();
    let n = rings.len();
    let looped = loft.looped && n >= 3;
    let centre = |ring: &[Vec3]| ring.iter().copied().sum::<Vec3>() / f32::from(u16::try_from(ring.len()).unwrap_or(1));
    let centres: Vec<Vec3> = rings.iter().map(|r| centre(r)).collect();
    // The direction the skin runs at each section (by its neighbours).
    let along = |i: usize| -> Vec3 {
        let (before, after) = if looped {
            ((i + n - 1) % n, (i + 1) % n)
        } else {
            (i.saturating_sub(1), (i + 1).min(n - 1))
        };
        centres[after] - centres[before]
    };
    if closed {
        // Counter-clockwise around the skin's direction, first point kept.
        let mut previous: Option<Vec3> = None;
        for (i, ring) in rings.iter_mut().enumerate() {
            let normal = crate::mesh::newell(ring.iter().copied());
            let reference = Some(along(i)).filter(|a| a.length_squared() > 1e-8).or(previous);
            if reference.is_some_and(|a| normal.dot(a) < 0.0) {
                ring[1..].reverse();
            }
            previous = Some(crate::mesh::newell(ring.iter().copied()));
        }
    }
    if n == 1 {
        // One section: just its cap.
        if !closed {
            return PolyMesh::new();
        }
        let ring = &rings[0];
        let axis = crate::mesh::newell(ring.iter().copied());
        let faces = cap_faces(ring, axis, 0, loft.cap_type);
        return assemble(ring.clone(), &faces);
    }

    // Where along the sections each ring of the skin lies: (pair, t).
    let pairs = if looped { n } else { n - 1 };
    let v = loft.subdivision_v.clamp(Loft::MIN_SUBDIVISION, Loft::MAX_SUBDIVISION);
    let total = crate::us(if looped { v } else { v - 1 }).max(pairs);
    let steps: Vec<usize> = if loft.per_segment {
        (0..pairs)
            .map(|i| total / pairs + usize::from(i < total % pairs))
            .collect()
    } else {
        let gaps: Vec<f32> = (0..pairs)
            .map(|i| centres[i].distance(centres[(i + 1) % n]) + 1e-6)
            .collect();
        let sum: f32 = gaps.iter().sum();
        let share = f32::from(u16::try_from(total).unwrap_or(u16::MAX));
        gaps.iter()
            .map(|&g| (share * g / sum).round().clamp(1.0, 4000.0) as usize)
            .collect()
    };
    let mut stations: Vec<(usize, f32)> = Vec::new();
    for (pair, &k) in steps.iter().enumerate() {
        let span = f32::from(u16::try_from(k).unwrap_or(u16::MAX));
        stations.extend((0..k).map(|s| (pair, f32::from(u16::try_from(s).unwrap_or(0)) / span)));
    }
    if !looped {
        stations.push((n - 2, 1.0));
    }

    let ring_len = rings[0].len();
    let section = |i: isize, j: usize| -> Vec3 {
        let last = isize::try_from(n).unwrap_or(isize::MAX) - 1;
        if looped {
            return rings[crate::us(u32::try_from(i.rem_euclid(last + 1)).unwrap_or(0))][j];
        }
        // Beyond the ends: mirrored, so curves end on the first and last section.
        let get = |k: isize| rings[crate::us(u32::try_from(k.clamp(0, last)).unwrap_or(0))][j];
        if i < 0 {
            get(0) * 2.0 - get(1)
        } else if i > last {
            get(last) * 2.0 - get(last - 1)
        } else {
            get(i)
        }
    };
    let mut points = Vec::with_capacity(stations.len() * ring_len);
    for &(pair, t) in &stations {
        let i = isize::try_from(pair).unwrap_or(0);
        for j in 0..ring_len {
            let (p0, p1, p2, p3) = (section(i - 1, j), section(i, j), section(i + 1, j), section(i + 2, j));
            points.push(if loft.linear {
                p1.lerp(p2, t)
            } else if loft.organic {
                b_spline(p0, p1, p2, p3, t)
            } else {
                catmull_rom(p0, p1, p2, p3, t)
            });
        }
    }

    let rows = u32_index(stations.len());
    let width = u32_index(ring_len);
    let at = |row: u32, j: u32| (row % rows) * width + (j % width);
    let mut faces: Vec<Vec<u32>> = Vec::new();
    let row_steps = if looped { rows } else { rows - 1 };
    let col_steps = if closed { width } else { width - 1 };
    for k in 0..row_steps {
        for j in 0..col_steps {
            let mut quad = vec![at(k, j), at(k, j + 1), at(k + 1, j + 1), at(k + 1, j)];
            if loft.reverse_normals {
                quad.reverse();
            }
            faces.push(quad);
        }
    }
    if closed && !looped {
        let start = &points[..ring_len];
        let end_row = crate::us(rows - 1) * ring_len;
        let end = &points[end_row..end_row + ring_len];
        if loft.start_cap {
            faces.extend(cap_faces(start, -along(0), 0, loft.cap_type));
        }
        if loft.end_cap {
            faces.extend(cap_faces(end, along(n - 1), u32_index(end_row), loft.cap_type));
        }
    }
    assemble(points, &faces)
}

/// The cap faces of the closed `ring` (point indices from `base`), facing
/// along `facing`.
fn cap_faces(ring: &[Vec3], facing: Vec3, base: u32, cap_type: CapType) -> Vec<Vec<u32>> {
    let Some(normal) = facing.try_normalize() else {
        return Vec::new();
    };
    let u = normal.any_orthonormal_vector();
    let v = normal.cross(u);
    let line = Polyline {
        points: ring.to_vec(),
        segments: vec![crate::spline::Segment {
            count: u32_index(ring.len()),
            closed: true,
        }],
    };
    let runs = profile(&line, |p| Vec2::new(p.dot(u), p.dot(v)));
    // Counter-clockwise in (u, v) faces along `facing`.
    caps(&runs, cap_type)
        .into_iter()
        .map(|polygon| polygon.into_iter().map(|(_, p)| base + p).collect())
        .collect()
}

/// `count` points along `points` spaced evenly by length, from its first point
/// (closed: round once, the start not repeated).
fn resample(points: &[Vec3], closed: bool, count: usize) -> Vec<Vec3> {
    let n = points.len();
    let edges = if closed { n } else { n - 1 };
    let mut lengths = Vec::with_capacity(edges + 1);
    lengths.push(0.0f32);
    for i in 0..edges {
        let next = lengths[i] + points[i].distance(points[(i + 1) % n]);
        lengths.push(next);
    }
    let total = lengths[edges];
    if total <= 1e-6 {
        return vec![points[0]; count];
    }
    let divisions = f32::from(u16::try_from(if closed { count } else { count - 1 }).unwrap_or(u16::MAX));
    (0..count)
        .map(|k| {
            let length = total * f32::from(u16::try_from(k).unwrap_or(u16::MAX)) / divisions;
            let i = lengths.partition_point(|&l| l <= length).clamp(1, edges);
            let span = (lengths[i] - lengths[i - 1]).max(f32::MIN_POSITIVE);
            points[i - 1].lerp(points[i % n], ((length - lengths[i - 1]) / span).clamp(0.0, 1.0))
        })
        .collect()
}

/// The Catmull-Rom curve through `p1` (t = 0) and `p2` (t = 1).
fn catmull_rom(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, t: f32) -> Vec3 {
    let (t2, t3) = (t * t, t * t * t);
    ((p1 * 2.0) + (p2 - p0) * t + (p0 * 2.0 - p1 * 5.0 + p2 * 4.0 - p3) * t2 + (p1 * 3.0 - p0 - p2 * 3.0 + p3) * t3)
        * 0.5
}

/// The uniform cubic B-spline span of the four control points.
fn b_spline(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, t: f32) -> Vec3 {
    let (t2, t3) = (t * t, t * t * t);
    let b0 = (1.0 - t).powi(3);
    let b1 = 3.0 * t3 - 6.0 * t2 + 4.0;
    let b2 = -3.0 * t3 + 3.0 * t2 + 3.0 * t + 1.0;
    (p0 * b0 + p1 * b1 + p2 * b2 + p3 * t3) / 6.0
}

/// Twice the signed area: positive when counter-clockwise.
fn area(polygon: &[Vec2]) -> f32 {
    let n = polygon.len();
    (0..n).map(|i| polygon[i].perp_dot(polygon[(i + 1) % n])).sum()
}

/// Even-odd point-in-polygon test.
fn contains(polygon: &[Vec2], p: Vec2) -> bool {
    let n = polygon.len();
    let mut inside = false;
    for i in 0..n {
        let (a, b) = (polygon[i], polygon[(i + n - 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spline::Segment;

    fn polyline(runs: &[(&[Vec3], bool)]) -> Polyline {
        Polyline {
            points: runs.iter().flat_map(|(p, _)| p.iter().copied()).collect(),
            segments: runs
                .iter()
                .map(|&(p, closed)| Segment {
                    count: u32_index(p.len()),
                    closed,
                })
                .collect(),
        }
    }

    /// A square of side `size` around the origin in the XY plane, counter-clockwise
    /// seen from +Z (or clockwise).
    fn square(size: f32, counter_clockwise: bool) -> Vec<Vec3> {
        let h = size * 0.5;
        let mut points = vec![
            Vec3::new(-h, -h, 0.0),
            Vec3::new(h, -h, 0.0),
            Vec3::new(h, h, 0.0),
            Vec3::new(-h, h, 0.0),
        ];
        if !counter_clockwise {
            points.reverse();
        }
        points
    }

    fn circle(radius: f32, n: u32) -> Vec<Vec3> {
        (0..n)
            .map(|i| {
                let a =
                    std::f32::consts::TAU * f32::from(u16::try_from(i).unwrap()) / f32::from(u16::try_from(n).unwrap());
                Vec3::new(radius * a.cos(), radius * a.sin(), 0.0)
            })
            .collect()
    }

    /// The enclosed volume (divergence theorem over the triangulated faces):
    /// positive when every face points outwards.
    fn volume(mesh: &PolyMesh) -> f32 {
        let mut triangles = Vec::new();
        let mut triangulator = crate::Triangulator::new();
        for f in 0..mesh.face_count() {
            triangulator.face_triangles(mesh, f, &mut triangles);
        }
        triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| mesh.points()[crate::us(i)]);
                a.dot(b.cross(c)) / 6.0
            })
            .sum()
    }

    /// Every edge used by exactly two faces, once each way.
    fn is_closed(mesh: &PolyMesh) -> bool {
        let mut edges = std::collections::HashMap::new();
        for face in mesh.faces() {
            for i in 0..face.len() {
                *edges.entry((face[i], face[(i + 1) % face.len()])).or_insert(0) += 1;
            }
        }
        edges
            .iter()
            .all(|(&(a, b), &count)| count == 1 && edges.get(&(b, a)) == Some(&1))
    }

    #[test]
    fn a_circle_becomes_a_closed_cylinder_facing_outwards() {
        for counter_clockwise in [true, false] {
            let mut points = circle(100.0, 32);
            if !counter_clockwise {
                points.reverse();
            }
            let extrusion = Extrusion {
                subdivision: 3,
                ..Extrusion::default()
            };
            let mesh = extrude(&polyline(&[(&points, true)]), &extrusion);
            assert_eq!(mesh.point_count(), 32 * 4, "four rings");
            assert_eq!(mesh.face_count(), 32 * 3 + 2, "shell quads and two n-gon caps");
            assert!(is_closed(&mesh));
            let expected = 32.0 * 0.5 * 100.0 * 100.0 * (std::f32::consts::TAU / 32.0).sin() * 20.0;
            assert!(
                (volume(&mesh) - expected).abs() < 1.0,
                "{} vs {expected}",
                volume(&mesh)
            );
            let (min, max) = mesh.bounds().unwrap();
            assert_eq!((min.z, max.z), (0.0, 20.0), "along +Z for a spline in XY");
        }
    }

    #[test]
    fn holes_are_cut_out_of_the_caps_whichever_way_they_run() {
        for (outer, inner) in [(true, true), (true, false), (false, true), (false, false)] {
            let (a, b) = (square(200.0, outer), square(100.0, inner));
            let mesh = extrude(&polyline(&[(&a, true), (&b, true)]), &Extrusion::default());
            assert!(is_closed(&mesh), "{outer} {inner}");
            let expected = (200.0 * 200.0 - 100.0 * 100.0) * 20.0;
            assert!(
                (volume(&mesh) - expected).abs() < 1.0,
                "{outer} {inner}: {}",
                volume(&mesh)
            );
            // The cap is triangles: 8 corners → 8 triangles per cap.
            let triangles = mesh.faces().filter(|f| f.len() == 3).count();
            assert_eq!(triangles, 16);
        }
    }

    #[test]
    fn islands_inside_holes_get_their_own_caps() {
        let (a, b, c) = (square(300.0, true), square(200.0, true), square(100.0, true));
        let mesh = extrude(&polyline(&[(&a, true), (&b, true), (&c, true)]), &Extrusion::default());
        assert!(is_closed(&mesh));
        let expected = (300.0f32 * 300.0 - 200.0 * 200.0 + 100.0 * 100.0) * 20.0;
        assert!((volume(&mesh) - expected).abs() < 1.0, "{}", volume(&mesh));
    }

    #[test]
    fn directions() {
        let xy = polyline(&[(&square(100.0, true), true)]);
        let zy = Polyline {
            points: xy.points.iter().map(|p| Vec3::new(0.0, p.y, p.x)).collect(),
            ..xy.clone()
        };
        let xz = Polyline {
            points: xy.points.iter().map(|p| Vec3::new(p.x, 0.0, p.y)).collect(),
            ..xy.clone()
        };
        let auto = Extrusion::default();
        assert_eq!(auto.vector(&xy), Vec3::new(0.0, 0.0, 20.0));
        assert_eq!(auto.vector(&zy), Vec3::new(20.0, 0.0, 0.0));
        assert_eq!(auto.vector(&xz), Vec3::new(0.0, 20.0, 0.0));
        let with = |direction, offset| Extrusion {
            direction,
            offset,
            movement: Vec3::new(3.0, 4.0, 0.0),
            ..Extrusion::default()
        };
        assert_eq!(with(ExtrudeDirection::Y, -5.0).vector(&xy), Vec3::new(0.0, -5.0, 0.0));
        assert!(
            with(ExtrudeDirection::Custom, 10.0)
                .vector(&xy)
                .abs_diff_eq(Vec3::new(6.0, 8.0, 0.0), 1e-5)
        );
        assert_eq!(
            with(ExtrudeDirection::Absolute, 99.0).vector(&xy),
            Vec3::new(3.0, 4.0, 0.0)
        );
        // A negative offset goes the other way and still faces outwards.
        let back = extrude(&xy, &with(ExtrudeDirection::Auto, -20.0));
        assert!(volume(&back) > 0.0);
        assert_eq!(back.bounds().unwrap().0.z, -20.0);
        // Zero movement: nothing.
        assert_eq!(extrude(&xy, &with(ExtrudeDirection::Auto, 0.0)).face_count(), 0);
    }

    #[test]
    fn open_splines_get_a_shell_that_reverse_normals_flips() {
        let line = [Vec3::ZERO, Vec3::new(100.0, 0.0, 0.0), Vec3::new(100.0, 100.0, 0.0)];
        let open = polyline(&[(&line, false)]);
        let mesh = extrude(&open, &Extrusion::default());
        assert_eq!(mesh.face_count(), 2, "two quads, no caps");
        let flipped = extrude(
            &open,
            &Extrusion {
                reverse_normals: true,
                ..Extrusion::default()
            },
        );
        assert!(mesh.face_normal(0).dot(flipped.face_normal(0)) < -0.99);
    }

    #[test]
    fn cap_options() {
        let square = polyline(&[(&square(100.0, true), true)]);
        let triangles = extrude(
            &square,
            &Extrusion {
                cap_type: CapType::Triangles,
                ..Extrusion::default()
            },
        );
        assert_eq!(triangles.faces().filter(|f| f.len() == 3).count(), 4);
        assert!(is_closed(&triangles));
        let open = extrude(
            &square,
            &Extrusion {
                start_cap: false,
                end_cap: false,
                ..Extrusion::default()
            },
        );
        assert_eq!(open.face_count(), 4, "the shell alone");
        let end_only = extrude(
            &square,
            &Extrusion {
                start_cap: false,
                ..Extrusion::default()
            },
        );
        assert_eq!(end_only.face_count(), 5);
        assert!(
            end_only.face_normal(4).abs_diff_eq(Vec3::Z, 1e-5),
            "the end cap faces forward"
        );
    }

    #[test]
    fn outlines_with_holes_triangulate_without_overlap() {
        let outer = [
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 10.0),
            Vec2::new(0.0, 10.0),
        ];
        let hole_a: Vec<Vec2> = [(2.0, 2.0), (4.0, 2.0), (4.0, 4.0), (2.0, 4.0)]
            .map(|(x, y)| Vec2::new(x, y))
            .to_vec();
        let hole_b: Vec<Vec2> = [(6.0, 6.0), (8.0, 6.0), (8.0, 8.0), (6.0, 8.0)]
            .map(|(x, y)| Vec2::new(x, y))
            .to_vec();
        let triangles = triangulate_outlines(&outer, &[&hole_a, &hole_b]);
        let all: Vec<Vec2> = outer.iter().chain(&hole_a).chain(&hole_b).copied().collect();
        let mut total = 0.0;
        for t in &triangles {
            let [a, b, c] = t.map(|i| all[crate::us(i)]);
            let doubled = (b - a).perp_dot(c - a);
            assert!(doubled >= 0.0, "counter-clockwise");
            total += doubled * 0.5;
        }
        assert!((total - (100.0 - 4.0 - 4.0)).abs() < 1e-3, "{total}");
    }

    /// A rectangle beside the Y axis in the XY plane: x 50–100, y 0–80.
    fn beside_axis(counter_clockwise: bool) -> Vec<Vec3> {
        let mut points = vec![
            Vec3::new(50.0, 0.0, 0.0),
            Vec3::new(100.0, 0.0, 0.0),
            Vec3::new(100.0, 80.0, 0.0),
            Vec3::new(50.0, 80.0, 0.0),
        ];
        if !counter_clockwise {
            points.reverse();
        }
        points
    }

    #[test]
    fn a_full_turn_of_a_rectangle_is_a_closed_pipe_facing_outwards() {
        for counter_clockwise in [true, false] {
            for angle in [std::f32::consts::TAU, -std::f32::consts::TAU] {
                let rectangle = polyline(&[(&beside_axis(counter_clockwise), true)]);
                let mesh = lathe(
                    &rectangle,
                    &Lathe {
                        angle,
                        ..Lathe::default()
                    },
                );
                assert_eq!(mesh.point_count(), 24 * 4, "no ring repeated");
                assert_eq!(mesh.face_count(), 24 * 4, "no caps on a full turn");
                assert!(is_closed(&mesh));
                // A 24-sided prism ring: outer minus inner polygon area times height.
                let polygon = |r: f32| 12.0 * r * r * (std::f32::consts::TAU / 24.0).sin();
                let expected = (polygon(100.0) - polygon(50.0)) * 80.0;
                assert!(
                    (volume(&mesh) - expected).abs() < expected * 1e-4,
                    "{counter_clockwise} {angle}: {} vs {expected}",
                    volume(&mesh)
                );
            }
        }
    }

    #[test]
    fn an_open_profile_from_the_axis_makes_a_cup_with_a_shared_centre() {
        let cup = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(60.0, 0.0, 0.0),
            Vec3::new(80.0, 100.0, 0.0),
        ];
        for points in [cup.to_vec(), cup.iter().rev().copied().collect()] {
            let mesh = lathe(&polyline(&[(&points, false)]), &Lathe::default());
            assert_eq!(mesh.point_count(), 1 + 24 * 2, "one centre point");
            let triangles = mesh.faces().filter(|f| f.len() == 3).count();
            assert_eq!(triangles, 24, "the bottom fans round the centre");
            let bottom = mesh.faces().position(|f| f.len() == 3).unwrap();
            assert!(mesh.face_normal(bottom).y < -0.99, "the bottom faces down");
            let wall = mesh.faces().position(|f| f.len() == 4).unwrap();
            let centre = mesh.face_center(wall);
            assert!(
                mesh.face_normal(wall).dot(Vec3::new(centre.x, 0.0, centre.z)) > 0.0,
                "the wall faces out"
            );
        }
    }

    #[test]
    fn half_a_turn_gets_caps_and_reverse_normals_flips_only_the_shell() {
        let rectangle = polyline(&[(&beside_axis(true), true)]);
        let half = Lathe {
            angle: std::f32::consts::PI,
            ..Lathe::default()
        };
        let mesh = lathe(&rectangle, &half);
        assert_eq!(mesh.point_count(), 25 * 4);
        assert_eq!(mesh.face_count(), 24 * 4 + 2);
        assert!(is_closed(&mesh));
        assert!(volume(&mesh) > 0.0);
        let reversed = lathe(
            &rectangle,
            &Lathe {
                reverse_normals: true,
                ..half
            },
        );
        assert!(
            mesh.face_normal(0).dot(reversed.face_normal(0)) < -0.99,
            "shell flipped"
        );
        let last = mesh.face_count() - 1;
        assert_eq!(mesh.face_normal(last), reversed.face_normal(last), "caps kept");
        let open = lathe(
            &rectangle,
            &Lathe {
                start_cap: false,
                end_cap: false,
                ..half
            },
        );
        assert_eq!(open.face_count(), 24 * 4);
    }

    #[test]
    fn movement_and_scaling_change_the_end() {
        let rectangle = polyline(&[(&beside_axis(true), true)]);
        let screw = lathe(
            &rectangle,
            &Lathe {
                angle: std::f32::consts::TAU * 2.0,
                movement: 200.0,
                ..Lathe::default()
            },
        );
        let (min, max) = screw.bounds().unwrap();
        assert!((min.y - 0.0).abs() < 1e-3 && (max.y - 280.0).abs() < 1e-3);
        assert!(is_closed(&screw), "capped at both ends");
        assert!(volume(&screw) > 0.0);
        let shrink = lathe(
            &rectangle,
            &Lathe {
                angle: std::f32::consts::PI,
                scaling: 0.5,
                ..Lathe::default()
            },
        );
        // The end ring is the profile at half size, turned half way: x = -25 … -50.
        let end = &shrink.points()[24 * 4..];
        assert!(
            end.iter().all(|p| p.x <= -24.9 && p.x >= -50.1 && p.y <= 40.1),
            "{end:?}"
        );
        assert_eq!(
            lathe(
                &rectangle,
                &Lathe {
                    angle: 0.0,
                    ..Lathe::default()
                }
            )
            .face_count(),
            0
        );
    }

    fn closed_circle(radius: f32, n: u32) -> Polyline {
        polyline(&[(&circle(radius, n), true)])
    }

    fn straight(length: f32) -> Polyline {
        polyline(&[(&[Vec3::ZERO, Vec3::new(0.0, 0.0, length)], false)])
    }

    #[test]
    fn a_circle_along_a_line_is_a_capped_cylinder_facing_outwards() {
        for counter_clockwise in [true, false] {
            let mut points = circle(10.0, 16);
            if !counter_clockwise {
                points.reverse();
            }
            let mesh = sweep(&polyline(&[(&points, true)]), &straight(100.0), &Sweep::default());
            assert_eq!(mesh.point_count(), 32);
            assert_eq!(mesh.face_count(), 16 + 2);
            assert!(is_closed(&mesh));
            let expected = 8.0 * 100.0 * (std::f32::consts::TAU / 16.0).sin() * 100.0;
            assert!(
                (volume(&mesh) - expected).abs() < 1.0,
                "{} vs {expected}",
                volume(&mesh)
            );
            let (min, max) = mesh.bounds().unwrap();
            assert!((min.z, max.z) == (0.0, 100.0) && (max.x - 10.0).abs() < 1e-4);
        }
    }

    #[test]
    fn along_a_closed_path_it_is_a_closed_tube_without_caps() {
        let tube = sweep(&closed_circle(10.0, 12), &closed_circle(100.0, 48), &Sweep::default());
        assert_eq!(tube.point_count(), 12 * 48, "no ring repeated");
        assert_eq!(tube.face_count(), 12 * 48);
        assert!(is_closed(&tube));
        assert!(volume(&tube) > 0.0);
        // Banking ends where it started: the seam's rings line up.
        // The seam (last ring to the first) looks like any other step.
        let ring = |k: usize| &tube.points()[k * 12..k * 12 + 12];
        for i in 0..12 {
            let seam = ring(47)[i].distance(ring(0)[i]);
            let step = ring(0)[i].distance(ring(1)[i]);
            assert!((seam - step).abs() < step * 0.05, "point {i}: {seam} vs {step}");
        }
    }

    #[test]
    fn banking_off_keeps_x_level_and_parallel_movement_keeps_it_unturned() {
        let rising = polyline(&[(
            &[Vec3::ZERO, Vec3::new(100.0, 100.0, 0.0), Vec3::new(200.0, 100.0, 100.0)],
            false,
        )]);
        let bar = polyline(&[(&[Vec3::new(-10.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 0.0)], false)]);
        let level = sweep(
            &bar,
            &rising,
            &Sweep {
                banking: false,
                constant_cross_section: false,
                ..Sweep::default()
            },
        );
        for pair in level.points().chunks(2) {
            assert!((pair[0].y - pair[1].y).abs() < 1e-3, "level: {pair:?}");
        }
        let unturned = sweep(
            &bar,
            &rising,
            &Sweep {
                parallel_movement: true,
                ..Sweep::default()
            },
        );
        for pair in unturned.points().chunks(2) {
            assert!(
                (pair[1] - pair[0]).abs_diff_eq(Vec3::new(20.0, 0.0, 0.0), 1e-3),
                "{pair:?}"
            );
        }
    }

    #[test]
    fn corners_keep_their_thickness() {
        let square = polyline(&[(&square(200.0, true), true)]);
        let bar = polyline(&[(&[Vec3::new(-10.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 0.0)], false)]);
        let width_at_corner = |constant| {
            let mesh = sweep(
                &bar,
                &square,
                &Sweep {
                    constant_cross_section: constant,
                    ..Sweep::default()
                },
            );
            mesh.points()[0].distance(mesh.points()[1])
        };
        assert!((width_at_corner(true) - 20.0 * std::f32::consts::SQRT_2).abs() < 1e-3);
        assert!((width_at_corner(false) - 20.0).abs() < 1e-3);
    }

    #[test]
    fn end_scale_rotation_and_growth() {
        let bar = polyline(&[(&[Vec3::new(-10.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 0.0)], false)]);
        let path = straight(100.0);
        let scaled = sweep(
            &bar,
            &path,
            &Sweep {
                end_scale: 0.5,
                end_rotation: std::f32::consts::FRAC_PI_2,
                ..Sweep::default()
            },
        );
        let end = &scaled.points()[2..4];
        assert!((end[0].distance(end[1]) - 10.0).abs() < 1e-3, "half size");
        assert!(
            (end[0].x).abs() < 1e-3 && (end[0].y.abs() - 5.0).abs() < 1e-3,
            "turned: {end:?}"
        );
        let half = sweep(
            &closed_circle(10.0, 8),
            &path,
            &Sweep {
                start_growth: 0.25,
                end_growth: 0.75,
                ..Sweep::default()
            },
        );
        let (min, max) = half.bounds().unwrap();
        assert!((min.z - 25.0).abs() < 1e-3 && (max.z - 75.0).abs() < 1e-3);
        assert!(is_closed(&half), "capped");
        let cut_loop = sweep(
            &closed_circle(10.0, 8),
            &closed_circle(100.0, 32),
            &Sweep {
                end_growth: 0.5,
                ..Sweep::default()
            },
        );
        assert!(is_closed(&cut_loop), "a cut loop gets caps");
        let none = Sweep {
            start_growth: 0.6,
            end_growth: 0.4,
            ..Sweep::default()
        };
        assert_eq!(sweep(&bar, &path, &none).face_count(), 0);
    }

    #[test]
    fn banking_on_a_twisting_closed_path_spreads_the_twist() {
        // A saddle-shaped loop: transporting a frame round it leaves a twist.
        let loop_points: Vec<Vec3> = (0..64u16)
            .map(|i| {
                let a = std::f32::consts::TAU * f32::from(i) / 64.0;
                Vec3::new(
                    100.0 * a.cos(),
                    60.0 * (2.0 * a).sin(),
                    100.0 * a.sin() + 40.0 * (3.0 * a).cos(),
                )
            })
            .collect();
        let bar = polyline(&[(&[Vec3::new(-10.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 0.0)], false)]);
        let ribbon = sweep(&bar, &polyline(&[(&loop_points, true)]), &Sweep::default());
        let points = ribbon.points();
        let width = |k: usize| points[2 * k].distance(points[2 * k + 1]);
        let turn = |a: usize, b: usize| {
            let (u, v) = (points[2 * a + 1] - points[2 * a], points[2 * b + 1] - points[2 * b]);
            u.normalize().dot(v.normalize())
        };
        assert!(turn(63, 0) > 0.95, "no jump at the seam: {}", turn(63, 0));
        assert!((1..63).all(|k| turn(k - 1, k) > 0.9));
        assert!((0..64).all(|k| width(k) >= 19.99));
    }

    fn lifted(points: Vec<Vec3>, z: f32) -> Vec<Vec3> {
        points.into_iter().map(|p| p + Vec3::Z * z).collect()
    }

    #[test]
    fn two_circles_make_a_capped_frustum_facing_outwards() {
        for (a, b) in [(true, true), (true, false), (false, true)] {
            let mut bottom = circle(100.0, 64);
            let mut top = lifted(circle(50.0, 64), 200.0);
            if !a {
                bottom[1..].reverse();
            }
            if !b {
                top[1..].reverse();
            }
            let sections = [polyline(&[(&bottom, true)]), polyline(&[(&top, true)])];
            let mesh = loft(&sections, &Loft::default());
            assert_eq!(mesh.point_count(), 29 * 10, "U − 1 around, V rings");
            assert!(is_closed(&mesh), "{a} {b}");
            // A frustum: π h (R² + R r + r²) / 3, polygonal.
            let expected = std::f32::consts::PI * 200.0 * (100.0f32 * 100.0 + 100.0 * 50.0 + 50.0 * 50.0) / 3.0;
            let got = volume(&mesh);
            assert!((got - expected).abs() < expected * 0.03, "{a} {b}: {got} vs {expected}");
        }
    }

    #[test]
    fn the_skin_passes_through_every_section_unless_organic() {
        let sections: Vec<Polyline> = [(100.0, 0.0), (30.0, 100.0), (100.0, 200.0)]
            .iter()
            .map(|&(r, z)| polyline(&[(&lifted(circle(r, 32), z), true)]))
            .collect();
        let radius_at_middle = |mesh: &PolyMesh| {
            mesh.points()
                .iter()
                .filter(|p| (p.z - 100.0).abs() < 1e-3)
                .map(|p| p.truncate().length())
                .fold(f32::MAX, f32::min)
        };
        let options = Loft {
            subdivision_v: 9,
            per_segment: true,
            ..Loft::default()
        };
        let through = loft(&sections, &options);
        assert!(
            (radius_at_middle(&through) - 30.0).abs() < 0.5,
            "{}",
            radius_at_middle(&through)
        );
        let organic = loft(
            &sections,
            &Loft {
                organic: true,
                ..options
            },
        );
        assert!(radius_at_middle(&organic) > 40.0, "{}", radius_at_middle(&organic));
        let (min, max) = organic.bounds().unwrap();
        assert!(
            min.z.abs() < 1e-3 && (max.z - 200.0).abs() < 1e-3,
            "ends on the first and last"
        );
        // Linear: halfway between two sections is halfway in radius.
        let linear = loft(
            &sections[..2],
            &Loft {
                linear: true,
                subdivision_v: 3,
                ..Loft::default()
            },
        );
        let middle: Vec<f32> = linear
            .points()
            .iter()
            .filter(|p| (p.z - 50.0).abs() < 1e-3)
            .map(|p| p.truncate().length())
            .collect();
        assert!(
            !middle.is_empty() && middle.iter().all(|r| (r - 65.0).abs() < 1.0),
            "{middle:?}"
        );
    }

    #[test]
    fn counts_loop_and_a_single_section() {
        let sections: Vec<Polyline> = (0..4u16)
            .map(|i| {
                // Four circles round the Y axis, like slices of a torus.
                let a = std::f32::consts::FRAC_PI_2 * f32::from(i);
                let turn = glam::Quat::from_rotation_y(a);
                let points = circle(20.0, 16)
                    .into_iter()
                    .map(|p| turn * (Vec3::new(p.x + 100.0, p.y, 0.0)))
                    .collect::<Vec<_>>();
                polyline(&[(&points, true)])
            })
            .collect();
        let ring = loft(
            &sections,
            &Loft {
                looped: true,
                subdivision_u: 17,
                subdivision_v: 8,
                per_segment: true,
                ..Loft::default()
            },
        );
        assert_eq!(ring.point_count(), 16 * 8);
        assert_eq!(ring.face_count(), 16 * 8, "closed round, no caps");
        assert!(is_closed(&ring));
        let open = [
            polyline(&[(&[Vec3::ZERO, Vec3::X * 100.0], false)]),
            polyline(&[(&[Vec3::Z * 50.0, Vec3::new(100.0, 0.0, 50.0)], false)]),
        ];
        let sheet = loft(
            &open,
            &Loft {
                subdivision_u: 5,
                subdivision_v: 4,
                ..Loft::default()
            },
        );
        assert_eq!((sheet.point_count(), sheet.face_count()), (5 * 4, 4 * 3));
        let lid = loft(&[closed_circle(50.0, 12)], &Loft::default());
        assert_eq!(lid.face_count(), 1, "one section: a cap");
        assert_eq!(loft(&[], &Loft::default()).face_count(), 0);
    }
}
