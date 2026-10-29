//! Effectors (see `docs/reference/mograph-effectors.md`): how a Plain or Random
//! effector changes a clone's placement and colour, and where its falloff
//! lets it work.

use std::f32::consts::TAU;

use glam::{EulerRot, Mat4, Quat, Vec3};

use crate::spline::{Polyline, Segment};

/// The shape of the region an effector works in, in its own space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FalloffShape {
    /// Everywhere, fully.
    #[default]
    Infinite,
    /// Inside an ellipsoid of the size.
    Sphere,
    /// Inside a box of the size.
    Box,
    /// From nothing on the −Z side of the size to fully on its +Z side.
    Linear,
}

impl FalloffShape {
    pub const ALL: [Self; 4] = [Self::Infinite, Self::Sphere, Self::Box, Self::Linear];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Infinite => "Infinite",
            Self::Sphere => "Sphere",
            Self::Box => "Box",
            Self::Linear => "Linear",
        }
    }
}

/// Where an effector works, and how its edge fades.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Falloff {
    pub shape: FalloffShape,
    /// The shape's extent (cm).
    pub size: Vec3,
    /// How much of the way in from the edge fades (0 – 1; 0: a hard edge).
    pub soft: f32,
}

impl Default for Falloff {
    fn default() -> Self {
        Self {
            shape: FalloffShape::Infinite,
            size: Vec3::splat(200.0),
            soft: 0.5,
        }
    }
}

impl Falloff {
    /// How strongly the effector works at `point` (its own space): 1 inside,
    /// 0 outside, smoothly between across the soft edge.
    #[must_use]
    pub fn weight(&self, point: Vec3) -> f32 {
        let half = (self.size.abs() * 0.5).max(Vec3::splat(1e-4));
        // How far out, as a fraction of the way to the edge (1: on it).
        let out = match self.shape {
            FalloffShape::Infinite => return 1.0,
            FalloffShape::Sphere => (point / half).length(),
            FalloffShape::Box => (point / half).abs().max_element(),
            FalloffShape::Linear => return smoothstep((point.z / half.z + 1.0) * 0.5),
        };
        let soft = self.soft.clamp(0.0, 1.0);
        if soft <= 0.0 {
            return if out <= 1.0 { 1.0 } else { 0.0 };
        }
        smoothstep((1.0 - out) / soft)
    }

    /// The shape's outline as lines (Infinite: none), in the effector's space.
    #[must_use]
    pub fn lines(&self) -> Polyline {
        let half = self.size.abs() * 0.5;
        let mut lines = Polyline::default();
        match self.shape {
            FalloffShape::Infinite => {}
            FalloffShape::Sphere => {
                const STEPS: usize = 48;
                for axes in [(Vec3::X, Vec3::Y), (Vec3::X, Vec3::Z), (Vec3::Y, Vec3::Z)] {
                    let ring = (0..STEPS).map(|i| {
                        let a = i as f32 / STEPS as f32 * TAU;
                        (axes.0 * a.cos() + axes.1 * a.sin()) * half
                    });
                    push_run(&mut lines, ring, true);
                }
            }
            FalloffShape::Box => push_box(&mut lines, -half, half),
            FalloffShape::Linear => {
                // Where it starts and where it is full, and an arrow between.
                for z in [-half.z, half.z] {
                    let corner = |x: f32, y: f32| Vec3::new(x * half.x, y * half.y, z);
                    push_run(
                        &mut lines,
                        [
                            corner(-1.0, -1.0),
                            corner(1.0, -1.0),
                            corner(1.0, 1.0),
                            corner(-1.0, 1.0),
                        ],
                        true,
                    );
                }
                let tip = Vec3::Z * half.z;
                push_run(&mut lines, [Vec3::Z * -half.z, tip], false);
                let barb = half.z.abs().min(half.x.abs()) * 0.2;
                push_run(
                    &mut lines,
                    [
                        tip + Vec3::new(-barb, 0.0, -barb),
                        tip,
                        tip + Vec3::new(barb, 0.0, -barb),
                    ],
                    false,
                );
            }
        }
        lines
    }
}

/// What an effector does to the clones it reaches.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Effector {
    /// 1.0 = 100 %.
    pub strength: f32,
    pub use_position: bool,
    /// Moved along the clone's own axes (cm).
    pub position: Vec3,
    pub use_scale: bool,
    /// Scale change (0.5: 50 % bigger).
    pub scale: Vec3,
    /// The same scale change on every axis (`scale.x`).
    pub uniform_scale: bool,
    pub use_rotation: bool,
    /// HPB (radians).
    pub rotation: Vec3,
    /// Colours the clones: the effector's colour (Plain) or a random one
    /// (Random), blended by how strongly it works.
    pub use_color: bool,
    pub color: [f32; 3],
    /// Random: the seed of each clone's values.
    pub seed: u32,
    pub falloff: Falloff,
}

impl Effector {
    /// A Plain effector's defaults (ours: the clones lifted 100 cm).
    #[must_use]
    pub fn plain() -> Self {
        Self {
            strength: 1.0,
            use_position: true,
            position: Vec3::new(0.0, 100.0, 0.0),
            use_scale: false,
            scale: Vec3::ZERO,
            uniform_scale: false,
            use_rotation: false,
            rotation: Vec3::ZERO,
            use_color: false,
            color: [1.0, 0.3, 0.2],
            seed: 12_345,
            falloff: Falloff::default(),
        }
    }

    /// A Random effector's defaults (ours: up to 50 cm each way).
    #[must_use]
    pub fn random() -> Self {
        Self {
            position: Vec3::splat(50.0),
            ..Self::plain()
        }
    }

    /// The change for one clone, `amount` being how strongly the effector works
    /// there (strength × falloff) and `random` its random factors in −1 … 1
    /// (Plain: all 1): a matrix to apply in the clone's own space.
    #[must_use]
    pub fn offset(&self, amount: f32, random: &Randoms) -> Mat4 {
        let position = if self.use_position {
            self.position * random.position * amount
        } else {
            Vec3::ZERO
        };
        let rotation = if self.use_rotation {
            self.rotation * random.rotation * amount
        } else {
            Vec3::ZERO
        };
        let scale = if !self.use_scale {
            Vec3::ONE
        } else if self.uniform_scale {
            Vec3::splat(1.0 + self.scale.x * random.uniform * amount)
        } else {
            Vec3::ONE + self.scale * random.scale * amount
        };
        Mat4::from_scale_rotation_translation(
            scale,
            Quat::from_euler(EulerRot::YXZ, rotation.x, rotation.y, rotation.z),
            position,
        )
    }

    /// What the viewport draws: a small cross at the effector's origin (so an
    /// Infinite one can be seen and picked) and its falloff's outline.
    #[must_use]
    pub fn lines(&self) -> Polyline {
        let mut lines = self.falloff.lines();
        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            push_run(&mut lines, [axis * -25.0, axis * 25.0], false);
        }
        lines
    }

    /// The colour a clone is blended towards.
    #[must_use]
    pub const fn tint(&self, random: &Randoms) -> [f32; 3] {
        match random.color {
            Some(color) => color,
            None => self.color,
        }
    }
}

/// One clone's random factors from a Random effector (Plain: [`Randoms::NONE`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Randoms {
    pub position: Vec3,
    pub rotation: Vec3,
    pub scale: Vec3,
    pub uniform: f32,
    /// A random colour (Random only).
    pub color: Option<[f32; 3]>,
}

impl Randoms {
    /// Every factor 1: the effector's values as they are.
    pub const NONE: Self = Self {
        position: Vec3::ONE,
        rotation: Vec3::ONE,
        scale: Vec3::ONE,
        uniform: 1.0,
        color: None,
    };

    /// Clone `index`'s factors for `seed`: each in −1 … 1, the colour's
    /// channels in 0 … 1, the same every time.
    #[must_use]
    pub fn of(seed: u32, index: u32) -> Self {
        let mut state = (u64::from(seed) << 32) ^ u64::from(index);
        let mut next = || {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            // 24 random bits → 0 … 1.
            (mix(state) >> 40) as f32 / (1u64 << 24) as f32
        };
        let mut signed = || next() * 2.0 - 1.0;
        let position = Vec3::new(signed(), signed(), signed());
        let rotation = Vec3::new(signed(), signed(), signed());
        let scale = Vec3::new(signed(), signed(), signed());
        let uniform = signed();
        let color = [next(), next(), next()];
        Self {
            position,
            rotation,
            scale,
            uniform,
            color: Some(color),
        }
    }
}

/// Appends one run of points to `lines`.
fn push_run(lines: &mut Polyline, points: impl IntoIterator<Item = Vec3>, closed: bool) {
    let start = lines.points.len();
    lines.points.extend(points);
    let count = u32::try_from(lines.points.len() - start).unwrap_or(u32::MAX);
    lines.segments.push(Segment { count, closed });
}

/// 0 below 0, 1 above 1, smooth between.
fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// SplitMix64's finaliser.
const fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The twelve edges of the box from `min` to `max`.
fn push_box(lines: &mut Polyline, min: Vec3, max: Vec3) {
    let corner = |i: u8| {
        Vec3::new(
            if i & 1 == 0 { min.x } else { max.x },
            if i & 2 == 0 { min.y } else { max.y },
            if i & 4 == 0 { min.z } else { max.z },
        )
    };
    for (a, b) in [
        (0, 1),
        (2, 3),
        (4, 5),
        (6, 7),
        (0, 2),
        (1, 3),
        (4, 6),
        (5, 7),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ] {
        push_run(lines, [corner(a), corner(b)], false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falloffs_are_full_inside_empty_outside_and_soft_between() {
        let sphere = Falloff {
            shape: FalloffShape::Sphere,
            size: Vec3::splat(200.0),
            soft: 0.5,
        };
        assert_eq!(sphere.weight(Vec3::ZERO), 1.0);
        assert_eq!(sphere.weight(Vec3::X * 40.0), 1.0, "inside the soft edge");
        let edge = sphere.weight(Vec3::X * 75.0);
        assert!(edge > 0.0 && edge < 1.0, "{edge}");
        assert_eq!(sphere.weight(Vec3::X * 120.0), 0.0);
        let hard = Falloff {
            shape: FalloffShape::Box,
            soft: 0.0,
            ..sphere
        };
        assert_eq!(hard.weight(Vec3::new(99.0, 99.0, 0.0)), 1.0, "a box's corner is inside");
        assert_eq!(hard.weight(Vec3::new(101.0, 0.0, 0.0)), 0.0);
        let linear = Falloff {
            shape: FalloffShape::Linear,
            ..sphere
        };
        assert_eq!(linear.weight(Vec3::Z * -100.0), 0.0);
        assert_eq!(linear.weight(Vec3::Z * 100.0), 1.0);
        assert!((linear.weight(Vec3::ZERO) - 0.5).abs() < 1e-6);
        assert_eq!(Falloff::default().weight(Vec3::splat(1e6)), 1.0, "Infinite");
        assert!(Falloff::default().lines().points.is_empty());
        assert!(!sphere.lines().points.is_empty() && !linear.lines().points.is_empty());
    }

    #[test]
    fn plain_offsets_scale_with_the_amount_and_random_varies_per_clone() {
        let plain = Effector {
            use_scale: true,
            scale: Vec3::splat(1.0),
            uniform_scale: true,
            ..Effector::plain()
        };
        let half = plain.offset(0.5, &Randoms::NONE);
        assert_eq!(half.w_axis.truncate(), Vec3::new(0.0, 50.0, 0.0));
        let (scale, _, _) = half.to_scale_rotation_translation();
        assert!(scale.abs_diff_eq(Vec3::splat(1.5), 1e-5));
        assert_eq!(plain.offset(0.0, &Randoms::NONE), Mat4::IDENTITY);

        let random = Effector::random();
        let (a, b) = (Randoms::of(random.seed, 0), Randoms::of(random.seed, 1));
        assert_eq!(a, Randoms::of(random.seed, 0), "repeatable");
        assert_ne!(a, b, "per clone");
        assert_ne!(a, Randoms::of(random.seed + 1, 0), "per seed");
        let moved = random.offset(1.0, &a).w_axis.truncate();
        assert!(moved.abs().max_element() <= 50.0 && moved != Vec3::ZERO, "{moved}");
        let all = (0..200).map(|i| Randoms::of(7, i));
        assert!(all.clone().all(|r| r.position.abs().max_element() <= 1.0));
        assert!(all.clone().any(|r| r.position.x < -0.5) && all.clone().any(|r| r.position.x > 0.5));
        assert!(all.flat_map(|r| r.color.unwrap()).all(|c| (0.0..=1.0).contains(&c)));
        assert_eq!(plain.tint(&Randoms::NONE), plain.color);
    }
}
