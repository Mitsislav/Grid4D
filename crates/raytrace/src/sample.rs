//! Random numbers and the directions drawn from them.
//!
//! Every pixel sample has its own random stream, seeded from the pixel, the
//! sample and the render's seed: renders do not depend on thread scheduling.

use std::f32::consts::TAU;

use glam::Vec3;

/// A small PCG random number generator.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Rng {
    state: u64,
}

impl Rng {
    /// The stream for sample `sample` of pixel (`x`, `y`).
    pub(crate) fn new(x: u32, y: u32, sample: u32, seed: u32) -> Self {
        let mixed = hash(x ^ hash(y ^ hash(sample ^ hash(seed))));
        let mut rng = Self {
            state: u64::from(mixed) << 32 | u64::from(hash(mixed)),
        };
        rng.next_u32();
        rng
    }

    pub(crate) fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        // XSH RR: the top bits, xor-shifted and rotated.
        let xorshifted = (((old >> 18) ^ old) >> 27) & 0xFFFF_FFFF;
        let rotation = old >> 59;
        let value = u32::try_from(xorshifted).unwrap_or(0);
        value.rotate_right(u32::try_from(rotation).unwrap_or(0))
    }

    /// A number in [0, 1).
    pub(crate) fn next_f32(&mut self) -> f32 {
        // 23 random mantissa bits under the exponent of 1.0 give [1, 2).
        f32::from_bits(0x3F80_0000 | (self.next_u32() >> 9)) - 1.0
    }
}

/// A well-mixed 32-bit hash (PCG output function).
fn hash(value: u32) -> u32 {
    let state = value.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let word = ((state >> ((state >> 28) + 4)) ^ state).wrapping_mul(277_803_737);
    (word >> 22) ^ word
}

/// Two unit vectors completing `n` (unit) to an orthonormal frame.
pub(crate) fn frame(n: Vec3) -> (Vec3, Vec3) {
    let tangent = n.any_orthonormal_vector();
    (tangent, n.cross(tangent))
}

/// A direction in the hemisphere around `n`, more often near `n` (cosine
/// weighted: its probability density is cos θ / π).
pub(crate) fn cosine_hemisphere(n: Vec3, u: f32, v: f32) -> Vec3 {
    let r = u.sqrt();
    let phi = TAU * v;
    let (t, b) = frame(n);
    (t * (r * phi.cos()) + b * (r * phi.sin()) + n * (1.0 - u).max(0.0).sqrt()).normalize_or(n)
}

/// A point in the unit ball.
pub(crate) fn in_ball(rng: &mut Rng) -> Vec3 {
    loop {
        let p = Vec3::new(rng.next_f32(), rng.next_f32(), rng.next_f32()) * 2.0 - Vec3::ONE;
        if p.length_squared() <= 1.0 {
            return p;
        }
    }
}

/// A direction within `angle` (radians) of `axis` (unit), spread evenly over
/// the cone's cap.
pub(crate) fn in_cone(axis: Vec3, angle: f32, u: f32, v: f32) -> Vec3 {
    let cos_max = angle.cos();
    let cos_theta = 1.0 - u * (1.0 - cos_max);
    let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
    let phi = TAU * v;
    let (t, b) = frame(axis);
    (t * (sin_theta * phi.cos()) + b * (sin_theta * phi.sin()) + axis * cos_theta).normalize_or(axis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_are_repeatable_and_differ_per_pixel() {
        let draw = |x, y| {
            let mut rng = Rng::new(x, y, 0, 7);
            [rng.next_f32(), rng.next_f32(), rng.next_f32()]
        };
        assert_eq!(draw(3, 4), draw(3, 4));
        assert_ne!(draw(3, 4), draw(4, 3));
        let mut rng = Rng::new(0, 0, 0, 0);
        let mean = (0..10_000).map(|_| rng.next_f32()).sum::<f32>() / 10_000.0;
        assert!((mean - 0.5).abs() < 0.02, "{mean}");
    }

    #[test]
    fn hemisphere_and_cone_samples_stay_inside() {
        let mut rng = Rng::new(1, 2, 3, 4);
        let n = Vec3::new(0.3, 0.9, -0.2).normalize();
        for _ in 0..1000 {
            let d = cosine_hemisphere(n, rng.next_f32(), rng.next_f32());
            assert!(d.dot(n) >= -1e-5 && (d.length() - 1.0).abs() < 1e-4);
            let c = in_cone(n, 0.1, rng.next_f32(), rng.next_f32());
            assert!(c.dot(n) >= 0.1f32.cos() - 1e-4);
            assert!(in_ball(&mut rng).length() <= 1.0);
        }
    }
}
