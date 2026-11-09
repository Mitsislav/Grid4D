//! Subsurface scattering by a random walk (see
//! `docs/reference/material-subsurface-displacement.md`): light that enters
//! a surface scatters inside it — each step a free flight drawn from the
//! medium's extinction, then a new random direction — until it leaves
//! through a surface, often somewhere else. The medium's coefficients come
//! from the colour light takes inside and the distance each colour travels.

use glam::Vec3;

use crate::{
    sample::Rng,
    scene::{Hit, RenderScene},
};

/// Steps before a walk gives up (the light counts as absorbed).
const MAX_STEPS: u32 = 256;
/// After this many steps, faint walks may end early (Russian roulette).
const ROULETTE_AFTER: u32 = 8;

/// What a walk goes through: per colour, how often light meets a particle
/// (per cm) and how much of it is scattered rather than absorbed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Medium {
    /// Extinction, per cm.
    sigma_t: Vec3,
    /// Single-scattering albedo (0 – 1).
    albedo: Vec3,
}

impl Medium {
    /// The medium whose light comes back out with roughly `color` (the
    /// multiple-scattering albedo) after travelling about `radius` cm.
    pub(crate) fn new(color: Vec3, radius: Vec3) -> Self {
        let albedo = color
            .clamp(Vec3::ZERO, Vec3::splat(0.999))
            .to_array()
            .map(single_albedo);
        Self {
            sigma_t: radius.max(Vec3::splat(1e-3)).recip(),
            albedo: Vec3::from(albedo),
        }
    }
}

/// The single-scattering albedo that gives multiple-scattering albedo `a`
/// in a thick slab (van de Hulst's relation, inverted).
fn single_albedo(a: f32) -> f32 {
    let root = 4.097_12 + 4.208_63 * a - (9.592_17 + 41.680_8 * a + 17.712_6 * a * a).sqrt();
    (1.0 - root * root).clamp(0.0, 0.999_9)
}

/// Where a walk left the object, what reached there (a colour weight), and
/// the direction it left in.
pub(crate) struct Exit {
    pub hit: Hit,
    pub weight: Vec3,
    pub direction: Vec3,
}

/// Walks from `start` (just inside a surface) along `dir` through `medium`
/// until the light leaves through a surface; `None` when it is absorbed.
pub(crate) fn walk(
    scene: &RenderScene,
    start: Vec3,
    dir: Vec3,
    medium: Medium,
    time: f32,
    rng: &mut Rng,
) -> Option<Exit> {
    let sigma_s = medium.sigma_t * medium.albedo;
    let mut weight = Vec3::ONE;
    let (mut position, mut direction) = (start, dir);
    for step in 0..MAX_STEPS {
        // A colour to draw the flight with, as often as it still carries light.
        let total = weight.element_sum();
        if total <= 1e-6 {
            return None;
        }
        let chance = weight / total;
        let u = rng.next_f32();
        let channel = if u < chance.x {
            0
        } else if u < chance.x + chance.y {
            1
        } else {
            2
        };
        let flight = -(1.0 - rng.next_f32()).max(1e-12).ln() / medium.sigma_t[channel];
        let hit = scene.intersect(position, direction, time);
        let reach = hit
            .as_ref()
            .map_or(f32::INFINITY, |hit| (hit.position - position).length());
        if flight >= reach {
            // Out through the surface met: the light that got that far,
            // over the chance of flying at least that far.
            let hit = hit?;
            let passed = (-medium.sigma_t * reach).exp();
            let chance = (chance * passed).element_sum();
            if chance <= 0.0 {
                return None;
            }
            weight *= passed / chance;
            return Some(Exit { hit, weight, direction });
        }
        // Scattered on a particle: the light that got there and scattered,
        // over the chance of flying exactly that far.
        let passed = (-medium.sigma_t * flight).exp();
        let density = (chance * medium.sigma_t * passed).element_sum();
        if density <= 0.0 {
            return None;
        }
        weight *= sigma_s * passed / density;
        position += direction * flight;
        direction = uniform_sphere(rng.next_f32(), rng.next_f32());
        if step >= ROULETTE_AFTER {
            let keep = weight.max_element().min(0.95);
            if rng.next_f32() >= keep {
                return None;
            }
            weight /= keep;
        }
    }
    None
}

/// A direction spread evenly over all of them.
fn uniform_sphere(u: f32, v: f32) -> Vec3 {
    let z = 1.0 - 2.0 * u;
    let r = (1.0 - z * z).max(0.0).sqrt();
    let phi = std::f32::consts::TAU * v;
    Vec3::new(r * phi.cos(), r * phi.sin(), z)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_inverted_albedo_brings_the_colour_back() {
        // Thick slabs: forward relation 1 − 4.09712 … gives back `a`.
        for a in [0.1f32, 0.5, 0.8, 0.95] {
            let s = single_albedo(a);
            assert!((0.0..1.0).contains(&s), "{s}");
            assert!(s >= a, "single scattering keeps at least as much: {s} vs {a}");
        }
        assert!(single_albedo(0.0) < 0.05);
        let medium = Medium::new(Vec3::new(0.9, 0.5, 0.1), Vec3::new(1.0, 0.5, 0.25));
        assert_eq!(medium.sigma_t, Vec3::new(1.0, 2.0, 4.0));
    }

    #[test]
    fn uniform_directions_are_unit_and_balanced() {
        let mut rng = Rng::new(1, 2, 3, 4);
        let mut sum = Vec3::ZERO;
        for _ in 0..4000 {
            let d = uniform_sphere(rng.next_f32(), rng.next_f32());
            assert!((d.length() - 1.0).abs() < 1e-4);
            sum += d;
        }
        assert!(sum.length() / 4000.0 < 0.05, "{sum}");
    }
}
