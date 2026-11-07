//! How surfaces reflect light (a diffuse part and a GGX highlight, glass that
//! refracts, a clear coat over them) and how much light arrives from each
//! light.

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

/// Where a path goes after a hit.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Bounce {
    /// Unit length.
    pub direction: Vec3,
    /// The path's throughput weight (reflectance × cos / probability).
    pub weight: Vec3,
    /// Through the surface (refracted) rather than back from it.
    pub through: bool,
}

/// A surface's reflection, ready to evaluate: an opaque part (a diffuse
/// layer and a GGX highlight), a part that lets light through (glass) and a
/// clear coat over both.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Material {
    /// The opaque part's diffuse colour.
    diffuse: Vec3,
    /// Reflectance straight on.
    f0: Vec3,
    /// GGX width (roughness²), kept above a minimum so point lights stay finite.
    alpha: f32,
    /// How much of the surface is glass (0 – 1).
    glass: f32,
    /// What colours the light that passes through.
    tint: Vec3,
    ior: f32,
    coat: f32,
    coat_alpha: f32,
}

/// A clear coat reflects like glass of index 1.5: 4 % straight on.
const COAT_F0: f32 = 0.04;

impl Material {
    pub(crate) fn new(surface: &Surface) -> Self {
        let metallic = surface.metallic.clamp(0.0, 1.0);
        let roughness = surface.roughness.clamp(0.05, 1.0);
        let glass = surface.transmitted();
        let opaque = 1.0 - glass;
        // Of the opaque part, the share that is a matte dielectric.
        let dielectric = (1.0 - metallic) * (1.0 - surface.transmission.clamp(0.0, 1.0));
        let coat_roughness = surface.clearcoat_roughness.clamp(0.05, 1.0);
        Self {
            diffuse: if opaque > 1e-6 {
                surface.color * (dielectric / opaque)
            } else {
                Vec3::ZERO
            },
            f0: Vec3::splat(0.08 * surface.specular.clamp(0.0, 1.0)).lerp(surface.color, metallic),
            alpha: roughness * roughness,
            glass,
            tint: surface.color,
            ior: surface.ior.clamp(1.0, 3.0),
            coat: surface.clearcoat.clamp(0.0, 1.0),
            coat_alpha: coat_roughness * coat_roughness,
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
        let (n_h, v_h) = (n.dot(h).max(0.0), v.dot(h).max(0.0));
        let d = ggx(n_h, self.alpha);
        let g = smith(n_l, n_v, self.alpha);
        let f = schlick(self.f0, v_h);
        let highlight = f * (d * g / (4.0 * n_l * n_v));
        let mut reflected = (self.diffuse + highlight * PI) * n_l;
        if self.glass > 0.0 {
            // Glass gives back only its reflection (light passing through it
            // arrives along the paths that refract).
            let glass = fresnel_dielectric(v_h, 1.0 / self.ior) * d * g / (4.0 * n_l * n_v) * PI * n_l;
            reflected = reflected * (1.0 - self.glass) + Vec3::splat(glass * self.glass);
        }
        if self.coat > 0.0 {
            let dc = ggx(n_h, self.coat_alpha);
            let gc = smith(n_l, n_v, self.coat_alpha);
            let coat = self.coat * schlick1(COAT_F0, v_h) * dc * gc / (4.0 * n_l * n_v) * PI * n_l;
            reflected = reflected * (1.0 - self.coat * schlick1(COAT_F0, n_v)) + Vec3::splat(coat);
        }
        reflected
    }

    /// Where a path seen from `v` goes next (`entering`: the ray met the
    /// surface's front, so a refracted path goes into the object); `None`
    /// when it ends.
    pub(crate) fn bounce(&self, n: Vec3, v: Vec3, entering: bool, rng: &mut Rng) -> Option<Bounce> {
        let n_v = n.dot(v).max(1e-4);
        // The coat first: chosen as often as it reflects.
        let mut scale = 1.0;
        if self.coat > 0.0 {
            let reflects = self.coat * schlick1(COAT_F0, n_v);
            let p_coat = reflects.clamp(0.0, 0.95);
            if rng.next_f32() < p_coat {
                let (l, weight, v_h) = mirror(n, v, self.coat_alpha, rng)?;
                let fresnel = schlick1(COAT_F0, v_h);
                return Some(Bounce {
                    direction: l,
                    weight: Vec3::splat(weight * fresnel * self.coat / p_coat),
                    through: false,
                });
            }
            scale = (1.0 - reflects) / (1.0 - p_coat);
        }
        if self.glass > 0.0 && rng.next_f32() < self.glass {
            return self.glass_bounce(n, v, entering, rng).map(|b| Bounce {
                weight: b.weight * scale,
                ..b
            });
        }
        let specular = luminance(schlick(self.f0, n_v));
        let diffuse = luminance(self.diffuse);
        if specular + diffuse <= 0.0 {
            return None;
        }
        let p_specular = (specular / (specular + diffuse)).clamp(0.05, 0.95);
        let (direction, weight) = if rng.next_f32() < p_specular {
            let (l, weight, v_h) = mirror(n, v, self.alpha, rng)?;
            (l, schlick(self.f0, v_h) * weight / p_specular)
        } else {
            let l = cosine_hemisphere(n, rng.next_f32(), rng.next_f32());
            (l, self.diffuse / (1.0 - p_specular))
        };
        Some(Bounce {
            direction,
            weight: weight * scale,
            through: false,
        })
    }

    /// The glass part: reflected or refracted about a microfacet normal, as
    /// often as Fresnel says.
    fn glass_bounce(&self, n: Vec3, v: Vec3, entering: bool, rng: &mut Rng) -> Option<Bounce> {
        let h = microfacet(n, self.alpha, rng);
        let v_h = v.dot(h);
        if v_h <= 0.0 {
            return None;
        }
        let n_v = n.dot(v).max(1e-4);
        let n_h = n.dot(h).max(1e-4);
        let eta = if entering { 1.0 / self.ior } else { self.ior };
        let fresnel = fresnel_dielectric(v_h, eta);
        if rng.next_f32() < fresnel {
            let l = h * (2.0 * v_h) - v;
            let n_l = n.dot(l);
            if n_l <= 0.0 {
                return None;
            }
            let weight = smith(n_l, n_v, self.alpha) * v_h / (n_v * n_h);
            return Some(Bounce {
                direction: l,
                weight: Vec3::splat(weight),
                through: false,
            });
        }
        let cos_t = (1.0 - eta * eta * (1.0 - v_h * v_h)).max(0.0).sqrt();
        let l = (h * (eta * v_h - cos_t) - v * eta).normalize_or(-v);
        let n_l = n.dot(l);
        if n_l >= 0.0 {
            return None;
        }
        let weight = smith(-n_l, n_v, self.alpha) * v_h / (n_v * n_h);
        Some(Bounce {
            direction: l,
            weight: self.tint * weight,
            through: true,
        })
    }
}

/// A GGX microfacet normal around `n`, drawn with density D(h)·(n·h).
fn microfacet(n: Vec3, alpha: f32, rng: &mut Rng) -> Vec3 {
    let (u1, u2) = (rng.next_f32(), rng.next_f32());
    let a2 = alpha * alpha;
    let cos_theta = ((1.0 - u1) / (1.0 + (a2 - 1.0) * u1)).max(0.0).sqrt();
    let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
    let phi = 2.0 * PI * u2;
    let (t, b) = frame(n);
    (t * (sin_theta * phi.cos()) + b * (sin_theta * phi.sin()) + n * cos_theta).normalize_or(n)
}

/// A direction reflected about a GGX microfacet normal, its weight without
/// Fresnel (G · v·h / (n·v · n·h)) and v·h.
fn mirror(n: Vec3, v: Vec3, alpha: f32, rng: &mut Rng) -> Option<(Vec3, f32, f32)> {
    let h = microfacet(n, alpha, rng);
    let v_h = v.dot(h);
    if v_h <= 0.0 {
        return None;
    }
    let l = h * (2.0 * v_h) - v;
    let n_l = n.dot(l);
    if n_l <= 0.0 {
        return None;
    }
    let n_v = n.dot(v).max(1e-4);
    let n_h = n.dot(h).max(1e-4);
    Some((l, smith(n_l, n_v, alpha) * v_h / (n_v * n_h), v_h))
}

/// The share of light a dielectric reflects at incidence cosine `cos_i`,
/// `eta` the index it comes from over the one it meets (1 past total internal
/// reflection).
fn fresnel_dielectric(cos_i: f32, eta: f32) -> f32 {
    let cos_i = cos_i.clamp(0.0, 1.0);
    let sin_t2 = eta * eta * (1.0 - cos_i * cos_i);
    if sin_t2 >= 1.0 {
        return 1.0;
    }
    let cos_t = (1.0 - sin_t2).sqrt();
    let s = (eta * cos_i - cos_t) / (eta * cos_i + cos_t);
    let p = (cos_i - eta * cos_t) / (cos_i + eta * cos_t);
    0.5 * (s * s + p * p)
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

/// Schlick's approximation for one reflectance.
fn schlick1(f0: f32, cos: f32) -> f32 {
    f0 + (1.0 - f0) * (1.0 - cos).clamp(0.0, 1.0).powi(5)
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
        Material::new(&Surface::plain(color, 1.0, 0.0, Vec3::ZERO))
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
        let gold = Material::new(&Surface::plain(Vec3::new(1.0, 0.8, 0.3), 0.3, 1.0, Vec3::ZERO));
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
            if let Some(bounce) = grey.bounce(Vec3::Y, Vec3::Y, true, &mut rng) {
                assert!(bounce.direction.y > -1e-4 && !bounce.through);
                assert!(bounce.weight.max_element() < 20.0, "{}", bounce.weight);
            }
        }
    }

    #[test]
    fn glass_refracts_by_its_index_and_tints_what_passes() {
        let mut surface = Surface::plain(Vec3::new(1.0, 0.5, 0.5), 0.05, 0.0, Vec3::ZERO);
        surface.transmission = 1.0;
        surface.ior = 1.5;
        let glass = Material::new(&surface);
        assert_eq!(glass.diffuse, Vec3::ZERO, "no matte part");
        // 45° in: Snell gives sin 45° / 1.5 inside, mostly straight on.
        let v = Vec3::new(-1.0, 1.0, 0.0).normalize();
        let mut rng = Rng::new(1, 2, 3, 4);
        let (mut through, mut back) = (0, 0);
        for _ in 0..400 {
            let Some(bounce) = glass.bounce(Vec3::Y, v, true, &mut rng) else {
                continue;
            };
            if bounce.through {
                through += 1;
                let sin = bounce.direction.x;
                assert!(
                    bounce.direction.y < 0.0 && (sin - 0.4714).abs() < 0.05,
                    "{}",
                    bounce.direction
                );
                assert!(bounce.weight.y < bounce.weight.x, "tinted: {}", bounce.weight);
            } else {
                back += 1;
                assert!(bounce.direction.y > 0.0);
            }
        }
        assert!(through > 300 && back > 0, "about 5 % reflects: {through} {back}");
        // Leaving at a grazing angle: totally reflected.
        let grazing = Vec3::new(-0.9, 0.1, 0.0).normalize();
        for _ in 0..50 {
            if let Some(bounce) = glass.bounce(Vec3::Y, grazing, false, &mut rng) {
                assert!(!bounce.through);
            }
        }
        // Lights do not shine through it, but its reflection shows.
        let mirror_l = Vec3::new(1.0, 1.0, 0.0).normalize();
        let highlight = glass.reflect(Vec3::Y, v, mirror_l);
        assert!(highlight.x > 0.0 && highlight.x == highlight.y, "white: {highlight}");
    }

    #[test]
    fn a_clear_coat_adds_a_sharp_white_highlight() {
        let mut surface = Surface::plain(Vec3::new(0.0, 0.0, 0.5), 1.0, 0.0, Vec3::ZERO);
        let plain = Material::new(&surface);
        surface.clearcoat = 1.0;
        let coated = Material::new(&surface);
        let v = Vec3::new(0.5, 1.0, 0.0).normalize();
        let l = Vec3::new(-0.5, 1.0, 0.0).normalize();
        let (a, b) = (plain.reflect(Vec3::Y, v, l), coated.reflect(Vec3::Y, v, l));
        assert!(b.x > a.x * 10.0 + 0.1, "{a} vs {b}");
        // Off the mirror direction the coat dims the base a little.
        let off = Vec3::new(0.9, 0.3, 0.0).normalize();
        assert!(coated.reflect(Vec3::Y, v, off).z <= plain.reflect(Vec3::Y, v, off).z);
        let mut rng = Rng::new(0, 0, 0, 9);
        let reflected = (0..400)
            .filter_map(|_| coated.bounce(Vec3::Y, v, true, &mut rng))
            .filter(|b| b.weight.x > 0.0)
            .count();
        assert!(reflected > 0, "some paths take the coat");
    }

    #[test]
    fn specular_sets_a_dielectric_s_reflection() {
        let mut surface = Surface::plain(Vec3::ONE, 0.5, 0.0, Vec3::ZERO);
        assert!((Material::new(&surface).f0.x - 0.04).abs() < 1e-6);
        surface.specular = 1.0;
        assert!((Material::new(&surface).f0.x - 0.08).abs() < 1e-6);
        assert!((fresnel_dielectric(1.0, 1.0 / 1.5) - 0.04).abs() < 1e-4);
        assert_eq!(fresnel_dielectric(0.1, 1.5), 1.0, "total internal reflection");
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
