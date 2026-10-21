//! How surfaces reflect light (a diffuse part and a GGX highlight) and how much
//! light arrives from each light.

use std::f32::consts::PI;

use glam::Vec3;
use grid4d_scene::{Falloff, LightType};

use crate::{
    sample::{Rng, cosine_hemisphere, frame, in_ball, in_cone},
    scene::{Light, Surface},
};

/// A soft shadow's light is a ball this wide (Omni and Spot), in cm.
const SOFT_LIGHT_RADIUS: f32 = 2.5;
/// A soft Infinite light's half angle, radians (half a degree across).
const SOFT_SUN_ANGLE: f32 = 0.004_363_3;

/// A surface's reflection, ready to evaluate.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Material {
    diffuse: Vec3,
    /// Reflectance straight on.
    f0: Vec3,
    /// GGX width (roughness²), kept above a minimum so point lights stay finite.
    alpha: f32,
}

impl Material {
    pub(crate) fn new(surface: &Surface) -> Self {
        let metallic = surface.metallic.clamp(0.0, 1.0);
        let roughness = surface.roughness.clamp(0.05, 1.0);
        Self {
            diffuse: surface.color * (1.0 - metallic),
            f0: Vec3::splat(0.04).lerp(surface.color, metallic),
            alpha: roughness * roughness,
        }
    }

    /// What light of unit strength from `l` reflects towards `v` (both unit,
    /// away from the surface; normal `n`): the reflectance times π and n · l,
    /// so a white matte surface facing a light of strength 1 shows 1.
    pub(crate) fn reflect(&self, n: Vec3, v: Vec3, l: Vec3) -> Vec3 {
        let n_l = n.dot(l);
        let n_v = n.dot(v).max(1e-4);
        if n_l <= 0.0 {
            return Vec3::ZERO;
        }
        let h = (l + v).normalize_or(n);
        let d = ggx(n.dot(h).max(0.0), self.alpha);
        let g = smith(n_l, n_v, self.alpha);
        let f = schlick(self.f0, v.dot(h).max(0.0));
        let highlight = f * (d * g / (4.0 * n_l * n_v));
        (self.diffuse + highlight * PI) * n_l
    }

    /// A direction to follow after a hit seen from `v`, and the throughput
    /// weight of the path continuing there (reflectance × cos / probability);
    /// `None` when the path ends.
    pub(crate) fn bounce(&self, n: Vec3, v: Vec3, rng: &mut Rng) -> Option<(Vec3, Vec3)> {
        let n_v = n.dot(v).max(1e-4);
        let specular = luminance(schlick(self.f0, n_v));
        let diffuse = luminance(self.diffuse);
        if specular + diffuse <= 0.0 {
            return None;
        }
        let p_specular = (specular / (specular + diffuse)).clamp(0.05, 0.95);
        if rng.next_f32() < p_specular {
            // A microfacet normal drawn with density D(h)·(n·h).
            let (u1, u2) = (rng.next_f32(), rng.next_f32());
            let a2 = self.alpha * self.alpha;
            let cos_theta = ((1.0 - u1) / (1.0 + (a2 - 1.0) * u1)).max(0.0).sqrt();
            let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
            let phi = 2.0 * PI * u2;
            let (t, b) = frame(n);
            let h = (t * (sin_theta * phi.cos()) + b * (sin_theta * phi.sin()) + n * cos_theta).normalize_or(n);
            let v_h = v.dot(h);
            if v_h <= 0.0 {
                return None;
            }
            let l = h * (2.0 * v_h) - v;
            let n_l = n.dot(l);
            if n_l <= 0.0 {
                return None;
            }
            let n_h = n.dot(h).max(1e-4);
            let weight = schlick(self.f0, v_h) * (smith(n_l, n_v, self.alpha) * v_h / (n_v * n_h));
            Some((l, weight / p_specular))
        } else {
            let l = cosine_hemisphere(n, rng.next_f32(), rng.next_f32());
            Some((l, self.diffuse / (1.0 - p_specular)))
        }
    }
}

/// Light arriving at a point from one light.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Incoming {
    /// Towards the light, unit length.
    pub direction: Vec3,
    /// How far the light is (infinite for an Infinite light).
    pub distance: f32,
    pub strength: Vec3,
}

impl Light {
    /// The light arriving at `p` (one sample of a soft light's ball); `None`
    /// where none arrives (outside a spot's cone, beyond a linear falloff).
    pub(crate) fn arriving(&self, p: Vec3, rng: &mut Rng) -> Option<Incoming> {
        let soft = self.shadow.casts() && !self.shadow.is_hard();
        if self.kind == LightType::Infinite {
            let towards = -self.direction;
            let direction = if soft {
                in_cone(towards, SOFT_SUN_ANGLE, rng.next_f32(), rng.next_f32())
            } else {
                towards
            };
            return Some(Incoming {
                direction,
                distance: f32::INFINITY,
                strength: self.color,
            });
        }
        let centre = self.position - p;
        let distance = centre.length().max(1e-3);
        let mut amount = match self.falloff {
            Falloff::None => 1.0,
            Falloff::Linear => (1.0 - distance / self.radius.max(1e-3)).clamp(0.0, 1.0),
            Falloff::InverseSquare => {
                let ratio = self.radius.max(1e-3) / distance;
                (ratio * ratio).min(10_000.0)
            }
        };
        if self.kind == LightType::Spot {
            let cos_angle = (-centre / distance).dot(self.direction);
            amount *= if self.cos_inner > self.cos_outer + 1e-4 {
                smoothstep(self.cos_outer, self.cos_inner, cos_angle)
            } else if cos_angle >= self.cos_outer {
                1.0
            } else {
                0.0
            };
        }
        if amount <= 0.0 {
            return None;
        }
        let to_light = if soft {
            centre + in_ball(rng) * SOFT_LIGHT_RADIUS
        } else {
            centre
        };
        let length = to_light.length().max(1e-3);
        Some(Incoming {
            direction: to_light / length,
            distance: length,
            strength: self.color * amount,
        })
    }
}

/// GGX (Trowbridge-Reitz) distribution of microfacet normals.
fn ggx(n_h: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    let d = n_h * n_h * (a2 - 1.0) + 1.0;
    a2 / (PI * d * d).max(1e-8)
}

/// Smith shadowing-masking (Schlick-GGX form).
fn smith(n_l: f32, n_v: f32, alpha: f32) -> f32 {
    let k = alpha * 0.5;
    let g1 = |x: f32| x / (x * (1.0 - k) + k);
    g1(n_l) * g1(n_v)
}

/// Schlick's Fresnel approximation.
fn schlick(f0: Vec3, cos: f32) -> Vec3 {
    f0 + (Vec3::ONE - f0) * (1.0 - cos).clamp(0.0, 1.0).powi(5)
}

fn luminance(c: Vec3) -> f32 {
    c.dot(Vec3::new(0.2126, 0.7152, 0.0722))
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use grid4d_scene::LightShadow;

    use super::*;

    fn matte(color: Vec3) -> Material {
        Material::new(&Surface {
            color,
            roughness: 1.0,
            metallic: 0.0,
            emission: Vec3::ZERO,
        })
    }

    #[test]
    fn a_white_matte_surface_facing_a_light_shows_about_its_strength() {
        let reflected = matte(Vec3::ONE).reflect(Vec3::Y, Vec3::Y, Vec3::Y);
        assert!((reflected.x - 1.0).abs() < 0.1, "{reflected}");
        // At 60° only half arrives.
        let tilted = Vec3::new(3f32.sqrt() / 2.0, 0.5, 0.0);
        let half = matte(Vec3::ONE).reflect(Vec3::Y, Vec3::Y, tilted);
        assert!((half.x - 0.5).abs() < 0.1, "{half}");
        assert_eq!(matte(Vec3::ONE).reflect(Vec3::Y, Vec3::Y, Vec3::NEG_Y), Vec3::ZERO);
    }

    #[test]
    fn metals_reflect_in_their_colour_and_have_no_diffuse_part() {
        let gold = Material::new(&Surface {
            color: Vec3::new(1.0, 0.8, 0.3),
            roughness: 0.3,
            metallic: 1.0,
            emission: Vec3::ZERO,
        });
        assert_eq!(gold.diffuse, Vec3::ZERO);
        // The mirror direction: the highlight, tinted.
        let v = Vec3::new(0.5, 1.0, 0.0).normalize();
        let l = Vec3::new(-0.5, 1.0, 0.0).normalize();
        let highlight = gold.reflect(Vec3::Y, v, l);
        assert!(highlight.x > highlight.z * 2.0, "{highlight}");
    }

    #[test]
    fn bounces_stay_above_the_surface_and_matte_weights_are_the_colour() {
        let grey = matte(Vec3::splat(0.5));
        let mut rng = Rng::new(0, 0, 0, 1);
        for _ in 0..200 {
            if let Some((l, weight)) = grey.bounce(Vec3::Y, Vec3::Y, &mut rng) {
                assert!(l.y > -1e-4);
                assert!(weight.max_element() < 20.0, "{weight}");
            }
        }
    }

    #[test]
    fn spots_light_inside_their_cone_and_falloff_dims() {
        let spot = Light {
            kind: LightType::Spot,
            position: Vec3::new(0.0, 100.0, 0.0),
            direction: Vec3::NEG_Y,
            color: Vec3::ONE,
            cos_outer: 30f32.to_radians().cos(),
            cos_inner: 30f32.to_radians().cos(),
            falloff: Falloff::Linear,
            radius: 200.0,
            shadow: LightShadow::None,
        };
        let mut rng = Rng::new(0, 0, 0, 0);
        let below = spot.arriving(Vec3::ZERO, &mut rng).unwrap();
        assert!(below.direction.abs_diff_eq(Vec3::Y, 1e-5));
        assert!((below.strength.x - 0.5).abs() < 1e-4, "half way to the radius");
        assert!(spot.arriving(Vec3::new(100.0, 0.0, 0.0), &mut rng).is_none(), "45° out");
    }
}
