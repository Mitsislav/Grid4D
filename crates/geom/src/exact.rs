//! Exact signs of polynomial expressions in `f64` inputs, for geometric
//! predicates that must never be wrong (the mesh Boolean, `boolean`).
//!
//! Two number types share one interface ([`Num`]), so a formula is written
//! once:
//!
//! * [`Interval`] — a lower and an upper bound, each operation rounded
//!   outwards by one unit in the last place. Fast; its sign is certain when
//!   the interval does not contain zero (the *filter*).
//! * [`Expansion`] — a sum of non-overlapping `f64` components (Shewchuk,
//!   *Adaptive precision floating-point arithmetic and fast robust geometric
//!   predicates*, 1997): sums and products are exact, so the sign is always
//!   right. Slow; used only when the filter cannot decide.
//!
//! Inputs are finite `f64`s (Grid4D's `f32` coordinates widen exactly).
//! Exactness holds while no intermediate value underflows below about
//! 1e-300 — coordinates and their differences are either zero or far larger.

/// What both number types can do.
pub(crate) trait Num: Clone {
    fn from_f64(value: f64) -> Self;
    fn add(&self, other: &Self) -> Self;
    fn sub(&self, other: &Self) -> Self;
    fn mul(&self, other: &Self) -> Self;
}

/// `a + b` as the rounded sum and its exact error.
fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let x = a + b;
    let b_virtual = x - a;
    let a_virtual = x - b_virtual;
    (x, (a - a_virtual) + (b - b_virtual))
}

/// `a × b` as the rounded product and its exact error (a fused multiply-add
/// is exact).
fn two_product(a: f64, b: f64) -> (f64, f64) {
    let x = a * b;
    (x, a.mul_add(b, -x))
}

/// An exact real number: the sum of its components, which do not overlap and
/// grow in magnitude; no component is zero (zero is the empty sum).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Expansion(Vec<f64>);

impl Expansion {
    /// The expansion plus one `f64` (Shewchuk's grow-expansion, zeros removed).
    fn grow(&self, b: f64) -> Self {
        let mut out = Vec::with_capacity(self.0.len() + 1);
        let mut q = b;
        for &e in &self.0 {
            let (sum, error) = two_sum(q, e);
            q = sum;
            if error != 0.0 {
                out.push(error);
            }
        }
        if q != 0.0 {
            out.push(q);
        }
        Self(out)
    }

    /// The expansion times one `f64` (scale-expansion, zeros removed).
    fn scale(&self, b: f64) -> Self {
        let mut out = Vec::with_capacity(self.0.len() * 2);
        let Some((&first, rest)) = self.0.split_first() else {
            return Self::default();
        };
        let (mut q, error) = two_product(first, b);
        if error != 0.0 {
            out.push(error);
        }
        for &e in rest {
            let (product, product_error) = two_product(e, b);
            let (sum, error) = two_sum(q, product_error);
            if error != 0.0 {
                out.push(error);
            }
            let (sum, error) = two_sum(product, sum);
            if error != 0.0 {
                out.push(error);
            }
            q = sum;
        }
        if q != 0.0 {
            out.push(q);
        }
        Self(out)
    }

    fn negated(&self) -> Self {
        Self(self.0.iter().map(|e| -e).collect())
    }

    /// −1, 0 or 1: the sign of the largest component.
    pub(crate) fn sign(&self) -> i8 {
        match self.0.last() {
            Some(&e) if e > 0.0 => 1,
            Some(_) => -1,
            None => 0,
        }
    }

    /// The nearest `f64`, nearly (the components summed from the smallest).
    pub(crate) fn approx(&self) -> f64 {
        self.0.iter().sum()
    }
}

impl Num for Expansion {
    fn from_f64(value: f64) -> Self {
        if value == 0.0 {
            Self::default()
        } else {
            Self(vec![value])
        }
    }

    fn add(&self, other: &Self) -> Self {
        // Grow the longer by the shorter's components.
        let (long, short) = if self.0.len() >= other.0.len() {
            (self, other)
        } else {
            (other, self)
        };
        short.0.iter().fold(long.clone(), |sum, &e| sum.grow(e))
    }

    fn sub(&self, other: &Self) -> Self {
        self.add(&other.negated())
    }

    fn mul(&self, other: &Self) -> Self {
        let (long, short) = if self.0.len() >= other.0.len() {
            (self, other)
        } else {
            (other, self)
        };
        short.0.iter().fold(Self::default(), |sum, &e| sum.add(&long.scale(e)))
    }
}

/// A closed interval of reals known to contain the true value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Interval {
    pub(crate) lo: f64,
    pub(crate) hi: f64,
}

impl Interval {
    const WHOLE: Self = Self {
        lo: f64::NEG_INFINITY,
        hi: f64::INFINITY,
    };

    fn widened(lo: f64, hi: f64) -> Self {
        if lo.is_nan() || hi.is_nan() {
            return Self::WHOLE;
        }
        Self {
            lo: lo.next_down(),
            hi: hi.next_up(),
        }
    }

    /// The sign when the interval decides it.
    pub(crate) fn sign(self) -> Option<i8> {
        if self.lo > 0.0 {
            Some(1)
        } else if self.hi < 0.0 {
            Some(-1)
        } else if self.lo == 0.0 && self.hi == 0.0 {
            Some(0)
        } else {
            None
        }
    }

    pub(crate) fn mid(self) -> f64 {
        0.5 * self.lo + 0.5 * self.hi
    }

    /// The interval's width relative to its largest bound (0 for a point).
    pub(crate) fn relative_width(self) -> f64 {
        let size = self.lo.abs().max(self.hi.abs());
        if size == 0.0 { 0.0 } else { (self.hi - self.lo) / size }
    }
}

impl Num for Interval {
    fn from_f64(value: f64) -> Self {
        Self { lo: value, hi: value }
    }

    fn add(&self, other: &Self) -> Self {
        Self::widened(self.lo + other.lo, self.hi + other.hi)
    }

    fn sub(&self, other: &Self) -> Self {
        Self::widened(self.lo - other.hi, self.hi - other.lo)
    }

    fn mul(&self, other: &Self) -> Self {
        let products = [
            self.lo * other.lo,
            self.lo * other.hi,
            self.hi * other.lo,
            self.hi * other.hi,
        ];
        if products.iter().any(|p| p.is_nan()) {
            return Self::WHOLE;
        }
        let lo = products.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = products.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Self::widened(lo, hi)
    }
}

/// `a·b` of two 3-vectors.
pub(crate) fn dot<N: Num>(a: &[N; 3], b: &[N; 3]) -> N {
    a[0].mul(&b[0]).add(&a[1].mul(&b[1])).add(&a[2].mul(&b[2]))
}

/// `a × b` of two 3-vectors.
pub(crate) fn cross<N: Num>(a: &[N; 3], b: &[N; 3]) -> [N; 3] {
    [
        a[1].mul(&b[2]).sub(&a[2].mul(&b[1])),
        a[2].mul(&b[0]).sub(&a[0].mul(&b[2])),
        a[0].mul(&b[1]).sub(&a[1].mul(&b[0])),
    ]
}

/// The determinant of the 3 × 3 matrix with rows `a`, `b`, `c`.
pub(crate) fn det3<N: Num>(a: &[N; 3], b: &[N; 3], c: &[N; 3]) -> N {
    dot(a, &cross(b, c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ex(v: f64) -> Expansion {
        Expansion::from_f64(v)
    }

    #[test]
    fn sums_and_products_are_exact() {
        // 1 + 2^-80 − 1 is 2^-80 exactly, which f64 loses.
        let tiny = 2f64.powi(-80);
        let e = ex(1.0).add(&ex(tiny)).sub(&ex(1.0));
        assert_eq!(e.approx(), tiny);
        assert_eq!(e.sign(), 1);
        // (1 + 2^-30)² − 1 − 2^-29 = 2^-60.
        let a = ex(1.0).add(&ex(2f64.powi(-30)));
        let d = a.mul(&a).sub(&ex(1.0)).sub(&ex(2f64.powi(-29)));
        assert_eq!(d.approx(), 2f64.powi(-60));
        assert_eq!(ex(3.0).sub(&ex(3.0)).sign(), 0);
        assert_eq!(ex(-2.0).mul(&ex(5.0)).sign(), -1);
    }

    #[test]
    fn the_orientation_of_nearly_collinear_points_is_exact() {
        // Points on a line y = x, the middle one nudged by one ulp: f64
        // arithmetic gets the sign wrong or zero, expansions do not.
        let p = [0.5f64, 0.5];
        let q = [12.0f64, 12.0];
        let r = [24.0f64.next_up(), 24.0];
        let orient = |n: fn(f64) -> Expansion| {
            let (px, py, qx, qy, rx, ry) = (n(p[0]), n(p[1]), n(q[0]), n(q[1]), n(r[0]), n(r[1]));
            qx.sub(&px).mul(&ry.sub(&py)).sub(&qy.sub(&py).mul(&rx.sub(&px)))
        };
        assert_eq!(orient(ex).sign(), -1, "r lies just right of pq");
    }

    #[test]
    fn intervals_contain_the_value_and_decide_clear_signs() {
        let third = Interval::from_f64(1.0).mul(&Interval::from_f64(1.0 / 3.0));
        assert!(third.lo <= 1.0 / 3.0 && 1.0 / 3.0 <= third.hi);
        assert_eq!(Interval::from_f64(2.0).sub(&Interval::from_f64(1.0)).sign(), Some(1));
        let zero = Interval::from_f64(0.1)
            .add(&Interval::from_f64(0.2))
            .sub(&Interval::from_f64(0.3));
        assert_eq!(zero.sign(), None, "too close to zero to decide");
        let nan = Interval::WHOLE.mul(&Interval::from_f64(0.0));
        assert_eq!(nan.sign(), None);
    }
}
