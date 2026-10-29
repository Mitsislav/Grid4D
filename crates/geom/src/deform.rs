//! Deformers' maths (see `docs/reference/deformer-basics.md`,
//! `deformer-bend.md`): pure functions that move a point given in the
//! deformer's own space, and the deformer's box as it is drawn.

use glam::{Quat, Vec3};

use crate::spline::{Polyline, Segment};

/// What a deformer does outside its box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DeformerMode {
    /// The box's part is deformed; what lies beyond follows along.
    #[default]
    Limited,
    /// Only points inside the box move.
    WithinBox,
    /// Everything is deformed as if the box had no end.
    Unlimited,
}

impl DeformerMode {
    pub const ALL: [Self; 3] = [Self::Limited, Self::WithinBox, Self::Unlimited];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Limited => "Limited",
            Self::WithinBox => "Within Box",
            Self::Unlimited => "Unlimited",
        }
    }
}

/// Where a Bend starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BendOrigin {
    /// At the box's bottom: the top bends over.
    #[default]
    Bottom,
    /// At its centre, both halves curling the same way.
    CenterC,
    /// At its centre, the halves curling opposite ways.
    CenterS,
    /// At its top: the bottom bends.
    Top,
}

impl BendOrigin {
    pub const ALL: [Self; 4] = [Self::Bottom, Self::CenterC, Self::CenterS, Self::Top];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Bottom => "Bottom",
            Self::CenterC => "Center C-Shape",
            Self::CenterS => "Center S-Shape",
            Self::Top => "Top",
        }
    }
}

/// The Bend deformer's parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bend {
    /// The box, centred on the origin (cm).
    pub size: Vec3,
    pub mode: DeformerMode,
    /// How far the box's height bends round (radians).
    pub strength: f32,
    /// The bend's direction about Y (radians; 0 = towards +X).
    pub angle: f32,
    /// The bent part keeps its length (else its ends stay the height apart).
    pub keep_length: bool,
    pub origin: BendOrigin,
}

impl Default for Bend {
    fn default() -> Self {
        Self {
            size: Vec3::splat(250.0),
            mode: DeformerMode::Limited,
            strength: 0.0,
            angle: 0.0,
            keep_length: false,
            origin: BendOrigin::Bottom,
        }
    }
}

impl Bend {
    /// `point` (in the deformer's space) bent.
    #[must_use]
    pub fn apply(&self, point: Vec3) -> Vec3 {
        let height = self.size.y.abs();
        if self.strength.abs() < 1e-6 || height < 1e-6 || !point.is_finite() {
            return point;
        }
        if self.mode == DeformerMode::WithinBox && !inside(point, self.size) {
            return point;
        }
        let unlimited = self.mode == DeformerMode::Unlimited;
        // Bend towards +X in a frame turned by the angle.
        let q = Quat::from_rotation_y(-self.angle) * point;
        let half = height * 0.5;
        let column = |p: Vec3, length: f32, theta: f32| bend_column(p, length, theta, self.keep_length, unlimited);
        let bent = match self.origin {
            BendOrigin::Bottom => {
                let lifted = column(q + Vec3::Y * half, height, self.strength);
                lifted - Vec3::Y * half
            }
            BendOrigin::Top => {
                let flipped = Vec3::new(q.x, -q.y, q.z) + Vec3::Y * half;
                let b = column(flipped, height, self.strength) - Vec3::Y * half;
                Vec3::new(b.x, -b.y, b.z)
            }
            BendOrigin::CenterC | BendOrigin::CenterS => {
                // Each half turns through half the strength from the centre.
                let theta = self.strength * 0.5;
                if q.y >= 0.0 {
                    column(q, half, theta)
                } else if self.origin == BendOrigin::CenterC {
                    let b = column(Vec3::new(q.x, -q.y, q.z), half, theta);
                    Vec3::new(b.x, -b.y, b.z)
                } else {
                    let b = column(Vec3::new(-q.x, -q.y, q.z), half, theta);
                    Vec3::new(-b.x, -b.y, b.z)
                }
            }
        };
        Quat::from_rotation_y(self.angle) * bent
    }

    /// The box's twelve edges, each cut into pieces and bent, as open
    /// segments of a polyline (drawn and picked like a spline).
    #[must_use]
    pub fn box_lines(&self) -> Polyline {
        box_lines(self.size, |p| self.apply(p))
    }
}

impl Bend {
    /// Where the strength handle sits (deformer space): the end of the bending
    /// part, where the box's top (or, with Origin Top, its bottom) centre went.
    #[must_use]
    pub fn handle(&self) -> Vec3 {
        let half = self.size.y.abs() * 0.5;
        let end = if self.origin == BendOrigin::Top { -half } else { half };
        self.apply(Vec3::new(0.0, end, 0.0))
    }

    /// The plane the handle is dragged in: the bend's own plane (through the
    /// origin, spanned by Y and the bend's direction).
    #[must_use]
    pub fn handle_plane(&self) -> (Vec3, Vec3) {
        (Vec3::ZERO, Quat::from_rotation_y(self.angle) * Vec3::Z)
    }

    /// This Bend with the strength that puts its handle nearest `target`.
    #[must_use]
    pub fn with_handle(&self, target: Vec3) -> Self {
        let with = |strength: f32| Self { strength, ..*self };
        let distance = |strength: f32| with(strength).handle().distance_squared(target);
        // Coarse search over two turns either way, then refine.
        let mut best = self.strength;
        let mut best_distance = distance(best);
        for i in -720..=720 {
            let strength = f32::from(i16::try_from(i).unwrap_or(0)) * std::f32::consts::PI / 180.0;
            let d = distance(strength);
            if d < best_distance {
                (best, best_distance) = (strength, d);
            }
        }
        let mut step = std::f32::consts::PI / 180.0;
        for _ in 0..24 {
            for candidate in [best - step, best + step] {
                let d = distance(candidate);
                if d < best_distance {
                    (best, best_distance) = (candidate, d);
                }
            }
            step *= 0.5;
        }
        with(best)
    }
}

/// The Twist deformer's parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Twist {
    pub size: Vec3,
    pub mode: DeformerMode,
    /// How far the box's top turns relative to its bottom (radians).
    pub angle: f32,
}

impl Default for Twist {
    fn default() -> Self {
        Self {
            size: Vec3::splat(250.0),
            mode: DeformerMode::Limited,
            angle: 0.0,
        }
    }
}

impl Twist {
    /// `point` (in the deformer's space) turned about Y by its height.
    #[must_use]
    pub fn apply(&self, point: Vec3) -> Vec3 {
        let Some(t) = along(point, self.size, self.mode) else {
            return point;
        };
        Quat::from_rotation_y(self.angle * t) * point
    }

    #[must_use]
    pub fn box_lines(&self) -> Polyline {
        box_lines(self.size, |p| self.apply(p))
    }

    /// The handle: the top of the box on its +X side, turned with the twist.
    #[must_use]
    pub fn handle(&self) -> Vec3 {
        self.apply(Vec3::new(self.size.x.abs() * 0.5, self.size.y.abs() * 0.5, 0.0))
    }

    /// The top of the box.
    #[must_use]
    pub fn handle_plane(&self) -> (Vec3, Vec3) {
        (Vec3::Y * (self.size.y.abs() * 0.5), Vec3::Y)
    }

    /// This Twist turned so its handle points at `target`, winding on past a
    /// full turn from the current angle.
    #[must_use]
    pub fn with_handle(&self, target: Vec3) -> Self {
        if target.x.hypot(target.z) < 1e-6 {
            return *self;
        }
        // A turn about Y takes +X towards −Z.
        let wanted = (-target.z).atan2(target.x);
        let tau = std::f32::consts::TAU;
        let delta = (wanted - self.angle).rem_euclid(tau);
        let delta = if delta > std::f32::consts::PI {
            delta - tau
        } else {
            delta
        };
        Self {
            angle: self.angle + delta,
            ..*self
        }
    }
}

/// How a Taper or Bulge eases its effect.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Shape {
    /// 0 – any (1.0 = 100 %).
    pub curvature: f32,
    /// Soft ends.
    pub fillet: bool,
}

impl Shape {
    fn ease(self, value: f32) -> f32 {
        if self.fillet {
            value * value * (3.0 - 2.0 * value)
        } else {
            value
        }
    }
}

/// The Taper deformer's parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Taper {
    pub size: Vec3,
    pub mode: DeformerMode,
    /// How much narrower the top gets (1.0 = 100 %: to a point).
    pub strength: f32,
    pub shape: Shape,
}

impl Default for Taper {
    fn default() -> Self {
        Self {
            size: Vec3::splat(250.0),
            mode: DeformerMode::Limited,
            strength: 0.0,
            shape: Shape::default(),
        }
    }
}

impl Taper {
    /// The scale of distances from the Y axis at `t` through the box's height.
    #[must_use]
    pub fn factor(&self, t: f32) -> f32 {
        // Inside the box: the eased curve; beyond it (Unlimited): straight on.
        let g = if (0.0..=1.0).contains(&t) {
            self.shape.ease(t.powf(1.0 + self.shape.curvature.max(0.0)))
        } else {
            t
        };
        (1.0 - self.strength * g).max(0.0)
    }

    /// `point` (in the deformer's space) tapered.
    #[must_use]
    pub fn apply(&self, point: Vec3) -> Vec3 {
        let Some(t) = along(point, self.size, self.mode) else {
            return point;
        };
        let k = self.factor(t);
        Vec3::new(point.x * k, point.y, point.z * k)
    }

    #[must_use]
    pub fn box_lines(&self) -> Polyline {
        box_lines(self.size, |p| self.apply(p))
    }

    /// The handle: the top of the box on its +X side, at the tapered width.
    #[must_use]
    pub fn handle(&self) -> Vec3 {
        self.apply(Vec3::new(self.size.x.abs() * 0.5, self.size.y.abs() * 0.5, 0.0))
    }

    /// The deformer's XY plane.
    #[must_use]
    pub fn handle_plane(&self) -> (Vec3, Vec3) {
        (Vec3::ZERO, Vec3::Z)
    }

    /// This Taper with the strength that puts its handle at `target`'s
    /// distance from the axis.
    #[must_use]
    pub fn with_handle(&self, target: Vec3) -> Self {
        let half = self.size.x.abs() * 0.5;
        if half < 1e-6 {
            return *self;
        }
        Self {
            strength: 1.0 - target.x / half,
            ..*self
        }
    }
}

/// The Bulge deformer's parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bulge {
    pub size: Vec3,
    pub mode: DeformerMode,
    /// How much wider the middle gets (1.0 = 100 %: twice as wide).
    pub strength: f32,
    pub shape: Shape,
}

impl Default for Bulge {
    fn default() -> Self {
        Self {
            size: Vec3::splat(250.0),
            mode: DeformerMode::Limited,
            strength: 0.0,
            shape: Shape::default(),
        }
    }
}

impl Bulge {
    /// The scale of distances from the Y axis at `t` through the box's height.
    #[must_use]
    pub fn factor(&self, t: f32) -> f32 {
        let unlimited = self.mode == DeformerMode::Unlimited;
        if !unlimited && !(0.0..=1.0).contains(&t) {
            return 1.0;
        }
        let u = (2.0 * t - 1.0).abs();
        let b = 1.0 - u.powf(2.0 + 2.0 * self.shape.curvature.max(0.0));
        let b = if (0.0..=1.0).contains(&b) {
            self.shape.ease(b)
        } else {
            b
        };
        (1.0 + self.strength * b).max(0.0)
    }

    /// `point` (in the deformer's space) swollen.
    #[must_use]
    pub fn apply(&self, point: Vec3) -> Vec3 {
        let Some(t) = along(point, self.size, self.mode) else {
            return point;
        };
        let k = self.factor(t);
        Vec3::new(point.x * k, point.y, point.z * k)
    }

    #[must_use]
    pub fn box_lines(&self) -> Polyline {
        box_lines(self.size, |p| self.apply(p))
    }

    /// The handle: the middle of the box on its +X side, at the swollen width.
    #[must_use]
    pub fn handle(&self) -> Vec3 {
        self.apply(Vec3::new(self.size.x.abs() * 0.5, 0.0, 0.0))
    }

    /// The deformer's XY plane.
    #[must_use]
    pub fn handle_plane(&self) -> (Vec3, Vec3) {
        (Vec3::ZERO, Vec3::Z)
    }

    /// This Bulge with the strength that puts its handle at `target`'s
    /// distance from the axis.
    #[must_use]
    pub fn with_handle(&self, target: Vec3) -> Self {
        let half = self.size.x.abs() * 0.5;
        if half < 1e-6 {
            return *self;
        }
        Self {
            strength: target.x / half - 1.0,
            ..*self
        }
    }
}

/// How far through the box's height `point` is (0 at the bottom, 1 at the
/// top), as the mode reads it: clamped to the box (Limited), as it is
/// (Unlimited); `None` for a point a Within Box deformer leaves alone.
fn along(point: Vec3, size: Vec3, mode: DeformerMode) -> Option<f32> {
    let height = size.y.abs();
    if height < 1e-6 || !point.is_finite() {
        return None;
    }
    let t = (point.y + height * 0.5) / height;
    match mode {
        DeformerMode::Limited => Some(t.clamp(0.0, 1.0)),
        DeformerMode::Unlimited => Some(t),
        DeformerMode::WithinBox => inside(point, size).then_some(t),
    }
}

/// Bends the column starting at y = 0 with `length` through `theta` towards +X:
/// a point `s` along it turns through `theta · s / length` on a circle.
fn bend_column(p: Vec3, length: f32, theta: f32, keep_length: bool, unlimited: bool) -> Vec3 {
    // The circle the centre line follows: as long as the column, or with its
    // chord as long as the column.
    let radius = if keep_length {
        length / theta
    } else {
        let half_sine = (theta * 0.5).sin();
        if half_sine.abs() < 1e-3 {
            length / theta
        } else {
            length / (2.0 * half_sine)
        }
    };
    let s = p.y;
    let inside = if unlimited { s } else { s.clamp(0.0, length) };
    let beyond = s - inside;
    let phi = theta * inside / length;
    let (sin, cos) = phi.sin_cos();
    let r = radius - p.x;
    let on_circle = Vec3::new(radius - r * cos, r * sin, p.z);
    // Beyond the column: straight on along the bend's end.
    on_circle + Vec3::new(sin, cos, 0.0) * beyond
}

fn inside(p: Vec3, size: Vec3) -> bool {
    let half = size.abs() * 0.5;
    p.x.abs() <= half.x + 1e-4 && p.y.abs() <= half.y + 1e-4 && p.z.abs() <= half.z + 1e-4
}

/// The edges of a box of `size` centred on the origin, each cut into pieces and
/// passed through `deform`, as open polyline segments.
#[must_use]
pub fn box_lines(size: Vec3, deform: impl Fn(Vec3) -> Vec3) -> Polyline {
    const PIECES: u16 = 24;
    let h = size * 0.5;
    let corner = |x: f32, y: f32, z: f32| Vec3::new(x * h.x, y * h.y, z * h.z);
    let mut edges: Vec<(Vec3, Vec3)> = Vec::with_capacity(12);
    for s in [-1.0, 1.0] {
        for t in [-1.0, 1.0] {
            edges.push((corner(-1.0, s, t), corner(1.0, s, t)));
            edges.push((corner(s, -1.0, t), corner(s, 1.0, t)));
            edges.push((corner(s, t, -1.0), corner(s, t, 1.0)));
        }
    }
    let mut out = Polyline::default();
    for (a, b) in edges {
        out.points
            .extend((0..=PIECES).map(|i| deform(a.lerp(b, f32::from(i) / f32::from(PIECES)))));
        out.segments.push(Segment {
            count: u32::from(PIECES) + 1,
            closed: false,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use std::f32::consts::{FRAC_PI_2, PI};

    use super::*;

    /// A bend of a column 100 high, its bottom at y = 0 (the box from −50).
    fn bend(strength: f32) -> Bend {
        Bend {
            size: Vec3::new(100.0, 100.0, 100.0),
            strength,
            ..Bend::default()
        }
    }

    fn close(a: Vec3, b: Vec3) -> bool {
        a.abs_diff_eq(b, 1e-3)
    }

    #[test]
    fn a_quarter_bend_keeping_length_ends_on_a_quarter_circle() {
        let b = Bend {
            keep_length: true,
            ..bend(FRAC_PI_2)
        };
        let radius = 100.0 / FRAC_PI_2;
        // The bottom stays; the top of the centre line reaches (R, −50 + R), pointing along +X.
        assert!(close(b.apply(Vec3::new(0.0, -50.0, 0.0)), Vec3::new(0.0, -50.0, 0.0)));
        assert!(close(
            b.apply(Vec3::new(0.0, 50.0, 0.0)),
            Vec3::new(radius, -50.0 + radius, 0.0)
        ));
        // Beyond the box (Limited): straight on along +X.
        assert!(close(
            b.apply(Vec3::new(0.0, 60.0, 0.0)),
            Vec3::new(radius + 10.0, -50.0 + radius, 0.0)
        ));
        // Below the box: untouched.
        assert!(close(b.apply(Vec3::new(5.0, -70.0, 3.0)), Vec3::new(5.0, -70.0, 3.0)));
        // The centre line keeps its length: points 1 apart stay ~1 apart.
        let (p, q) = (b.apply(Vec3::new(0.0, 0.0, 0.0)), b.apply(Vec3::new(0.0, 1.0, 0.0)));
        assert!((p.distance(q) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn without_keeping_length_the_ends_stay_the_height_apart() {
        let b = bend(FRAC_PI_2);
        let (bottom, top) = (b.apply(Vec3::new(0.0, -50.0, 0.0)), b.apply(Vec3::new(0.0, 50.0, 0.0)));
        assert!((bottom.distance(top) - 100.0).abs() < 1e-3);
        assert!(top.x > 0.0);
    }

    #[test]
    fn angle_and_sign_choose_the_direction() {
        let towards = |b: Bend| b.apply(Vec3::new(0.0, 50.0, 0.0));
        let minus_z = towards(Bend {
            angle: FRAC_PI_2,
            ..bend(FRAC_PI_2)
        });
        assert!(minus_z.z < -10.0 && minus_z.x.abs() < 1e-3, "{minus_z}");
        assert!(towards(bend(-FRAC_PI_2)).x < -10.0);
    }

    #[test]
    fn modes() {
        let beyond = Vec3::new(0.0, 80.0, 0.0);
        let limited = bend(FRAC_PI_2).apply(beyond);
        let within = Bend {
            mode: DeformerMode::WithinBox,
            ..bend(FRAC_PI_2)
        };
        assert_eq!(within.apply(beyond), beyond, "outside the box: stays");
        assert_ne!(within.apply(Vec3::new(0.0, 40.0, 0.0)), Vec3::new(0.0, 40.0, 0.0));
        let unlimited = Bend {
            mode: DeformerMode::Unlimited,
            ..bend(FRAC_PI_2)
        }
        .apply(beyond);
        assert!(unlimited.x > limited.x - 50.0 && unlimited != limited, "bent on");
        assert!(
            unlimited.y < limited.y,
            "the circle turns further over: {unlimited} vs {limited}"
        );
    }

    #[test]
    fn origins() {
        let with = |origin| Bend { origin, ..bend(PI) };
        let top = with(BendOrigin::Top);
        assert!(
            close(top.apply(Vec3::new(0.0, 50.0, 0.0)), Vec3::new(0.0, 50.0, 0.0)),
            "the top stays"
        );
        assert!(top.apply(Vec3::new(0.0, -50.0, 0.0)).x > 10.0);
        let c = with(BendOrigin::CenterC);
        assert!(close(c.apply(Vec3::ZERO), Vec3::ZERO));
        assert!(c.apply(Vec3::new(0.0, 50.0, 0.0)).x > 10.0 && c.apply(Vec3::new(0.0, -50.0, 0.0)).x > 10.0);
        let s = with(BendOrigin::CenterS);
        assert!(s.apply(Vec3::new(0.0, 50.0, 0.0)).x > 10.0 && s.apply(Vec3::new(0.0, -50.0, 0.0)).x < -10.0);
    }

    #[test]
    fn no_strength_is_the_identity_and_the_box_has_twelve_edges() {
        let p = Vec3::new(3.0, 70.0, -9.0);
        assert_eq!(bend(0.0).apply(p), p);
        let lines = bend(FRAC_PI_2).box_lines();
        assert_eq!(lines.segments.len(), 12);
        assert!(lines.segments.iter().all(|s| !s.closed && s.count == 25));
        let (min, max) = bend(0.0).box_lines().bounds().unwrap();
        assert!(
            close(min, Vec3::splat(-50.0)) && close(max, Vec3::splat(50.0)),
            "{min} {max}"
        );
    }

    #[test]
    fn twist_turns_by_height() {
        let t = Twist {
            size: Vec3::splat(100.0),
            angle: FRAC_PI_2,
            ..Twist::default()
        };
        let x = Vec3::new(10.0, 0.0, 0.0);
        assert!(
            close(t.apply(x + Vec3::Y * -50.0), x + Vec3::Y * -50.0),
            "the bottom stays"
        );
        assert!(
            close(t.apply(x + Vec3::Y * 50.0), Vec3::new(0.0, 50.0, -10.0)),
            "the top turns a quarter"
        );
        let middle = t.apply(x);
        assert!(
            (middle.x - 10.0 * (PI / 4.0).cos()).abs() < 1e-3 && middle.z < 0.0,
            "{middle}"
        );
        assert!(
            close(t.apply(x + Vec3::Y * 90.0), Vec3::new(0.0, 90.0, -10.0)),
            "Limited: above turns fully"
        );
        // The handle follows the pointer around, winding on.
        let turned = t.with_handle(Vec3::new(-10.0, 50.0, 0.0));
        assert!((turned.angle - PI).abs() < 1e-4, "{}", turned.angle);
        assert!(close(turned.handle(), Vec3::new(-50.0, 50.0, 0.0)));
        let wound = turned.with_handle(Vec3::new(0.0, 50.0, 10.0));
        assert!(
            (wound.angle - 1.5 * PI).abs() < 1e-4,
            "past half a turn: {}",
            wound.angle
        );
    }

    #[test]
    fn taper_narrows_towards_the_top() {
        let full = Taper {
            size: Vec3::splat(100.0),
            strength: 1.0,
            ..Taper::default()
        };
        let p = |y: f32| full.apply(Vec3::new(10.0, y, 10.0));
        assert!(close(p(-50.0), Vec3::new(10.0, -50.0, 10.0)), "the bottom stays");
        assert!(close(p(50.0), Vec3::new(0.0, 50.0, 0.0)), "the top to a point");
        assert!(close(p(0.0), Vec3::new(5.0, 0.0, 5.0)), "half way, half width");
        let curved = Taper {
            shape: Shape {
                curvature: 1.0,
                fillet: false,
            },
            ..full
        };
        assert!(curved.apply(Vec3::new(10.0, 0.0, 0.0)).x > 7.0, "slower at first");
        let filleted = Taper {
            shape: Shape {
                curvature: 0.0,
                fillet: true,
            },
            ..full
        };
        assert!(filleted.factor(0.05) > full.factor(0.05), "eases in");
        let widen = Taper { strength: -1.0, ..full };
        assert!(close(
            widen.apply(Vec3::new(10.0, 50.0, 0.0)),
            Vec3::new(20.0, 50.0, 0.0)
        ));
        // The handle: at the tapered top; dragged to half the width, strength 50 %.
        assert!(close(full.handle(), Vec3::new(0.0, 50.0, 0.0)));
        assert!((full.with_handle(Vec3::new(25.0, 50.0, 0.0)).strength - 0.5).abs() < 1e-6);
    }

    #[test]
    fn bulge_swells_the_middle() {
        let b = Bulge {
            size: Vec3::splat(100.0),
            strength: 1.0,
            ..Bulge::default()
        };
        let p = |y: f32| b.apply(Vec3::new(10.0, y, 0.0));
        assert!(close(p(0.0), Vec3::new(20.0, 0.0, 0.0)), "the middle doubles");
        assert!(close(p(50.0), Vec3::new(10.0, 50.0, 0.0)) && close(p(-50.0), Vec3::new(10.0, -50.0, 0.0)));
        assert!(
            close(p(80.0), Vec3::new(10.0, 80.0, 0.0)),
            "outside the height: nothing"
        );
        let flat = Bulge {
            shape: Shape {
                curvature: 2.0,
                fillet: false,
            },
            ..b
        };
        assert!(flat.factor(0.25) > b.factor(0.25), "a flatter, wider swell");
        let pinch = Bulge { strength: -2.0, ..b };
        assert_eq!(pinch.factor(0.5), 0.0, "never inside out");
        assert!(close(b.handle(), Vec3::new(100.0, 0.0, 0.0)));
        assert!((b.with_handle(Vec3::new(75.0, 0.0, 0.0)).strength - 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_bend_handle_follows_the_pointer_along_its_arc() {
        let b = Bend {
            keep_length: true,
            ..bend(0.0)
        };
        assert!(close(b.handle(), Vec3::new(0.0, 50.0, 0.0)));
        let quarter = Bend {
            strength: FRAC_PI_2,
            ..b
        };
        let target = quarter.handle();
        let found = b.with_handle(target + Vec3::new(0.5, 0.5, 0.0));
        assert!(
            (found.strength - FRAC_PI_2).abs() < 0.02,
            "{}",
            found.strength.to_degrees()
        );
        let (point, normal) = quarter.handle_plane();
        assert!(
            normal.dot(quarter.handle() - point).abs() < 1e-3,
            "the handle lies in its plane"
        );
    }
}
