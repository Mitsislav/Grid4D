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
}
