//! Planes, points and the exact predicates the Boolean is built on.
//!
//! Every plane comes from the inputs: a triangle's own plane (its *support*)
//! or a plane through one of its edges (*edge planes*, which bound it within
//! its support). Every point is either an **input point** (its `f64`
//! coordinates) or, when the inputs cross, a point made by the cut:
//!
//! * [`Method::Exact`] keeps a made point **implicit**, as the meeting point
//!   of three planes, so every later test on it is exact (the plane-based
//!   representation);
//! * [`Method::Rounded`] rounds it to `f64` coordinates at once and treats it
//!   as an input point afterwards — exact tests on rounded points, the usual
//!   way, which the spike compares against.
//!
//! Signs are found with [`Interval`]s first and with [`Expansion`]s only when
//! an interval cannot decide.

use std::collections::HashMap;

use glam::DVec3;

use crate::exact::{Expansion, Interval, Num, cross, det3, dot};

use super::Method;

/// How a plane was made, from input coordinates.
#[derive(Clone, Copy, Debug)]
pub(super) enum PlaneDef {
    /// Through three points (a triangle's own plane; normal by the right hand).
    Support([DVec3; 3]),
    /// Through two points and along an axis (an edge plane, `axis` 0 – 2).
    Edge([DVec3; 2], usize),
}

/// A plane `n·x + d = 0` as `[nx, ny, nz, d]`.
fn coefficients<N: Num>(def: &PlaneDef) -> [N; 4] {
    let v = |p: DVec3| [N::from_f64(p.x), N::from_f64(p.y), N::from_f64(p.z)];
    let sub = |a: &[N; 3], b: &[N; 3]| [a[0].sub(&b[0]), a[1].sub(&b[1]), a[2].sub(&b[2])];
    let (n, a) = match *def {
        PlaneDef::Support([a, b, c]) => {
            let a = v(a);
            (cross(&sub(&v(b), &a), &sub(&v(c), &a)), a)
        }
        PlaneDef::Edge([a, b], axis) => {
            let a = v(a);
            let mut along = [N::from_f64(0.0), N::from_f64(0.0), N::from_f64(0.0)];
            along[axis] = N::from_f64(1.0);
            (cross(&sub(&v(b), &a), &along), a)
        }
    };
    let d = N::from_f64(0.0).sub(&dot(&n, &a));
    let [x, y, z] = n;
    [x, y, z, d]
}

struct PlaneData {
    def: PlaneDef,
    interval: [Interval; 4],
    exact: Option<Box<[Expansion; 4]>>,
}

/// Where a point is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PointKind {
    /// Input coordinates (or, with [`Method::Rounded`], a made point rounded).
    Explicit,
    /// The meeting point of three planes (sorted ids).
    Implicit([u32; 3]),
}

struct PointData {
    kind: PointKind,
    /// The coordinates (rounded for an implicit point).
    approx: DVec3,
    /// An implicit point's homogeneous coordinates `x / w`, as intervals.
    interval: Option<([Interval; 3], Interval)>,
    exact: Option<Box<([Expansion; 3], Expansion)>>,
    /// Made by a cut (not an input point).
    made: bool,
}

/// The planes and points of one Boolean, and the predicates on them.
pub(super) struct Kernel {
    method: Method,
    planes: Vec<PlaneData>,
    points: Vec<PointData>,
    implicit: HashMap<[u32; 3], u32>,
    explicit: HashMap<[u64; 3], u32>,
    /// How many signs the intervals could not decide (for the spike's report).
    pub(super) exact_calls: usize,
}

fn bits(p: DVec3) -> [u64; 3] {
    // +0 and −0 are the same point.
    [p.x + 0.0, p.y + 0.0, p.z + 0.0].map(f64::to_bits)
}

/// The homogeneous coordinates of the meeting point of three planes, by
/// Cramer's rule: `x = X / w`.
fn meeting<N: Num>(p: &[N; 4], q: &[N; 4], r: &[N; 4]) -> ([N; 3], N) {
    let n = |c: &[N; 4]| [c[0].clone(), c[1].clone(), c[2].clone()];
    let (np, nq, nr) = (n(p), n(q), n(r));
    let w = det3(&np, &nq, &nr);
    // X = −(d_p (n_q × n_r) + d_q (n_r × n_p) + d_r (n_p × n_q)).
    let terms = [
        (cross(&nq, &nr), &p[3]),
        (cross(&nr, &np), &q[3]),
        (cross(&np, &nq), &r[3]),
    ];
    let zero = N::from_f64(0.0);
    let mut x = [zero.clone(), zero.clone(), zero];
    for (axis, slot) in x.iter_mut().enumerate() {
        for (normal, d) in &terms {
            *slot = slot.sub(&normal[axis].mul(d));
        }
    }
    (x, w)
}

/// The sign of `n·X + d·w` for plane `c`, and of `w`.
fn side_of<N: Num>(c: &[N; 4], x: &[N; 3], w: &N) -> N {
    c[0].mul(&x[0])
        .add(&c[1].mul(&x[1]))
        .add(&c[2].mul(&x[2]))
        .add(&c[3].mul(w))
}

impl Kernel {
    pub(super) fn new(method: Method) -> Self {
        Self {
            method,
            planes: Vec::new(),
            points: Vec::new(),
            implicit: HashMap::new(),
            explicit: HashMap::new(),
            exact_calls: 0,
        }
    }

    pub(super) fn method(&self) -> Method {
        self.method
    }

    pub(super) fn add_plane(&mut self, def: PlaneDef) -> u32 {
        let id = crate::u32_index(self.planes.len());
        self.planes.push(PlaneData {
            def,
            interval: coefficients(&def),
            exact: None,
        });
        id
    }

    /// An input point (the same id for the same coordinates).
    pub(super) fn input_point(&mut self, p: DVec3) -> u32 {
        self.explicit_point(p, false)
    }

    fn explicit_point(&mut self, p: DVec3, made: bool) -> u32 {
        let key = bits(p);
        if let Some(&id) = self.explicit.get(&key) {
            return id;
        }
        let id = crate::u32_index(self.points.len());
        self.points.push(PointData {
            kind: PointKind::Explicit,
            approx: p + DVec3::ZERO,
            interval: None,
            exact: None,
            made,
        });
        self.explicit.insert(key, id);
        id
    }

    pub(super) fn point_count(&self) -> usize {
        self.points.len()
    }

    pub(super) fn approx(&self, point: u32) -> DVec3 {
        self.points[crate::us(point)].approx
    }

    pub(super) fn is_made(&self, point: u32) -> bool {
        self.points[crate::us(point)].made
    }

    pub(super) fn kind(&self, point: u32) -> PointKind {
        self.points[crate::us(point)].kind
    }

    fn plane_exact(&mut self, plane: u32) -> [Expansion; 4] {
        let data = &mut self.planes[crate::us(plane)];
        if let Some(exact) = &data.exact {
            return (**exact).clone();
        }
        let exact = coefficients::<Expansion>(&data.def);
        data.exact = Some(Box::new(exact.clone()));
        exact
    }

    /// The point where the line of planes `a` and `b` meets plane `c` (the
    /// three planes meet in one point: the caller knows the line crosses `c`).
    pub(super) fn meet(&mut self, a: u32, b: u32, c: u32) -> u32 {
        let mut key = [a, b, c];
        key.sort_unstable();
        if let Some(&id) = self.implicit.get(&key) {
            return id;
        }
        let [p, q, r] = key.map(|k| self.planes[crate::us(k)].interval);
        let (x, w) = meeting(&p, &q, &r);
        let decided = w.sign().is_some_and(|s| s != 0)
            && x.iter()
                .chain([&w])
                .all(|v| v.relative_width() < 1e-10 || v.sign() == Some(0));
        let approx = if decided && self.method == Method::Exact {
            DVec3::new(x[0].mid(), x[1].mid(), x[2].mid()) / w.mid()
        } else {
            let [p, q, r] = key.map(|k| self.plane_exact(k));
            let (x, w) = meeting(&p, &q, &r);
            DVec3::new(x[0].approx(), x[1].approx(), x[2].approx()) / w.approx()
        };
        if self.method == Method::Rounded {
            let id = self.explicit_point(approx, true);
            self.implicit.insert(key, id);
            return id;
        }
        let id = crate::u32_index(self.points.len());
        self.points.push(PointData {
            kind: PointKind::Implicit(key),
            approx,
            interval: Some((x, w)),
            exact: None,
            made: true,
        });
        self.implicit.insert(key, id);
        id
    }

    fn point_exact(&mut self, point: u32) -> ([Expansion; 3], Expansion) {
        let data = &self.points[crate::us(point)];
        if let Some(exact) = &data.exact {
            return (**exact).clone();
        }
        let exact = match data.kind {
            PointKind::Explicit => (
                data.approx.to_array().map(Expansion::from_f64),
                Expansion::from_f64(1.0),
            ),
            PointKind::Implicit(key) => {
                let [p, q, r] = key.map(|k| self.plane_exact(k));
                meeting(&p, &q, &r)
            }
        };
        self.points[crate::us(point)].exact = Some(Box::new(exact.clone()));
        exact
    }

    fn point_interval(&self, point: u32) -> ([Interval; 3], Interval) {
        let data = &self.points[crate::us(point)];
        data.interval
            .unwrap_or_else(|| (data.approx.to_array().map(Interval::from_f64), Interval::from_f64(1.0)))
    }

    /// −1, 0 or 1: which side of `plane` the point lies on (the side its
    /// normal points to is 1).
    pub(super) fn side(&mut self, point: u32, plane: u32) -> i8 {
        if let PointKind::Implicit(key) = self.points[crate::us(point)].kind
            && key.contains(&plane)
        {
            return 0;
        }
        let (x, w) = self.point_interval(point);
        let c = self.planes[crate::us(plane)].interval;
        if let (Some(s), Some(ws)) = (side_of(&c, &x, &w).sign(), w.sign())
            && ws != 0
        {
            return s * ws;
        }
        self.exact_calls += 1;
        let (x, w) = self.point_exact(point);
        let c = self.plane_exact(plane);
        side_of(&c, &x, &w).sign() * w.sign()
    }

    /// Whether two points are the same point.
    pub(super) fn same(&mut self, a: u32, b: u32) -> bool {
        if a == b {
            return true;
        }
        let (ka, kb) = (self.kind(a), self.kind(b));
        if ka == PointKind::Explicit && kb == PointKind::Explicit {
            // Distinct explicit ids have distinct coordinates.
            return false;
        }
        let ((xa, wa), (xb, wb)) = (self.point_interval(a), self.point_interval(b));
        let differs = (0..3).any(|i| xa[i].mul(&wb).sub(&xb[i].mul(&wa)).sign().is_some_and(|s| s != 0));
        if differs {
            return false;
        }
        self.exact_calls += 1;
        let ((xa, wa), (xb, wb)) = (self.point_exact(a), self.point_exact(b));
        (0..3).all(|i| xa[i].mul(&wb).sub(&xb[i].mul(&wa)).sign() == 0)
    }

    /// −1, 0 or 1: the sign of `a`'s coordinate minus `b`'s along `axis`.
    pub(super) fn compare(&mut self, a: u32, b: u32, axis: usize) -> i8 {
        if a == b {
            return 0;
        }
        let ((xa, wa), (xb, wb)) = (self.point_interval(a), self.point_interval(b));
        let difference = xa[axis].mul(&wb).sub(&xb[axis].mul(&wa));
        if let (Some(s), Some(w)) = (difference.sign(), wa.mul(&wb).sign())
            && w != 0
        {
            return s * w;
        }
        self.exact_calls += 1;
        let ((xa, wa), (xb, wb)) = (self.point_exact(a), self.point_exact(b));
        xa[axis].mul(&wb).sub(&xb[axis].mul(&wa)).sign() * wa.mul(&wb).sign()
    }

    /// Whether a plane has no normal (its triangle's corners are collinear).
    pub(super) fn is_degenerate(&mut self, plane: u32) -> bool {
        let c = self.planes[crate::us(plane)].interval;
        if c[..3].iter().any(|v| v.sign().is_some_and(|s| s != 0)) {
            return false;
        }
        let c = self.plane_exact(plane);
        c[..3].iter().all(|v| v.sign() == 0)
    }

    /// The plane's normal, approximately.
    pub(super) fn normal(&self, plane: u32) -> DVec3 {
        let c = self.planes[crate::us(plane)].interval;
        DVec3::new(c[0].mid(), c[1].mid(), c[2].mid())
    }

    /// [`side`](Self::side) for classification: exact with
    /// [`Method::Exact`]; with [`Method::Rounded`] a point within `tolerance`
    /// of the plane counts as on it (rounded points are almost never exactly
    /// on anything).
    pub(super) fn side_within(&mut self, point: u32, plane: u32, tolerance: f64) -> i8 {
        if self.method == Method::Exact {
            return self.side(point, plane);
        }
        let c = self.planes[crate::us(plane)].interval;
        let n = DVec3::new(c[0].mid(), c[1].mid(), c[2].mid());
        let distance = (n.dot(self.approx(point)) + c[3].mid()) / n.length().max(f64::MIN_POSITIVE);
        if distance.abs() <= tolerance {
            0
        } else {
            self.side(point, plane)
        }
    }

    /// The exact sign of the dot product of two planes' normals.
    pub(super) fn normals_agree(&mut self, a: u32, b: u32) -> i8 {
        let n = |c: [Interval; 4]| [c[0], c[1], c[2]];
        let (pa, pb) = (self.planes[crate::us(a)].interval, self.planes[crate::us(b)].interval);
        if let Some(s) = dot(&n(pa), &n(pb)).sign() {
            return s;
        }
        let (pa, pb) = (self.plane_exact(a), self.plane_exact(b));
        let n = |c: [Expansion; 4]| {
            let [x, y, z, _] = c;
            [x, y, z]
        };
        dot(&n(pa), &n(pb)).sign()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn support(kernel: &mut Kernel, a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> u32 {
        kernel.add_plane(PlaneDef::Support([a, b, c].map(DVec3::from_array)))
    }

    #[test]
    fn three_planes_meet_where_they_should_and_sides_are_exact() {
        for method in [Method::Exact, Method::Rounded] {
            let mut k = Kernel::new(method);
            // x = 1, y = 2, z = 3 as planes through points.
            let px = support(&mut k, [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [1.0, 0.0, 1.0]);
            let py = support(&mut k, [0.0, 2.0, 0.0], [0.0, 2.0, 1.0], [1.0, 2.0, 0.0]);
            let pz = support(&mut k, [0.0, 0.0, 3.0], [1.0, 0.0, 3.0], [0.0, 1.0, 3.0]);
            let p = k.meet(px, py, pz);
            assert!(k.approx(p).distance(DVec3::new(1.0, 2.0, 3.0)) < 1e-12);
            for plane in [px, py, pz] {
                assert_eq!(k.side(p, plane), 0);
            }
            let input = k.input_point(DVec3::new(1.0, 2.0, 3.0));
            assert!(k.same(p, input));
            let off = k.input_point(DVec3::new(1.0, 2.0, 3.0f64.next_up()));
            assert_eq!(k.side(off, pz), 1, "one ulp above z = 3");
            assert!(!k.same(p, off));
        }
    }

    #[test]
    fn a_meeting_point_on_a_skew_plane_is_exactly_on_it() {
        // A plane through awkward points; the line x = 1/3-ish, y = 0.1 meets
        // it at a point no f64 can hold. Exact: on the plane. Rounded: off.
        let mut k = Kernel::new(Method::Exact);
        let skew = support(&mut k, [0.1, 0.7, 0.3], [1.9, 0.2, 0.55], [0.4, 1.3, 2.1]);
        let a = support(
            &mut k,
            [1.0 / 3.0, 0.0, 0.0],
            [1.0 / 3.0, 1.0, 0.0],
            [1.0 / 3.0, 0.0, 1.0],
        );
        let b = support(&mut k, [0.0, 0.1, 0.0], [0.0, 0.1, 1.0], [1.0, 0.1, 0.0]);
        let p = k.meet(a, b, skew);
        // The same plane made again from its points in another order: the
        // point lies on it, which only exact arithmetic can tell.
        let again = support(&mut k, [1.9, 0.2, 0.55], [0.4, 1.3, 2.1], [0.1, 0.7, 0.3]);
        assert_eq!(k.side(p, again), 0);
        assert!(k.exact_calls > 0, "the filter cannot decide a point on the plane");
        let mut rounded = Kernel::new(Method::Rounded);
        let skew = support(&mut rounded, [0.1, 0.7, 0.3], [1.9, 0.2, 0.55], [0.4, 1.3, 2.1]);
        let a = support(
            &mut rounded,
            [1.0 / 3.0, 0.0, 0.0],
            [1.0 / 3.0, 1.0, 0.0],
            [1.0 / 3.0, 0.0, 1.0],
        );
        let b = support(&mut rounded, [0.0, 0.1, 0.0], [0.0, 0.1, 1.0], [1.0, 0.1, 0.0]);
        let q = rounded.meet(a, b, skew);
        assert!(
            rounded.approx(q).distance(k.approx(p)) < 1e-12,
            "rounded, it is next to the exact point"
        );
        assert_eq!(rounded.kind(q), PointKind::Explicit);
    }
}
