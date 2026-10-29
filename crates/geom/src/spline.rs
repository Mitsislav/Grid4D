//! Splines: curves through (or guided by) control points, in one or more
//! segments, and the primitives that make them (see
//! `docs/reference/object-spline.md`).
//!
//! Every type is evaluated as a chain of cubic Bezier **spans** — four control
//! points each — so sampling ([`Spline::sample`]) has one code path: Linear
//! spans are straight, Bezier spans use the points' tangents, Cubic and Akima
//! spans are Hermite curves with tangents from the neighbours, B-Spline spans
//! come from four consecutive points.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use glam::Vec3;

/// How control points are joined.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SplineType {
    /// Straight lines.
    #[default]
    Linear,
    /// Smooth through the points (may overshoot).
    Cubic,
    /// Smooth through the points without overshoot.
    Akima,
    /// Very smooth; the points only pull the curve.
    BSpline,
    /// Through the points, shaped by each point's two tangents.
    Bezier,
}

impl SplineType {
    pub const ALL: [Self; 5] = [Self::Linear, Self::Cubic, Self::Akima, Self::BSpline, Self::Bezier];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::Cubic => "Cubic",
            Self::Akima => "Akima",
            Self::BSpline => "B-Spline",
            Self::Bezier => "Bezier",
        }
    }
}

/// One run of consecutive points: how many, and whether it closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Segment {
    pub count: u32,
    pub closed: bool,
}

/// A curve: control points, their Bezier tangents, and the segments they form.
#[derive(Clone, Debug, PartialEq)]
pub struct Spline {
    pub kind: SplineType,
    pub points: Vec<Vec3>,
    /// Per point: the incoming (left) and outgoing (right) tangent, relative to
    /// the point. Used by Bezier splines; zero tangents make straight joins.
    pub tangents: Vec<[Vec3; 2]>,
    /// Consecutive runs of `points`; their counts add up to the number of points.
    pub segments: Vec<Segment>,
}

/// How a spline is cut into straight pieces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Interpolation {
    /// The control points only.
    None,
    /// `number` points between each pair of control points, at equal steps of
    /// the curve parameter (denser where points are close).
    Natural { number: u32 },
    /// The same number of points, at equal distances along the curve.
    Uniform { number: u32 },
    /// A point wherever the curve turns by more than `angle` (radians).
    Adaptive { angle: f32 },
    /// Adaptive, then no piece longer than `max_length`.
    Subdivided { angle: f32, max_length: f32 },
}

impl Default for Interpolation {
    fn default() -> Self {
        Self::Adaptive {
            angle: 5f32.to_radians(),
        }
    }
}

impl Interpolation {
    pub const MAX_NUMBER: u32 = 1000;
    pub const MIN_ANGLE: f32 = 0.1 * PI / 180.0;
    pub const MIN_LENGTH: f32 = 0.1;

    /// The same settings, every value in its range.
    #[must_use]
    pub fn validated(self) -> Self {
        let angle = |a: f32| a.clamp(Self::MIN_ANGLE, FRAC_PI_2);
        match self {
            Self::None => Self::None,
            Self::Natural { number } => Self::Natural {
                number: number.min(Self::MAX_NUMBER),
            },
            Self::Uniform { number } => Self::Uniform {
                number: number.min(Self::MAX_NUMBER),
            },
            Self::Adaptive { angle: a } => Self::Adaptive { angle: angle(a) },
            Self::Subdivided { angle: a, max_length } => Self::Subdivided {
                angle: angle(a),
                max_length: max_length.max(Self::MIN_LENGTH),
            },
        }
    }
}

/// A spline cut into straight pieces: points per segment.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Polyline {
    pub points: Vec<Vec3>,
    /// Consecutive runs of `points`, each open or closed (the last point of a
    /// closed run joins its first).
    pub segments: Vec<Segment>,
}

impl Polyline {
    /// The points of each segment, with whether it closes.
    pub fn runs(&self) -> impl Iterator<Item = (&[Vec3], bool)> {
        runs(&self.points, &self.segments)
    }

    /// Smallest and largest coordinates; `None` without points.
    #[must_use]
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let first = *self.points.first()?;
        Some(
            self.points
                .iter()
                .fold((first, first), |(min, max), &p| (min.min(p), max.max(p))),
        )
    }
}

/// Splits `points` into the runs `segments` describe (a short last run if the
/// counts claim more points than there are).
fn runs<'a, T>(points: &'a [T], segments: &'a [Segment]) -> impl Iterator<Item = (&'a [T], bool)> {
    let mut start = 0usize;
    segments.iter().map(move |segment| {
        let count = usize::try_from(segment.count).unwrap_or(0);
        let end = (start + count).min(points.len());
        let run = &points[start.min(end)..end];
        start = end;
        (run, segment.closed)
    })
}

impl Spline {
    /// A spline of one segment.
    #[must_use]
    pub fn new(kind: SplineType, points: Vec<Vec3>, closed: bool) -> Self {
        let count = u32::try_from(points.len()).unwrap_or(u32::MAX);
        Self {
            kind,
            tangents: vec![[Vec3::ZERO; 2]; points.len()],
            points,
            segments: vec![Segment { count, closed }],
        }
    }

    /// Appends `other`'s segments (its type is ignored).
    pub fn append(&mut self, other: Self) {
        self.points.extend(other.points);
        self.tangents.extend(other.tangents);
        self.segments.extend(other.segments);
    }

    /// The same curve run backwards: each segment's points in the opposite
    /// order, tangents swapped.
    #[must_use]
    pub fn reversed(mut self) -> Self {
        let mut start = 0usize;
        for segment in &self.segments {
            let count = usize::try_from(segment.count).unwrap_or(0);
            let end = (start + count).min(self.points.len());
            self.points[start..end].reverse();
            self.tangents[start..end].reverse();
            for [left, right] in &mut self.tangents[start..end] {
                std::mem::swap(left, right);
            }
            start = end;
        }
        self
    }

    /// Applies `f` to every point and tangent direction (tangents are
    /// directions: `f` must be linear, e.g. a rotation).
    #[must_use]
    pub fn mapped(mut self, f: impl Fn(Vec3) -> Vec3) -> Self {
        for p in &mut self.points {
            *p = f(*p);
        }
        for [left, right] in &mut self.tangents {
            *left = f(*left);
            *right = f(*right);
        }
        self
    }

    /// The spline without the points `remove` flags, and where each old point
    /// went (`None`: removed). A segment left without points disappears; one
    /// left with a single point stays open.
    #[must_use]
    pub fn without_points(&self, remove: &crate::BitSet) -> (Self, Vec<Option<u32>>) {
        let mut out = Self {
            kind: self.kind,
            points: Vec::new(),
            tangents: Vec::new(),
            segments: Vec::new(),
        };
        let mut map = Vec::with_capacity(self.points.len());
        let mut start = 0usize;
        for segment in &self.segments {
            let count = usize::try_from(segment.count).unwrap_or(0);
            let end = (start + count).min(self.points.len());
            let before = out.points.len();
            for i in start..end {
                if remove.contains(i) {
                    map.push(None);
                    continue;
                }
                map.push(u32::try_from(out.points.len()).ok());
                out.points.push(self.points[i]);
                out.tangents
                    .push(self.tangents.get(i).copied().unwrap_or([Vec3::ZERO; 2]));
            }
            let kept = out.points.len() - before;
            if kept > 0 {
                out.segments.push(Segment {
                    count: u32::try_from(kept).unwrap_or(u32::MAX),
                    closed: segment.closed && kept > 1,
                });
            }
            start = end;
        }
        (out, map)
    }

    /// Gives every point without tangents smooth ones (the direction from its
    /// neighbour before to its neighbour after, a third of the way, like a
    /// Cubic spline's), leaving set tangents alone: what turning a spline into
    /// a Bezier one does.
    pub fn smooth_tangents(&mut self) {
        let mut start = 0usize;
        for segment in &self.segments {
            let count = usize::try_from(segment.count).unwrap_or(0);
            let end = (start + count).min(self.points.len());
            let points = &self.points[start..end];
            let n = points.len();
            for i in 0..n {
                if self.tangents[start + i] != [Vec3::ZERO; 2] || n < 2 {
                    continue;
                }
                let before = if i > 0 || segment.closed {
                    points[(i + n - 1) % n]
                } else {
                    points[i]
                };
                let after = if i + 1 < n || segment.closed {
                    points[(i + 1) % n]
                } else {
                    points[i]
                };
                // Half the neighbours' difference is the Hermite tangent; a third of that the handle.
                // An open segment's end has one neighbour: the whole third towards it.
                let end_point = !segment.closed && (i == 0 || i + 1 == n);
                let d = (after - before) / if end_point { 3.0 } else { 6.0 };
                self.tangents[start + i] = [-d, d];
            }
            start = end;
        }
    }

    /// The cubic Bezier spans of each segment, as `[start, control, control, end]`.
    pub fn spans(&self) -> impl Iterator<Item = (Vec<[Vec3; 4]>, bool)> + '_ {
        let tangents = runs(&self.tangents, &self.segments).map(|(t, _)| t);
        runs(&self.points, &self.segments)
            .zip(tangents)
            .map(|((points, closed), tangents)| (segment_spans(self.kind, points, tangents, closed), closed))
    }

    /// The curve cut into straight pieces by `interpolation`.
    #[must_use]
    pub fn sample(&self, interpolation: Interpolation) -> Polyline {
        let interpolation = interpolation.validated();
        let mut out = Polyline::default();
        for (spans, closed) in self.spans() {
            let before = out.points.len();
            sample_segment(&spans, closed, interpolation, &mut out.points);
            let count = u32::try_from(out.points.len() - before).unwrap_or(u32::MAX);
            out.segments.push(Segment { count, closed });
        }
        out
    }
}

/// A point on a cubic Bezier span.
fn bezier(span: &[Vec3; 4], t: f32) -> Vec3 {
    let u = 1.0 - t;
    span[0] * (u * u * u) + span[1] * (3.0 * u * u * t) + span[2] * (3.0 * u * t * t) + span[3] * (t * t * t)
}

/// The Bezier span of a Hermite curve from `p0` to `p1` with tangents `t0`, `t1`.
fn hermite(p0: Vec3, t0: Vec3, p1: Vec3, t1: Vec3) -> [Vec3; 4] {
    [p0, p0 + t0 / 3.0, p1 - t1 / 3.0, p1]
}

fn segment_spans(kind: SplineType, points: &[Vec3], tangents: &[[Vec3; 2]], closed: bool) -> Vec<[Vec3; 4]> {
    let n = points.len();
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![[points[0]; 4]];
    }
    let pairs = if closed { n } else { n - 1 };
    let next = |i: usize| (i + 1) % n;
    match kind {
        SplineType::Linear => (0..pairs)
            .map(|i| {
                let (a, b) = (points[i], points[next(i)]);
                [a, a.lerp(b, 1.0 / 3.0), a.lerp(b, 2.0 / 3.0), b]
            })
            .collect(),
        SplineType::Bezier => (0..pairs)
            .map(|i| {
                let j = next(i);
                let right = tangents.get(i).map_or(Vec3::ZERO, |t| t[1]);
                let left = tangents.get(j).map_or(Vec3::ZERO, |t| t[0]);
                [points[i], points[i] + right, points[j] + left, points[j]]
            })
            .collect(),
        SplineType::Cubic => {
            let tangent = |i: usize| {
                if closed {
                    (points[next(i)] - points[(i + n - 1) % n]) * 0.5
                } else if i == 0 {
                    points[1] - points[0]
                } else if i == n - 1 {
                    points[n - 1] - points[n - 2]
                } else {
                    (points[i + 1] - points[i - 1]) * 0.5
                }
            };
            (0..pairs)
                .map(|i| hermite(points[i], tangent(i), points[next(i)], tangent(next(i))))
                .collect()
        }
        SplineType::Akima => {
            let tangents = akima_tangents(points, closed);
            (0..pairs)
                .map(|i| hermite(points[i], tangents[i], points[next(i)], tangents[next(i)]))
                .collect()
        }
        SplineType::BSpline => {
            // Open: the end points repeated so the curve starts and ends on them.
            let q: Vec<Vec3> = if closed {
                (0..n + 3).map(|i| points[(i + n - 1) % n]).collect()
            } else {
                let mut q = vec![points[0], points[0]];
                q.extend_from_slice(points);
                q.extend([points[n - 1], points[n - 1]]);
                q
            };
            q.windows(4)
                .take(if closed { n } else { n + 1 })
                .map(|w| {
                    [
                        (w[0] + w[1] * 4.0 + w[2]) / 6.0,
                        (w[1] * 4.0 + w[2] * 2.0) / 6.0,
                        (w[1] * 2.0 + w[2] * 4.0) / 6.0,
                        (w[1] + w[2] * 4.0 + w[3]) / 6.0,
                    ]
                })
                .collect()
        }
    }
}

/// Akima's tangents, per coordinate-free form: a weighted mean of the slopes
/// on either side, the weights from how much the slopes change further out,
/// so a straight run stays straight (no overshoot).
fn akima_tangents(points: &[Vec3], closed: bool) -> Vec<Vec3> {
    let n = points.len();
    // Slopes m[k] = p[k+1] - p[k], with two extra on each side.
    let slope = |k: isize| -> Vec3 {
        let len = isize::try_from(n).unwrap_or(isize::MAX);
        let at = |i: isize| points[usize::try_from(i.rem_euclid(len)).unwrap_or(0)];
        if closed {
            return at(k + 1) - at(k);
        }
        let last = len - 2; // the last real slope
        let real = |k: isize| at(k + 1) - at(k);
        match k {
            k if k < 0 && last >= 1 => {
                // Extrapolated: m[-1] = 2 m[0] - m[1], m[-2] = 2 m[-1] - m[0].
                let m0 = real(0);
                let m1 = real(1);
                let m_1 = m0 * 2.0 - m1;
                if k == -1 { m_1 } else { m_1 * 2.0 - m0 }
            }
            k if k > last && last >= 1 => {
                let ml = real(last);
                let ml1 = real(last - 1);
                let m_n = ml * 2.0 - ml1;
                if k == last + 1 { m_n } else { m_n * 2.0 - ml }
            }
            k => real(k.clamp(0, last.max(0))),
        }
    };
    (0..n)
        .map(|i| {
            let i = isize::try_from(i).unwrap_or(0);
            let (m2, m1, m0, p1) = (slope(i - 2), slope(i - 1), slope(i), slope(i + 1));
            let w1 = (p1 - m0).length();
            let w2 = (m1 - m2).length();
            if w1 + w2 <= f32::EPSILON {
                (m1 + m0) * 0.5
            } else {
                (m1 * w1 + m0 * w2) / (w1 + w2)
            }
        })
        .collect()
}

/// Fine steps per span when measuring turn or length.
const MEASURE_STEPS: u32 = 32;

fn sample_segment(spans: &[[Vec3; 4]], closed: bool, interpolation: Interpolation, out: &mut Vec<Vec3>) {
    let Some(first) = spans.first() else {
        return;
    };
    if spans.len() == 1 && first.iter().all(|&p| p == first[0]) {
        out.push(first[0]); // a single point
        return;
    }
    match interpolation {
        Interpolation::None => {
            out.extend(spans.iter().map(|s| s[0]));
        }
        Interpolation::Natural { number } => {
            for span in spans {
                out.extend((0..=number).map(|k| bezier(span, step(k, number + 1))));
            }
        }
        Interpolation::Uniform { number } => {
            // Ends on the last point of an open segment by itself.
            uniform(spans, closed, number, out);
            return;
        }
        Interpolation::Adaptive { angle } => {
            for span in spans {
                let pieces = adaptive_pieces(span, angle);
                out.extend((0..pieces).map(|k| bezier(span, step(k, pieces))));
            }
        }
        Interpolation::Subdivided { angle, max_length } => {
            for span in spans {
                let pieces = adaptive_pieces(span, angle);
                for k in 0..pieces {
                    let (t0, t1) = (step(k, pieces), step(k + 1, pieces));
                    let length = bezier(span, t0).distance(bezier(span, t1));
                    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "clamped")]
                    let parts = (length / max_length).ceil().clamp(1.0, 4096.0) as u32;
                    out.extend((0..parts).map(|j| bezier(span, t0 + (t1 - t0) * step(j, parts))));
                }
            }
        }
    }
    if !closed && let Some(last) = spans.last() {
        out.push(last[3]);
    }
}

/// `k / n` as a curve parameter.
fn step(k: u32, n: u32) -> f32 {
    f32::from(u16::try_from(k).unwrap_or(u16::MAX)) / f32::from(u16::try_from(n.max(1)).unwrap_or(u16::MAX))
}

/// Pieces a span needs so that none turns by more than `angle`.
fn adaptive_pieces(span: &[Vec3; 4], angle: f32) -> u32 {
    let mut turn = 0.0;
    let mut previous: Option<Vec3> = None;
    for k in 0..MEASURE_STEPS {
        let d = bezier(span, step(k + 1, MEASURE_STEPS)) - bezier(span, step(k, MEASURE_STEPS));
        let Some(d) = d.try_normalize() else { continue };
        if let Some(p) = previous {
            turn += p.dot(d).clamp(-1.0, 1.0).acos();
        }
        previous = Some(d);
    }
    // Rounded up; at most a few hundred pieces per span.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 1..=512"
    )]
    let pieces = (turn / angle).ceil().clamp(1.0, 512.0) as u32;
    pieces
}

/// `number` points between control points, spaced evenly along the whole
/// segment, from its first point (to its last, if open).
fn uniform(spans: &[[Vec3; 4]], closed: bool, number: u32, out: &mut Vec<Vec3>) {
    // A fine polyline with its running length.
    let mut fine = Vec::new();
    for span in spans {
        fine.extend((0..MEASURE_STEPS).map(|k| bezier(span, step(k, MEASURE_STEPS))));
    }
    if let Some(last) = spans.last() {
        fine.push(last[3]);
    }
    let mut lengths = Vec::with_capacity(fine.len());
    let mut total = 0.0;
    lengths.push(0.0);
    for pair in fine.windows(2) {
        total += pair[0].distance(pair[1]);
        lengths.push(total);
    }
    let vertices = u32::try_from(spans.len()).unwrap_or(u32::MAX);
    let count = if closed {
        (number + 1) * vertices
    } else {
        (number + 1) * vertices + 1
    };
    let gaps = if closed { count } else { count - 1 };
    let mut index = 0usize;
    for k in 0..count {
        let target = total * step(k, gaps);
        while index + 2 < lengths.len() && lengths[index + 1] < target {
            index += 1;
        }
        let (a, b) = (lengths[index], lengths[index + 1]);
        let t = if b > a {
            ((target - a) / (b - a)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        out.push(fine[index].lerp(fine[index + 1], t));
    }
}

// ---------------------------------------------------------------------------
// Primitives

/// Which plane a primitive lies in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SplinePlane {
    /// Facing the front view (+Z).
    #[default]
    XY,
    /// Facing the right view (+X).
    ZY,
    /// Lying on the floor, facing up (+Y).
    XZ,
}

impl SplinePlane {
    pub const ALL: [Self; 3] = [Self::XY, Self::ZY, Self::XZ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::XY => "XY",
            Self::ZY => "ZY",
            Self::XZ => "XZ",
        }
    }

    /// Takes a point drawn in XY (normal +Z) into the plane (a rotation).
    #[must_use]
    pub fn place(self, p: Vec3) -> Vec3 {
        match self {
            Self::XY => p,
            Self::ZY => Vec3::new(p.z, p.y, -p.x),
            Self::XZ => Vec3::new(p.x, p.z, -p.y),
        }
    }
}

/// How an Arc primitive is closed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ArcType {
    /// The open curve.
    #[default]
    Arc,
    /// Closed through the centre.
    Sector,
    /// Closed by the chord.
    Segment,
    /// The band between two radii.
    Ring,
}

impl ArcType {
    pub const ALL: [Self; 4] = [Self::Arc, Self::Sector, Self::Segment, Self::Ring];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Arc => "Arc",
            Self::Sector => "Sector",
            Self::Segment => "Segment",
            Self::Ring => "Ring",
        }
    }
}

/// A parametric spline's shape (sizes in cm, angles in radians), drawn in the
/// XY plane around the origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SplineShape {
    Circle {
        radius: f32,
        /// `Some(radius along Y)`: an ellipse.
        ellipse: Option<f32>,
        /// `Some(inner radius)`: a ring (a second, inner circle).
        ring: Option<f32>,
    },
    Rectangle {
        width: f32,
        height: f32,
        /// `Some(radius)`: rounded corners.
        rounding: Option<f32>,
    },
    Arc {
        kind: ArcType,
        radius: f32,
        inner_radius: f32,
        start: f32,
        end: f32,
    },
    NSide {
        radius: f32,
        sides: u32,
        rounding: Option<f32>,
    },
    Star {
        inner_radius: f32,
        outer_radius: f32,
        twist: f32,
        points: u32,
    },
    Helix {
        start_radius: f32,
        start_angle: f32,
        end_radius: f32,
        end_angle: f32,
        /// 0…1; 0.5 changes the radius evenly.
        radial_bias: f32,
        height: f32,
        height_bias: f32,
        subdivision: u32,
    },
    Flower {
        inner_radius: f32,
        outer_radius: f32,
        petals: u32,
    },
}

/// A spline primitive: its shape, plane, direction and sampling.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplinePrimitive {
    pub shape: SplineShape,
    pub plane: SplinePlane,
    pub reverse: bool,
    pub interpolation: Interpolation,
}

impl SplinePrimitive {
    /// A primitive in the default plane, forwards, sampled adaptively.
    #[must_use]
    pub fn new(shape: SplineShape) -> Self {
        Self {
            shape,
            plane: SplinePlane::XY,
            reverse: false,
            interpolation: Interpolation::default(),
        }
    }

    /// Its control spline, in place.
    #[must_use]
    pub fn spline(&self) -> Spline {
        let spline = self.shape.spline().mapped(|p| self.plane.place(p));
        if self.reverse { spline.reversed() } else { spline }
    }

    /// Its curve cut into pieces.
    #[must_use]
    pub fn polyline(&self) -> Polyline {
        self.spline().sample(self.interpolation)
    }
}

/// Bezier handle length for an arc of `angle` radians on a unit circle.
fn handle(angle: f32) -> f32 {
    4.0 / 3.0 * (angle / 4.0).tan()
}

fn at_angle(radius_x: f32, radius_y: f32, angle: f32) -> Vec3 {
    Vec3::new(radius_x * angle.cos(), radius_y * angle.sin(), 0.0)
}

/// Bezier points along an elliptical arc from `start` to `end` (radians), at
/// most a quarter turn apart, with their tangents.
fn arc_points(radius_x: f32, radius_y: f32, start: f32, end: f32) -> (Vec<Vec3>, Vec<[Vec3; 2]>) {
    let sweep = end - start;
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a few pieces")]
    let pieces = (sweep.abs() / FRAC_PI_2).ceil().clamp(1.0, 64.0) as u32;
    let delta = sweep / f32::from(u16::try_from(pieces).unwrap_or(1));
    let h = handle(delta);
    let mut points = Vec::new();
    let mut tangents = Vec::new();
    for k in 0..=pieces {
        let a = start + delta * f32::from(u16::try_from(k).unwrap_or(0));
        // The derivative of the ellipse, scaled to the handle.
        let d = Vec3::new(-radius_x * a.sin(), radius_y * a.cos(), 0.0) * h;
        points.push(at_angle(radius_x, radius_y, a));
        tangents.push([-d, d]);
    }
    (points, tangents)
}

/// A closed Bezier spline through `corners` (in order) with each corner
/// rounded by `radius` (clamped so neighbouring roundings never overlap).
fn rounded_polygon(corners: &[Vec3], radius: f32) -> Spline {
    let n = corners.len();
    let mut points = Vec::with_capacity(2 * n);
    let mut tangents = Vec::with_capacity(2 * n);
    for i in 0..n {
        let (prev, c, next) = (corners[(i + n - 1) % n], corners[i], corners[(i + 1) % n]);
        let (u, v) = ((c - prev).normalize_or_zero(), (next - c).normalize_or_zero());
        let turn = u.dot(v).clamp(-1.0, 1.0).acos();
        let limit = 0.5 * (c - prev).length().min((next - c).length());
        let along = (radius * (turn * 0.5).tan()).min(limit);
        let r = if turn > 1e-4 { along / (turn * 0.5).tan() } else { 0.0 };
        let h = handle(turn) * r;
        points.push(c - u * along);
        tangents.push([Vec3::ZERO, u * h]);
        points.push(c + v * along);
        tangents.push([-v * h, Vec3::ZERO]);
    }
    let count = u32::try_from(points.len()).unwrap_or(0);
    Spline {
        kind: SplineType::Bezier,
        points,
        tangents,
        segments: vec![Segment { count, closed: true }],
    }
}

/// Schlick's bias: 0.5 leaves `t` as it is, less eases in, more eases out.
fn bias(t: f32, b: f32) -> f32 {
    let b = b.clamp(0.01, 0.99);
    t / ((1.0 / b - 2.0) * (1.0 - t) + 1.0)
}

fn count(n: u32) -> f32 {
    f32::from(u16::try_from(n).unwrap_or(u16::MAX))
}

impl SplineShape {
    pub const MAX_SIDES: u32 = 1000;
    pub const MAX_SUBDIVISION: u32 = 10_000;

    /// Its control spline in the XY plane.
    #[must_use]
    pub fn spline(&self) -> Spline {
        match *self {
            Self::Circle { radius, ellipse, ring } => {
                let circle = |rx: f32, ry: f32| {
                    let (mut points, mut tangents) = arc_points(rx, ry, 0.0, TAU);
                    points.pop(); // the last point repeats the first
                    tangents.pop();
                    let count = u32::try_from(points.len()).unwrap_or(0);
                    Spline {
                        kind: SplineType::Bezier,
                        points,
                        tangents,
                        segments: vec![Segment { count, closed: true }],
                    }
                };
                let (rx, ry) = (radius.max(0.0), ellipse.unwrap_or(radius).max(0.0));
                let mut spline = circle(rx, ry);
                if let Some(inner) = ring {
                    let scale = if rx > 0.0 { inner.max(0.0) / rx } else { 0.0 };
                    spline.append(circle(rx * scale, ry * scale));
                }
                spline
            }
            Self::Rectangle {
                width,
                height,
                rounding,
            } => {
                let (x, y) = (width.max(0.0) * 0.5, height.max(0.0) * 0.5);
                let corners = [
                    Vec3::new(x, y, 0.0),
                    Vec3::new(-x, y, 0.0),
                    Vec3::new(-x, -y, 0.0),
                    Vec3::new(x, -y, 0.0),
                ];
                match rounding {
                    Some(r) if r > 0.0 => rounded_polygon(&corners, r),
                    _ => Spline::new(SplineType::Linear, corners.to_vec(), true),
                }
            }
            Self::Arc {
                kind,
                radius,
                inner_radius,
                start,
                end,
            } => {
                let radius = radius.max(0.0);
                let (points, tangents) = arc_points(radius, radius, start, end);
                let open = |points: Vec<Vec3>, tangents: Vec<[Vec3; 2]>, closed: bool| {
                    let count = u32::try_from(points.len()).unwrap_or(0);
                    Spline {
                        kind: SplineType::Bezier,
                        points,
                        tangents,
                        segments: vec![Segment { count, closed }],
                    }
                };
                match kind {
                    ArcType::Arc => open(points, tangents, false),
                    ArcType::Segment => {
                        let mut tangents = tangents;
                        // Straight chord: no tangent into it.
                        if let Some(first) = tangents.first_mut() {
                            first[0] = Vec3::ZERO;
                        }
                        if let Some(last) = tangents.last_mut() {
                            last[1] = Vec3::ZERO;
                        }
                        open(points, tangents, true)
                    }
                    ArcType::Sector => {
                        let mut points = points;
                        let mut tangents = tangents;
                        if let Some(first) = tangents.first_mut() {
                            first[0] = Vec3::ZERO;
                        }
                        if let Some(last) = tangents.last_mut() {
                            last[1] = Vec3::ZERO;
                        }
                        points.push(Vec3::ZERO);
                        tangents.push([Vec3::ZERO; 2]);
                        open(points, tangents, true)
                    }
                    ArcType::Ring => {
                        let inner = inner_radius.max(0.0);
                        let (inner_points, inner_tangents) = arc_points(inner, inner, start, end);
                        let mut points = points;
                        let mut tangents = tangents;
                        if let Some(last) = tangents.last_mut() {
                            last[1] = Vec3::ZERO;
                        }
                        if let Some(first) = tangents.first_mut() {
                            first[0] = Vec3::ZERO;
                        }
                        // Back along the inner arc.
                        let mut inner_tangents: Vec<[Vec3; 2]> =
                            inner_tangents.into_iter().rev().map(|[l, r]| [r, l]).collect();
                        if let Some(first) = inner_tangents.first_mut() {
                            first[0] = Vec3::ZERO;
                        }
                        if let Some(last) = inner_tangents.last_mut() {
                            last[1] = Vec3::ZERO;
                        }
                        points.extend(inner_points.into_iter().rev());
                        tangents.extend(inner_tangents);
                        open(points, tangents, true)
                    }
                }
            }
            Self::NSide {
                radius,
                sides,
                rounding,
            } => {
                let sides = sides.clamp(3, Self::MAX_SIDES);
                let radius = radius.max(0.0);
                let corners: Vec<Vec3> = (0..sides)
                    .map(|k| at_angle(radius, radius, FRAC_PI_2 + TAU * count(k) / count(sides)))
                    .collect();
                match rounding {
                    Some(r) if r > 0.0 => rounded_polygon(&corners, r),
                    _ => Spline::new(SplineType::Linear, corners, true),
                }
            }
            Self::Star {
                inner_radius,
                outer_radius,
                twist,
                points,
            } => {
                let points = points.clamp(3, Self::MAX_SIDES);
                let step = PI / count(points);
                let corners = (0..2 * points)
                    .map(|k| {
                        let a = FRAC_PI_2 + step * count(k);
                        if k % 2 == 0 {
                            at_angle(outer_radius, outer_radius, a)
                        } else {
                            at_angle(inner_radius, inner_radius, a + twist)
                        }
                    })
                    .collect();
                Spline::new(SplineType::Linear, corners, true)
            }
            Self::Helix {
                start_radius,
                start_angle,
                end_radius,
                end_angle,
                radial_bias,
                height,
                height_bias,
                subdivision,
            } => {
                let n = subdivision.clamp(3, Self::MAX_SUBDIVISION);
                let points = (0..=n)
                    .map(|k| {
                        let t = count(k) / count(n);
                        let a = start_angle + (end_angle - start_angle) * t;
                        let r = start_radius + (end_radius - start_radius) * bias(t, radial_bias);
                        let z = height * bias(t, height_bias);
                        Vec3::new(r * a.cos(), r * a.sin(), z)
                    })
                    .collect();
                Spline::new(SplineType::Akima, points, false)
            }
            Self::Flower {
                inner_radius,
                outer_radius,
                petals,
            } => {
                let petals = petals.clamp(1, Self::MAX_SIDES);
                let step = TAU / count(petals);
                // Round tips, sharp joins between the petals.
                let width = outer_radius * step * 0.45;
                let mut points = Vec::new();
                let mut tangents = Vec::new();
                for k in 0..petals {
                    let tip = FRAC_PI_2 + step * count(k);
                    let along = Vec3::new(-tip.sin(), tip.cos(), 0.0) * width;
                    points.push(at_angle(outer_radius, outer_radius, tip));
                    tangents.push([-along, along]);
                    points.push(at_angle(inner_radius, inner_radius, tip + step * 0.5));
                    tangents.push([Vec3::ZERO; 2]);
                }
                let count = u32::try_from(points.len()).unwrap_or(0);
                Spline {
                    kind: SplineType::Bezier,
                    points,
                    tangents,
                    segments: vec![Segment { count, closed: true }],
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    fn none() -> Interpolation {
        Interpolation::None
    }

    #[test]
    fn a_bezier_circle_stays_on_the_circle() {
        let circle = SplineShape::Circle {
            radius: 200.0,
            ellipse: None,
            ring: None,
        }
        .spline();
        assert_eq!((circle.points.len(), circle.kind), (4, SplineType::Bezier));
        let line = circle.sample(Interpolation::Natural { number: 15 });
        assert_eq!(line.points.len(), 64, "closed: (number + 1) × points");
        for p in &line.points {
            assert!((p.length() - 200.0).abs() < 0.2, "{p} off the circle");
            assert!(p.z.abs() < EPS, "in the XY plane");
        }
        assert_eq!(
            line.segments,
            [Segment {
                count: 64,
                closed: true
            }]
        );
    }

    #[test]
    fn natural_and_uniform_follow_the_documented_counts() {
        let open = Spline::new(
            SplineType::Cubic,
            vec![Vec3::ZERO, Vec3::new(100.0, 50.0, 0.0), Vec3::new(300.0, 0.0, 0.0)],
            false,
        );
        let count = |s: &Spline, i| s.sample(i).points.len();
        assert_eq!(count(&open, Interpolation::Natural { number: 8 }), 9 * 2 + 1);
        assert_eq!(count(&open, Interpolation::Uniform { number: 8 }), 9 * 2 + 1);
        let mut closed = open.clone();
        closed.segments[0].closed = true;
        assert_eq!(count(&closed, Interpolation::Natural { number: 8 }), 9 * 3);
        assert_eq!(count(&closed, Interpolation::Uniform { number: 8 }), 9 * 3);

        // Uniform: equal steps along the curve, from the first to the last point.
        let line = open.sample(Interpolation::Uniform { number: 8 });
        let steps: Vec<f32> = line.points.windows(2).map(|w| w[0].distance(w[1])).collect();
        let mean = steps.iter().sum::<f32>() / 18.0;
        assert!(steps.iter().all(|s| (s - mean).abs() < mean * 0.05), "{steps:?}");
        assert!(line.points[0].abs_diff_eq(Vec3::ZERO, EPS));
        assert!(line.points[18].abs_diff_eq(Vec3::new(300.0, 0.0, 0.0), 0.01));
    }

    #[test]
    fn adaptive_adds_points_only_where_the_curve_turns() {
        let square = SplineShape::Rectangle {
            width: 100.0,
            height: 100.0,
            rounding: None,
        }
        .spline();
        assert_eq!(
            square.sample(Interpolation::default()).points.len(),
            4,
            "straight sides"
        );

        let circle = SplineShape::Circle {
            radius: 100.0,
            ellipse: None,
            ring: None,
        }
        .spline();
        let line = circle.sample(Interpolation::Adaptive {
            angle: 5f32.to_radians(),
        });
        // 360° in pieces of at most 5°: 72 or a few more.
        assert!((72..=80).contains(&line.points.len()), "{}", line.points.len());
        let coarse = circle.sample(Interpolation::Adaptive {
            angle: 30f32.to_radians(),
        });
        assert!(coarse.points.len() < line.points.len());
    }

    #[test]
    fn subdivided_bounds_the_length_of_pieces() {
        let square = SplineShape::Rectangle {
            width: 100.0,
            height: 100.0,
            rounding: None,
        }
        .spline();
        let line = square.sample(Interpolation::Subdivided {
            angle: 5f32.to_radians(),
            max_length: 10.0,
        });
        assert_eq!(line.points.len(), 40);
        let mut ring = line.points.clone();
        ring.push(line.points[0]);
        assert!(ring.windows(2).all(|w| w[0].distance(w[1]) <= 10.0 + EPS));
    }

    #[test]
    fn rounded_rectangles_keep_their_size_and_round_their_corners() {
        let spline = SplineShape::Rectangle {
            width: 400.0,
            height: 200.0,
            rounding: Some(50.0),
        }
        .spline();
        assert_eq!(spline.points.len(), 8);
        let line = spline.sample(Interpolation::Natural { number: 8 });
        let (min, max) = line.bounds().unwrap();
        assert!(min.abs_diff_eq(Vec3::new(-200.0, -100.0, 0.0), 0.01));
        assert!(max.abs_diff_eq(Vec3::new(200.0, 100.0, 0.0), 0.01));
        // The corner region is cut: no point near the sharp corner.
        assert!(
            line.points
                .iter()
                .all(|p| p.distance(Vec3::new(200.0, 100.0, 0.0)) > 20.0)
        );
        // A radius larger than the sides allow is clamped (half the short side).
        let pill = SplineShape::Rectangle {
            width: 400.0,
            height: 200.0,
            rounding: Some(500.0),
        }
        .spline()
        .sample(Interpolation::default());
        let (min, max) = pill.bounds().unwrap();
        assert!(min.abs_diff_eq(Vec3::new(-200.0, -100.0, 0.0), 0.5) && max.y <= 100.0 + 0.5);
    }

    #[test]
    fn arcs_open_or_close_as_their_type_says() {
        let arc = |kind| {
            SplineShape::Arc {
                kind,
                radius: 200.0,
                inner_radius: 100.0,
                start: 0.0,
                end: FRAC_PI_2,
            }
            .spline()
        };
        let open = arc(ArcType::Arc);
        assert!(!open.segments[0].closed);
        let line = open.sample(none());
        assert!(line.points[0].abs_diff_eq(Vec3::new(200.0, 0.0, 0.0), EPS));
        assert!(
            line.points
                .last()
                .unwrap()
                .abs_diff_eq(Vec3::new(0.0, 200.0, 0.0), 0.01)
        );
        let sector = arc(ArcType::Sector);
        assert!(sector.segments[0].closed && sector.points.contains(&Vec3::ZERO));
        assert!(arc(ArcType::Segment).segments[0].closed);
        let ring = arc(ArcType::Ring);
        assert_eq!(ring.segments.len(), 1);
        assert!(ring.points.iter().any(|p| (p.length() - 100.0).abs() < EPS));
    }

    #[test]
    fn rings_are_two_segments_and_reverse_runs_backwards() {
        let ring = SplinePrimitive::new(SplineShape::Circle {
            radius: 200.0,
            ellipse: None,
            ring: Some(100.0),
        });
        let line = ring.polyline();
        assert_eq!(line.segments.len(), 2);
        let inner: Vec<_> = line.runs().nth(1).unwrap().0.to_vec();
        assert!(inner.iter().all(|p| (p.length() - 100.0).abs() < 0.2));

        let forward = SplinePrimitive::new(SplineShape::NSide {
            radius: 100.0,
            sides: 5,
            rounding: None,
        });
        let backward = SplinePrimitive {
            reverse: true,
            ..forward
        };
        let (a, b) = (forward.spline().points, backward.spline().points);
        assert_eq!(a.iter().rev().copied().collect::<Vec<_>>(), b);
    }

    #[test]
    fn planes_turn_the_shape() {
        let star = |plane| {
            SplinePrimitive {
                plane,
                ..SplinePrimitive::new(SplineShape::Star {
                    inner_radius: 50.0,
                    outer_radius: 100.0,
                    twist: 0.0,
                    points: 5,
                })
            }
            .spline()
        };
        assert!(
            star(SplinePlane::XZ).points.iter().all(|p| p.y.abs() < EPS),
            "on the floor"
        );
        assert!(star(SplinePlane::ZY).points.iter().all(|p| p.x.abs() < EPS));
        // The first (top) point of the XY star goes to −Z on the floor.
        assert!(star(SplinePlane::XZ).points[0].abs_diff_eq(Vec3::new(0.0, 0.0, -100.0), EPS));
        assert_eq!(star(SplinePlane::XY).points.len(), 10);
    }

    #[test]
    fn a_helix_rises_along_the_normal_and_turns() {
        let helix = SplineShape::Helix {
            start_radius: 200.0,
            start_angle: 0.0,
            end_radius: 200.0,
            end_angle: 4.0 * PI,
            radial_bias: 0.5,
            height: 200.0,
            height_bias: 0.5,
            subdivision: 100,
        }
        .spline();
        assert_eq!(helix.points.len(), 101);
        assert!(helix.points[0].abs_diff_eq(Vec3::new(200.0, 0.0, 0.0), EPS));
        assert!(helix.points[100].abs_diff_eq(Vec3::new(200.0, 0.0, 200.0), 0.01));
        assert!((helix.points[50].z - 100.0).abs() < 0.01, "an even rise at 50 %");
        let line = helix.sample(Interpolation::default());
        assert!(line.points.iter().all(|p| (p.truncate().length() - 200.0).abs() < 1.0));
    }

    #[test]
    fn akima_does_not_overshoot_and_bsplines_end_on_their_ends() {
        // A step: Cubic overshoots past 100, Akima stays inside.
        let step: Vec<Vec3> = [0.0, 0.0, 0.0, 100.0, 100.0, 100.0]
            .iter()
            .enumerate()
            .map(|(i, &y)| Vec3::new(f32::from(u8::try_from(i).unwrap()) * 50.0, y, 0.0))
            .collect();
        let peak = |kind| {
            Spline::new(kind, step.clone(), false)
                .sample(Interpolation::Natural { number: 16 })
                .points
                .iter()
                .fold(f32::MIN, |m, p| m.max(p.y))
        };
        assert!(peak(SplineType::Cubic) > 100.5, "cubic overshoots");
        assert!(peak(SplineType::Akima) <= 100.0 + EPS, "akima does not");

        let b = Spline::new(SplineType::BSpline, step.clone(), false).sample(none());
        let line = Spline::new(SplineType::BSpline, step.clone(), false).sample(Interpolation::default());
        assert!(line.points[0].abs_diff_eq(step[0], EPS));
        assert!(line.points.last().unwrap().abs_diff_eq(step[5], 0.01));
        assert_eq!(b.points.len(), 7 + 1, "spans + 1 for an open B-spline");
    }

    #[test]
    fn removing_points_keeps_the_rest_in_order_and_drops_empty_segments() {
        let mut spline = Spline::new(SplineType::Linear, vec![Vec3::X, Vec3::Y, Vec3::Z], true);
        spline.append(Spline::new(SplineType::Linear, vec![Vec3::ONE, Vec3::NEG_ONE], false));
        let remove: crate::BitSet = [1, 3, 4].into_iter().collect();
        let (left, map) = spline.without_points(&remove);
        assert_eq!(left.points, [Vec3::X, Vec3::Z]);
        assert_eq!(
            left.segments,
            [Segment { count: 2, closed: true }],
            "the emptied segment is gone"
        );
        assert_eq!(map, [Some(0), None, Some(1), None, None]);
        let (single, _) = left.without_points(&[0].into_iter().collect());
        assert_eq!(
            single.segments,
            [Segment {
                count: 1,
                closed: false
            }]
        );
    }

    #[test]
    fn smooth_tangents_fill_only_missing_ones() {
        let mut spline = Spline::new(
            SplineType::Linear,
            vec![Vec3::ZERO, Vec3::new(100.0, 100.0, 0.0), Vec3::new(200.0, 0.0, 0.0)],
            false,
        );
        spline.tangents[2] = [Vec3::new(-5.0, 0.0, 0.0), Vec3::new(5.0, 0.0, 0.0)];
        spline.smooth_tangents();
        let [left, right] = spline.tangents[1];
        assert!(
            right.abs_diff_eq(Vec3::new(200.0 / 6.0, 0.0, 0.0), EPS),
            "level at the top: {right}"
        );
        assert_eq!(left, -right, "in line");
        assert_eq!(spline.tangents[2][1], Vec3::new(5.0, 0.0, 0.0), "set tangents stay");
        assert!(spline.tangents[0][1].x > 0.0, "the first point points onwards");
        spline.kind = SplineType::Bezier;
        let line = spline.sample(Interpolation::Natural { number: 8 });
        assert!(
            line.points.iter().all(|p| p.y <= 100.5),
            "a smooth arch through the points"
        );
    }

    #[test]
    fn single_points_and_empty_splines_sample_safely() {
        let point = Spline::new(SplineType::Bezier, vec![Vec3::ONE], true);
        assert_eq!(point.sample(Interpolation::default()).points, [Vec3::ONE]);
        let empty = Spline::new(SplineType::Linear, Vec::new(), false);
        let line = empty.sample(Interpolation::default());
        assert!(line.points.is_empty() && line.bounds().is_none());
        let zero = SplineShape::Circle {
            radius: 0.0,
            ellipse: None,
            ring: None,
        }
        .spline()
        .sample(Interpolation::default());
        assert!(zero.points.iter().all(|p| p.length() < EPS));
    }
}
