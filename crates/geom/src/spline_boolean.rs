//! The Spline Boolean: closed outlines combined as flat shapes — united, cut,
//! intersected or exclusive — giving closed outlines again (spec
//! `docs/reference/generator-spline-boolean.md`).
//!
//! The method: every outline's edges are cut where they cross or touch
//! (crossings decided by exact orientation tests, the same predicates as the
//! mesh Boolean), points closer than [`MERGE_DISTANCE`] become one and equal
//! edges one edge, owned by every shape that has it an odd number of times.
//! Each edge is then classified by a ray from its middle: the shapes whose
//! edges the ray crosses an odd number of times hold the region on that side
//! (each shape by even-odd, so a letter's hole is a hole); the other side
//! differs by the edge's own shapes. Where the mode's rule holds on one side
//! only, the edge is part of the result, turned so the result lies on its
//! left; the kept edges are chained into loops, turning as sharply right as
//! possible at shared points so touching shapes stay separate outlines.

use std::collections::HashMap;

use glam::{DVec2, Vec3};

use crate::{
    exact::{Expansion, Interval, Num},
    spline::{Polyline, Segment, SplinePlane},
};

/// How the shapes are combined.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SplineBooleanMode {
    /// Inside any shape.
    #[default]
    Union,
    /// Inside the first shape and none of the others.
    Subtract,
    /// Inside every shape.
    Intersect,
    /// Inside an odd number of shapes.
    Exclusive,
}

impl SplineBooleanMode {
    pub const ALL: [Self; 4] = [Self::Union, Self::Subtract, Self::Intersect, Self::Exclusive];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Union => "Union",
            Self::Subtract => "Subtract",
            Self::Intersect => "Intersect",
            Self::Exclusive => "Exclusive",
        }
    }

    /// Whether a point inside the shapes whose bits are set in `inside` (of
    /// `count` shapes) is in the result.
    fn keeps(self, inside: &[u64], count: usize) -> bool {
        let ones: usize = inside.iter().map(|w| w.count_ones() as usize).sum();
        match self {
            Self::Union => ones > 0,
            Self::Subtract => ones == 1 && inside.first().is_some_and(|w| w & 1 == 1),
            Self::Intersect => count > 0 && ones == count,
            Self::Exclusive => ones % 2 == 1,
        }
    }
}

/// A Spline Boolean generator's settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SplineBoolean {
    pub mode: SplineBooleanMode,
    /// The plane the shapes are combined in (the generator's own axes).
    pub plane: SplinePlane,
}

/// Points closer than this (cm) become one; shorter edges vanish.
pub const MERGE_DISTANCE: f64 = 0.001;

/// A point's coordinates in `plane`, seen from the plane's front (the
/// coordinate off the plane dropped).
#[must_use]
pub fn flatten(plane: SplinePlane, p: Vec3) -> DVec2 {
    let p = p.as_dvec3();
    match plane {
        SplinePlane::XY => DVec2::new(p.x, p.y),
        SplinePlane::ZY => DVec2::new(-p.z, p.y),
        SplinePlane::XZ => DVec2::new(p.x, -p.z),
    }
}

/// The point of `plane` at coordinates `q` (the inverse of [`flatten`] on the
/// plane).
#[expect(clippy::cast_possible_truncation, reason = "coordinates back to the scene's f32")]
fn lift(plane: SplinePlane, q: DVec2) -> Vec3 {
    plane.place(Vec3::new(q.x as f32, q.y as f32, 0.0))
}

/// The sign of the turn a → b → c (1 counter-clockwise, −1 clockwise, 0
/// straight), exactly.
fn orient(a: DVec2, b: DVec2, c: DVec2) -> i8 {
    fn formula<N: Num>(a: DVec2, b: DVec2, c: DVec2) -> N {
        let n = N::from_f64;
        let (ax, ay, bx, by, cx, cy) = (n(a.x), n(a.y), n(b.x), n(b.y), n(c.x), n(c.y));
        bx.sub(&ax).mul(&cy.sub(&ay)).sub(&by.sub(&ay).mul(&cx.sub(&ax)))
    }
    formula::<Interval>(a, b, c)
        .sign()
        .unwrap_or_else(|| formula::<Expansion>(a, b, c).sign())
}

/// One input edge: its ends and the shape it belongs to.
#[derive(Clone, Copy, Debug)]
struct Piece {
    a: DVec2,
    b: DVec2,
    shape: usize,
}

impl Piece {
    fn min_x(&self) -> f64 {
        self.a.x.min(self.b.x)
    }

    fn max_x(&self) -> f64 {
        self.a.x.max(self.b.x)
    }
}

/// Fixed-size bit sets stored one after another.
struct Bits {
    words: usize,
    data: Vec<u64>,
}

impl Bits {
    fn new(words: usize) -> Self {
        Self {
            words,
            data: Vec::new(),
        }
    }

    fn push_empty(&mut self) {
        self.data.resize(self.data.len() + self.words, 0);
    }

    fn get(&self, i: usize) -> &[u64] {
        &self.data[i * self.words..(i + 1) * self.words]
    }

    fn flip(&mut self, i: usize, bit: usize) {
        self.data[i * self.words + bit / 64] ^= 1 << (bit % 64);
    }

    fn is_empty(&self, i: usize) -> bool {
        self.get(i).iter().all(|&w| w == 0)
    }
}

/// The shapes in `shapes` (each a polyline whose closed segments are its
/// outlines, read even-odd; open segments and degenerate outlines ignored)
/// combined by `settings`, flattened onto its plane: closed outlines in the
/// plane, counter-clockwise around the result and clockwise around its
/// holes, seen from the plane's front. Empty when nothing remains.
#[must_use]
pub fn spline_boolean(shapes: &[Polyline], settings: &SplineBoolean) -> Polyline {
    let (pieces, count) = pieces_of(shapes, settings.plane);
    if pieces.is_empty() {
        return Polyline::default();
    }
    let (vertices, edges, owners) = arrangement(&pieces, count);
    let kept = classify(&vertices, &edges, &owners, settings.mode, count);
    let loops = chain(&vertices, &kept);
    let mut out = Polyline::default();
    for ring in loops {
        let Ok(n) = u32::try_from(ring.len()) else { continue };
        out.points
            .extend(ring.iter().map(|&v| lift(settings.plane, vertices[v])));
        out.segments.push(Segment { count: n, closed: true });
    }
    out
}

/// The edges of every usable outline, flattened, and how many shapes have
/// one (shapes without are left out, so the first counted shape is the
/// first that has an outline).
fn pieces_of(shapes: &[Polyline], plane: SplinePlane) -> (Vec<Piece>, usize) {
    let mut pieces = Vec::new();
    let mut count = 0;
    let mut ring = Vec::new();
    for shape in shapes {
        let before = pieces.len();
        for (points, closed) in shape.runs() {
            if !closed {
                continue;
            }
            ring.clear();
            for &p in points {
                if !p.is_finite() {
                    ring.clear();
                    break;
                }
                let q = flatten(plane, p);
                if ring
                    .last()
                    .is_none_or(|&last: &DVec2| last.distance(q) > MERGE_DISTANCE)
                {
                    ring.push(q);
                }
            }
            while ring.len() > 1
                && ring
                    .first()
                    .zip(ring.last())
                    .is_some_and(|(a, b)| a.distance(*b) <= MERGE_DISTANCE)
            {
                ring.pop();
            }
            let ring_area: f64 = ring
                .iter()
                .zip(ring.iter().cycle().skip(1))
                .map(|(a, b)| a.perp_dot(*b))
                .sum::<f64>()
                * 0.5;
            if ring.len() < 3 || ring_area.abs() <= MERGE_DISTANCE * MERGE_DISTANCE {
                continue; // degenerate: fewer than three points or no area
            }
            for (k, &a) in ring.iter().enumerate() {
                let b = ring[(k + 1) % ring.len()];
                pieces.push(Piece { a, b, shape: count });
            }
        }
        if pieces.len() > before {
            count += 1;
        }
    }
    (pieces, count)
}

/// The pieces cut where they cross or touch, their ends merged: the points,
/// the edges (point pairs, smaller first) and each edge's owners (the shapes
/// that have it an odd number of times; edges no shape owns are dropped).
fn arrangement(pieces: &[Piece], count: usize) -> (Vec<DVec2>, Vec<[usize; 2]>, Bits) {
    let splits = splits(pieces);
    let mut vertices = Vertices::default();
    let mut index: HashMap<[usize; 2], usize> = HashMap::new();
    let mut edges = Vec::new();
    let mut owners = Bits::new(count.div_ceil(64).max(1));
    let mut points = Vec::new();
    for (piece, cuts) in pieces.iter().zip(&splits) {
        points.clear();
        points.push(vertices.add(piece.a));
        points.extend(cuts.iter().map(|&(_, p)| vertices.add(p)));
        points.push(vertices.add(piece.b));
        points.dedup();
        for pair in points.windows(2) {
            let key = [pair[0].min(pair[1]), pair[0].max(pair[1])];
            let edge = *index.entry(key).or_insert_with(|| {
                edges.push(key);
                owners.push_empty();
                edges.len() - 1
            });
            owners.flip(edge, piece.shape);
        }
    }
    // Edges every owner has twice cancel (a shape going back over itself).
    let mut kept_edges = Vec::with_capacity(edges.len());
    let mut kept_owners = Bits::new(owners.words);
    for (e, &edge) in edges.iter().enumerate() {
        if !owners.is_empty(e) {
            kept_edges.push(edge);
            kept_owners.data.extend_from_slice(owners.get(e));
        }
    }
    (vertices.points, kept_edges, kept_owners)
}

/// Points merged on a grid of [`MERGE_DISTANCE`] cells.
#[derive(Default)]
struct Vertices {
    points: Vec<DVec2>,
    cells: HashMap<glam::I64Vec2, Vec<usize>>,
}

impl Vertices {
    fn add(&mut self, p: DVec2) -> usize {
        let cell = (p / MERGE_DISTANCE).floor().as_i64vec2();
        for dx in -1..=1 {
            for dy in -1..=1 {
                let near = cell.saturating_add(glam::I64Vec2::new(dx, dy));
                if let Some(found) = self.cells.get(&near).and_then(|list| {
                    list.iter()
                        .copied()
                        .find(|&v| self.points[v].distance(p) <= MERGE_DISTANCE)
                }) {
                    return found;
                }
            }
        }
        self.points.push(p);
        let v = self.points.len() - 1;
        self.cells.entry(cell).or_default().push(v);
        v
    }
}

/// Where each piece is cut, by its parameter (sorted): at crossings with
/// other pieces and where another piece's end touches it.
fn splits(pieces: &[Piece]) -> Vec<Vec<(f64, DVec2)>> {
    let mut cuts: Vec<Vec<(f64, DVec2)>> = vec![Vec::new(); pieces.len()];
    let mut order: Vec<usize> = (0..pieces.len()).collect();
    order.sort_by(|&i, &j| pieces[i].min_x().total_cmp(&pieces[j].min_x()));
    for (k, &i) in order.iter().enumerate() {
        let p = pieces[i];
        let (p_lo, p_hi) = (p.a.y.min(p.b.y), p.a.y.max(p.b.y));
        for &j in &order[k + 1..] {
            let q = pieces[j];
            if q.min_x() > p.max_x() + MERGE_DISTANCE {
                break;
            }
            if q.a.y.max(q.b.y) < p_lo - MERGE_DISTANCE || q.a.y.min(q.b.y) > p_hi + MERGE_DISTANCE {
                continue;
            }
            // Ends touching the other piece's inside (T-junctions, overlaps).
            for (from, onto) in [(q, i), (p, j)] {
                for end in [from.a, from.b] {
                    if let Some(t) = touches(&pieces[onto], end) {
                        cuts[onto].push((t, end));
                    }
                }
            }
            // A proper crossing.
            let (o1, o2) = (orient(p.a, p.b, q.a), orient(p.a, p.b, q.b));
            let (o3, o4) = (orient(q.a, q.b, p.a), orient(q.a, q.b, p.b));
            if o1 * o2 < 0 && o3 * o4 < 0 {
                let (d, e) = (p.b - p.a, q.b - q.a);
                let denominator = d.perp_dot(e);
                if denominator != 0.0 {
                    let t = (q.a - p.a).perp_dot(e) / denominator;
                    let u = (q.a - p.a).perp_dot(d) / denominator;
                    let point = p.a + d * t.clamp(0.0, 1.0);
                    cuts[i].push((t, point));
                    cuts[j].push((u, point));
                }
            }
        }
    }
    for list in &mut cuts {
        list.sort_by(|x, y| x.0.total_cmp(&y.0));
    }
    cuts
}

/// The parameter where `point` touches the inside of `piece` (within
/// [`MERGE_DISTANCE`], away from its ends); `None` if it does not.
fn touches(piece: &Piece, point: DVec2) -> Option<f64> {
    let d = piece.b - piece.a;
    let length_squared = d.length_squared();
    if length_squared == 0.0 {
        return None;
    }
    let t = (point - piece.a).dot(d) / length_squared;
    let foot = piece.a + d * t;
    let near_ends = point.distance(piece.a) <= MERGE_DISTANCE || point.distance(piece.b) <= MERGE_DISTANCE;
    (t > 0.0 && t < 1.0 && !near_ends && foot.distance(point) <= MERGE_DISTANCE).then_some(t)
}

/// The edges on the result's boundary, each turned so the result lies on its
/// left (as point pairs, from → to).
fn classify(
    vertices: &[DVec2],
    edges: &[[usize; 2]],
    owners: &Bits,
    mode: SplineBooleanMode,
    count: usize,
) -> Vec<[usize; 2]> {
    let rows = Bins::new(vertices, edges, 1);
    let columns = Bins::new(vertices, edges, 0);
    let mut side = vec![0u64; owners.words];
    let mut other = vec![0u64; owners.words];
    let mut kept = Vec::new();
    for (e, &[u, v]) in edges.iter().enumerate() {
        let (a, b) = (vertices[u], vertices[v]);
        let d = b - a;
        let middle = (a + b) * 0.5;
        // A ray along +X for steep edges, +Y for flat ones.
        let along_x = d.y.abs() >= d.x.abs();
        side.iter_mut().for_each(|w| *w = 0);
        let bins = if along_x { &rows } else { &columns };
        for &f in bins.near(middle) {
            if f == e {
                continue;
            }
            let [p, q] = edges[f];
            let (p, q) = (vertices[p], vertices[q]);
            let crosses = if along_x {
                let (lo, hi) = if p.y <= q.y { (p, q) } else { (q, p) };
                (lo.y <= middle.y && hi.y > middle.y) && orient(lo, hi, middle) > 0
            } else {
                let (lo, hi) = if p.x <= q.x { (p, q) } else { (q, p) };
                (lo.x <= middle.x && hi.x > middle.x) && orient(lo, hi, middle) < 0
            };
            if crosses {
                for (w, o) in side.iter_mut().zip(owners.get(f)) {
                    *w ^= o;
                }
            }
        }
        for ((o, s), own) in other.iter_mut().zip(&side).zip(owners.get(e)) {
            *o = s ^ own;
        }
        let (in_ray, in_other) = (mode.keeps(&side, count), mode.keeps(&other, count));
        if in_ray == in_other {
            continue;
        }
        // Whether the ray's side is left of u → v.
        let ray_left = if along_x { d.y < 0.0 } else { d.x > 0.0 };
        kept.push(if in_ray == ray_left { [u, v] } else { [v, u] });
    }
    kept
}

/// Edges sorted into bands across one axis, for the rays of [`classify`].
struct Bins {
    /// The axis the bands are cut across (1: rows by Y, 0: columns by X).
    axis: usize,
    start: f64,
    size: f64,
    bands: Vec<Vec<usize>>,
}

impl Bins {
    fn new(vertices: &[DVec2], edges: &[[usize; 2]], axis: usize) -> Self {
        let coordinate = |v: usize| vertices[v][axis];
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for &[u, v] in edges {
            lo = lo.min(coordinate(u)).min(coordinate(v));
            hi = hi.max(coordinate(u)).max(coordinate(v));
        }
        let count = edges.len().isqrt().clamp(1, 4096);
        let size = ((hi - lo) / count as f64).max(MERGE_DISTANCE);
        let mut bins = Self {
            axis,
            start: lo,
            size,
            bands: vec![Vec::new(); count],
        };
        for (e, &[u, v]) in edges.iter().enumerate() {
            let (a, b) = (coordinate(u), coordinate(v));
            let (first, last) = (bins.band(a.min(b)), bins.band(a.max(b)));
            for band in &mut bins.bands[first..=last] {
                band.push(e);
            }
        }
        bins
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to the bands"
    )]
    fn band(&self, value: f64) -> usize {
        let k = ((value - self.start) / self.size).floor();
        if k.is_nan() || k < 0.0 {
            0
        } else {
            (k as usize).min(self.bands.len() - 1)
        }
    }

    /// The edges whose band holds `point`'s coordinate.
    fn near(&self, point: DVec2) -> &[usize] {
        &self.bands[self.band(point[self.axis])]
    }
}

/// The kept edges chained into loops (point indices, each loop closed);
/// loops of fewer than three points or no area are dropped.
fn chain(vertices: &[DVec2], kept: &[[usize; 2]]) -> Vec<Vec<usize>> {
    let mut outgoing: HashMap<usize, Vec<usize>> = HashMap::new();
    for (h, &[from, _]) in kept.iter().enumerate() {
        outgoing.entry(from).or_default().push(h);
    }
    let angle = |h: usize| {
        let [from, to] = kept[h];
        let d = vertices[to] - vertices[from];
        d.y.atan2(d.x)
    };
    let mut used = vec![false; kept.len()];
    let mut loops = Vec::new();
    for first in 0..kept.len() {
        if used[first] {
            continue;
        }
        used[first] = true;
        let start = kept[first][0];
        let mut ring = vec![start];
        let mut h = first;
        loop {
            let [from, at] = kept[h];
            let back = {
                let d = vertices[from] - vertices[at];
                d.y.atan2(d.x)
            };
            // The sharpest right turn: the least clockwise rotation from the
            // way back.
            let turn = |candidate: usize| {
                let delta = (back - angle(candidate)).rem_euclid(std::f64::consts::TAU);
                if delta <= 0.0 { std::f64::consts::TAU } else { delta }
            };
            let next = outgoing
                .get(&at)
                .into_iter()
                .flatten()
                .copied()
                .filter(|&c| !used[c] || (c == first && at == start))
                .min_by(|&x, &y| turn(x).total_cmp(&turn(y)));
            match next {
                Some(next) if next == first => break,
                Some(next) => {
                    used[next] = true;
                    ring.push(at);
                    h = next;
                }
                None => {
                    ring.push(at);
                    break;
                }
            }
        }
        if ring.len() >= 3 && area(vertices, &ring).abs() > MERGE_DISTANCE * MERGE_DISTANCE {
            loops.push(ring);
        }
    }
    loops
}

/// The signed area of a loop (positive counter-clockwise).
fn area(vertices: &[DVec2], ring: &[usize]) -> f64 {
    let mut sum = 0.0;
    for (k, &v) in ring.iter().enumerate() {
        let w = ring[(k + 1) % ring.len()];
        sum += vertices[v].perp_dot(vertices[w]);
    }
    sum * 0.5
}

/// The signed area of every closed segment of `polyline` in `plane`, summed
/// (holes subtract when wound the other way).
#[must_use]
pub fn polyline_area(polyline: &Polyline, plane: SplinePlane) -> f64 {
    polyline
        .runs()
        .filter(|&(_, closed)| closed)
        .map(|(points, _)| {
            let mut sum = 0.0;
            for (k, &p) in points.iter().enumerate() {
                let q = points[(k + 1) % points.len()];
                sum += flatten(plane, p).perp_dot(flatten(plane, q));
            }
            sum * 0.5
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use super::*;

    fn circle(center: [f32; 2], radius: f32, n: usize) -> Polyline {
        let points = (0..n)
            .map(|k| {
                let t = std::f32::consts::TAU * k as f32 / n as f32;
                Vec3::new(center[0] + radius * t.cos(), center[1] + radius * t.sin(), 0.0)
            })
            .collect();
        closed(points)
    }

    fn closed(points: Vec<Vec3>) -> Polyline {
        let count = u32::try_from(points.len()).unwrap();
        Polyline {
            points,
            segments: vec![Segment { count, closed: true }],
        }
    }

    fn rectangle(min: [f32; 2], max: [f32; 2]) -> Polyline {
        closed(vec![
            Vec3::new(min[0], min[1], 0.0),
            Vec3::new(max[0], min[1], 0.0),
            Vec3::new(max[0], max[1], 0.0),
            Vec3::new(min[0], max[1], 0.0),
        ])
    }

    fn run(shapes: &[Polyline], mode: SplineBooleanMode) -> Polyline {
        spline_boolean(
            shapes,
            &SplineBoolean {
                mode,
                plane: SplinePlane::XY,
            },
        )
    }

    fn area_of(p: &Polyline) -> f64 {
        polyline_area(p, SplinePlane::XY)
    }

    /// The area of each closed segment.
    fn areas(p: &Polyline) -> Vec<f64> {
        p.runs().map(|(points, _)| area_of(&closed(points.to_vec()))).collect()
    }

    #[test]
    fn two_overlapping_circles_in_every_mode() {
        // Radius 100, centres 100 apart: the lens is 2r²(π/3 − √3/4)·2.
        let r = 100.0f64;
        let (a, b) = (circle([0.0, 0.0], 100.0, 2048), circle([100.0, 0.0], 100.0, 2048));
        let disc = PI * r * r;
        let lens = 2.0 * r * r * (2.0 * PI / 3.0 - 3f64.sqrt() / 2.0) / 2.0;
        let close = |got: f64, want: f64| (got - want).abs() / want < 1e-3;
        let union = run(&[a.clone(), b.clone()], SplineBooleanMode::Union);
        assert_eq!(union.segments.len(), 1, "one outline");
        assert!(close(area_of(&union), 2.0 * disc - lens), "{}", area_of(&union));
        let subtract = run(&[a.clone(), b.clone()], SplineBooleanMode::Subtract);
        assert_eq!(subtract.segments.len(), 1, "a crescent");
        assert!(close(area_of(&subtract), disc - lens));
        let intersect = run(&[a.clone(), b.clone()], SplineBooleanMode::Intersect);
        assert_eq!(intersect.segments.len(), 1, "a lens");
        assert!(close(area_of(&intersect), lens));
        let exclusive = run(&[a, b], SplineBooleanMode::Exclusive);
        assert_eq!(exclusive.segments.len(), 2, "two crescents");
        assert!(close(area_of(&exclusive), 2.0 * (disc - lens)));
        for result in [&union, &subtract, &intersect, &exclusive] {
            assert!(areas(result).iter().all(|&a| a > 0.0), "outlines counter-clockwise");
        }
    }

    #[test]
    fn a_rectangle_minus_a_circle_inside_has_a_hole() {
        let plate = rectangle([-100.0, -50.0], [100.0, 50.0]);
        let hole = circle([0.0, 0.0], 20.0, 64);
        let result = run(&[plate, hole.clone()], SplineBooleanMode::Subtract);
        let mut found = areas(&result);
        found.sort_by(f64::total_cmp);
        assert_eq!(found.len(), 2);
        assert!(found[0] < 0.0, "the hole is clockwise");
        assert!((found[0] + area_of(&hole)).abs() < 1e-3);
        assert!((found[1] - 20_000.0).abs() < 1e-3, "{found:?}");
    }

    #[test]
    fn coincident_edges_and_touching_corners_stay_clean() {
        // Two squares sharing an edge: one square of twice the area, no sliver.
        let a = rectangle([0.0, 0.0], [10.0, 10.0]);
        let b = rectangle([10.0, 0.0], [20.0, 10.0]);
        let union = run(&[a.clone(), b.clone()], SplineBooleanMode::Union);
        assert_eq!(areas(&union), vec![200.0]);
        assert!(
            run(&[a.clone(), b.clone()], SplineBooleanMode::Intersect)
                .segments
                .is_empty()
        );
        assert_eq!(areas(&run(&[a.clone(), b], SplineBooleanMode::Subtract)), vec![100.0]);
        // Two squares touching at a corner: two outlines.
        let c = rectangle([10.0, 10.0], [20.0, 20.0]);
        let touching = run(&[a.clone(), c], SplineBooleanMode::Union);
        assert_eq!(areas(&touching), vec![100.0, 100.0]);
        // A square over itself: the same square, or nothing.
        assert_eq!(
            areas(&run(&[a.clone(), a.clone()], SplineBooleanMode::Union)),
            vec![100.0]
        );
        assert!(
            run(&[a.clone(), a.clone()], SplineBooleanMode::Exclusive)
                .segments
                .is_empty()
        );
        // Overlapping along part of an edge (a T-junction at both ends).
        let d = rectangle([5.0, 10.0], [8.0, 15.0]);
        let t = run(&[a, d], SplineBooleanMode::Union);
        assert_eq!(t.segments.len(), 1);
        assert!((area_of(&t) - 115.0).abs() < 1e-6);
    }

    #[test]
    fn nested_outlines_read_even_odd_and_open_ones_are_ignored() {
        // A ring (outline + hole in one shape) united with a small disc in
        // its hole: the ring and the disc.
        let mut ring = rectangle([-10.0, -10.0], [10.0, 10.0]);
        let hole = rectangle([-5.0, -5.0], [5.0, 5.0]);
        ring.points.extend(hole.points);
        ring.segments.extend(hole.segments);
        let disc = rectangle([-1.0, -1.0], [1.0, 1.0]);
        let union = run(&[ring.clone(), disc], SplineBooleanMode::Union);
        assert!((area_of(&union) - (400.0 - 100.0 + 4.0)).abs() < 1e-6);
        assert_eq!(union.segments.len(), 3);
        // An open segment and a degenerate one count for nothing.
        let mut open = rectangle([-100.0, -100.0], [100.0, 100.0]);
        open.segments[0].closed = false;
        let line = closed(vec![Vec3::ZERO, Vec3::X, Vec3::X * 2.0]);
        let with = run(&[ring.clone(), open, line], SplineBooleanMode::Intersect);
        assert!((area_of(&with) - 300.0).abs() < 1e-6, "only the ring counts");
    }

    #[test]
    fn huge_coordinates_never_panic() {
        // Found by the fuzzer: points near f32::MAX put the merge grid's
        // cells at the end of the integer range.
        let huge = closed(vec![
            Vec3::new(3e38, 3e38, 0.0),
            Vec3::new(-3e38, 3e38, 0.0),
            Vec3::new(-3e38, -3e38, 0.0),
            Vec3::new(3e38, -3e38, 0.0),
        ]);
        let small = rectangle([0.0, 0.0], [10.0, 10.0]);
        let tiny_far = closed(vec![
            Vec3::new(f32::MAX, 0.0, 0.0),
            Vec3::new(f32::MAX, 1.0, 0.0),
            Vec3::new(-f32::MAX, 0.5, 0.0),
        ]);
        for mode in SplineBooleanMode::ALL {
            for shapes in [vec![huge.clone(), small.clone()], vec![tiny_far.clone(), small.clone()]] {
                let result = run(&shapes, mode);
                let counted: usize = result.segments.iter().map(|s| s.count as usize).sum();
                assert_eq!(counted, result.points.len());
                assert!(result.segments.iter().all(|s| s.closed && s.count >= 3));
            }
        }
    }

    #[test]
    fn shapes_off_the_plane_are_flattened_onto_it() {
        let tilted = Polyline {
            points: rectangle([0.0, 0.0], [10.0, 10.0])
                .points
                .iter()
                .map(|p| Vec3::new(p.x, p.y, p.x * 3.0))
                .collect(),
            segments: vec![Segment { count: 4, closed: true }],
        };
        let result = run(&[tilted], SplineBooleanMode::Union);
        assert_eq!(areas(&result), vec![100.0]);
        assert!(result.points.iter().all(|p| p.z == 0.0), "in the plane");
        // In ZY and XZ the result lies in those planes and faces their front.
        for plane in [SplinePlane::ZY, SplinePlane::XZ] {
            let shape = Polyline {
                points: rectangle([0.0, 0.0], [10.0, 10.0])
                    .points
                    .iter()
                    .map(|&p| plane.place(p))
                    .collect(),
                segments: vec![Segment { count: 4, closed: true }],
            };
            let result = spline_boolean(
                &[shape],
                &SplineBoolean {
                    mode: SplineBooleanMode::Union,
                    plane,
                },
            );
            assert!((polyline_area(&result, plane) - 100.0).abs() < 1e-6);
        }
    }

    #[test]
    fn garbage_never_panics() {
        let mut state = 12345u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            f32::from(u16::try_from(state % 64).unwrap()) * 0.25
        };
        for _ in 0..200 {
            let shapes: Vec<Polyline> = (0..3)
                .map(|_| closed((0..6).map(|_| Vec3::new(next(), next(), 0.0)).collect()))
                .collect();
            for mode in SplineBooleanMode::ALL {
                let result = run(&shapes, mode);
                assert_eq!(
                    result.segments.iter().map(|s| s.count as usize).sum::<usize>(),
                    result.points.len()
                );
            }
        }
        let bad = closed(vec![Vec3::NAN, Vec3::X, Vec3::Y]);
        assert!(run(&[bad], SplineBooleanMode::Union).segments.is_empty());
        assert!(run(&[], SplineBooleanMode::Intersect).segments.is_empty());
    }
}
