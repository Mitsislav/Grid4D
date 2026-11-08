//! Lights with a surface — area lights, emitting triangles — and the table
//! that chooses one of them by power for next-event estimation (see
//! `docs/reference/render-lights-area.md`).

use std::f32::consts::{PI, TAU};

use glam::Vec3;
use grid4d_eval::EvaluatedLight;
use grid4d_scene::{AreaShape, LightShadow, LightType};

use crate::shade::luminance;

/// The distance at which an area light of intensity 1 lights a facing
/// surface like an Omni light of intensity 1 with Inverse Square falloff at
/// its default Radius/Decay (500 cm): what makes switching a light's type
/// keep the exposure close.
pub(crate) const AREA_REFERENCE_DISTANCE: f32 = 500.0;

/// An area light, in the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AreaLight {
    pub shape: AreaShape,
    pub centre: Vec3,
    /// Half of the shape along the light's X and Y axes (Rectangle, Disc), or
    /// half the Tube along its axis (`half_x`).
    pub half_x: Vec3,
    pub half_y: Vec3,
    /// Sphere and Tube.
    pub radius: f32,
    /// The direction a flat shape shines in (its −Z).
    pub facing: Vec3,
    pub two_sided: bool,
    /// Camera rays and bounces see it (else it only lights the scene).
    pub visible: bool,
    pub shadow: LightShadow,
    /// The light it gives off, per unit of area and solid angle.
    pub radiance: Vec3,
    /// The area that gives off light.
    pub area: f32,
}

/// A point chosen on a light with a surface.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SurfacePoint {
    pub position: Vec3,
    /// The shape's normal there (outwards for Sphere and Tube, the facing
    /// direction for flat shapes).
    pub normal: Vec3,
    /// The chance of the point per unit of area, given its light was chosen.
    pub density: f32,
}

impl AreaLight {
    /// `light` as an area light; `None` for other types or a shape with no
    /// area.
    pub(crate) fn new(light: &EvaluatedLight) -> Option<Self> {
        if light.light_type != LightType::Area {
            return None;
        }
        let details = &light.details;
        let [size_x, size_y] = details.area_size;
        let along_z = -light.direction * light.z_scale;
        let (half_x, half_y, radius, projected, area) = match details.area_shape {
            AreaShape::Rectangle => {
                let (hx, hy) = (light.x_axis * (size_x * 0.5), light.y_axis * (size_y * 0.5));
                let area = 4.0 * hx.cross(hy).length();
                (hx, hy, 0.0, area, area)
            }
            AreaShape::Disc => {
                let (hx, hy) = (light.x_axis * (size_x * 0.5), light.y_axis * (size_x * 0.5));
                let area = PI * hx.cross(hy).length();
                (hx, hy, 0.0, area, area)
            }
            AreaShape::Sphere => {
                let r = size_x * 0.5 * light.x_axis.length();
                (Vec3::ZERO, Vec3::ZERO, r, PI * r * r, 4.0 * PI * r * r)
            }
            AreaShape::Tube => {
                let r = size_x * 0.5 * light.x_axis.length();
                let half = along_z * (size_y * 0.5);
                let length = 2.0 * half.length();
                (half, Vec3::ZERO, r, 2.0 * r * length, TAU * r * length)
            }
        };
        if !(projected > f32::MIN_POSITIVE && area > f32::MIN_POSITIVE) {
            return None;
        }
        // Intensity 1 at the reference distance, facing, as an Omni light
        // with Inverse Square falloff (radiance × projected area / d² = π × 1).
        let scale = PI * AREA_REFERENCE_DISTANCE * AREA_REFERENCE_DISTANCE / projected;
        Some(Self {
            shape: details.area_shape,
            centre: light.position,
            half_x,
            half_y,
            radius,
            facing: light.direction,
            two_sided: details.two_sided,
            visible: details.area_visible,
            shadow: light.shadow,
            radiance: Vec3::from(light.color) * scale,
            area,
        })
    }

    /// Its power, in the units the light table weighs by (the same as
    /// emitting triangles').
    pub(crate) fn power(&self) -> f32 {
        let sides = if self.shape.is_flat() && self.two_sided {
            2.0
        } else {
            1.0
        };
        PI * luminance(self.radiance) * self.area * sides
    }

    /// Whether multiple importance sampling weighs it: only when bounces can
    /// meet it (visible) and it is stopped by what is in the way (it casts
    /// shadows); otherwise it is only ever sampled directly.
    pub(crate) fn weighed(&self) -> bool {
        self.visible && self.shadow.casts()
    }

    /// A point on it as seen from `from`, from `s` and `t` in [0, 1):
    /// evenly over a flat shape; over the cone of directions a Sphere fills
    /// (none of them hidden behind it); over the half of a Tube that faces
    /// `from`. [`Self::density`] gives the same density for any point.
    pub(crate) fn sample(&self, from: Vec3, s: f32, t: f32) -> SurfacePoint {
        match self.shape {
            AreaShape::Rectangle => SurfacePoint {
                position: self.centre + self.half_x * (2.0 * s - 1.0) + self.half_y * (2.0 * t - 1.0),
                normal: self.facing,
                density: 1.0 / self.area,
            },
            AreaShape::Disc => {
                let (r, phi) = (s.sqrt(), TAU * t);
                SurfacePoint {
                    position: self.centre + self.half_x * (r * phi.cos()) + self.half_y * (r * phi.sin()),
                    normal: self.facing,
                    density: 1.0 / self.area,
                }
            }
            AreaShape::Sphere => {
                let to_centre = self.centre - from;
                let distance = to_centre.length();
                if distance <= self.radius * 1.000_1 {
                    // From inside (or on it): evenly over the whole sphere.
                    let z = 1.0 - 2.0 * s;
                    let ring = (1.0 - z * z).max(0.0).sqrt();
                    let phi = TAU * t;
                    let normal = Vec3::new(ring * phi.cos(), ring * phi.sin(), z);
                    return SurfacePoint {
                        position: self.centre + normal * self.radius,
                        normal,
                        density: 1.0 / self.area,
                    };
                }
                let axis = to_centre / distance;
                let angle = (self.radius / distance).clamp(-1.0, 1.0).asin();
                let w = crate::sample::in_cone(axis, angle, s, t);
                // The nearer point where `w` meets the sphere (on its rim at
                // worst, where rounding makes the ray graze it).
                let b = to_centre.dot(w);
                let c = distance * distance - self.radius * self.radius;
                let along = b - (b * b - c).max(0.0).sqrt();
                let position = from + w * along;
                let normal = ((position - self.centre) / self.radius).normalize_or(-axis);
                SurfacePoint {
                    position,
                    normal,
                    density: self.density(from, position, normal),
                }
            }
            AreaShape::Tube => {
                let axis = self.half_x.normalize_or(Vec3::Z);
                let side = from - self.centre;
                let towards = (side - axis * side.dot(axis))
                    .try_normalize()
                    .unwrap_or_else(|| axis.any_orthonormal_vector());
                let across = axis.cross(towards);
                let phi = PI * (t - 0.5);
                let normal = towards * phi.cos() + across * phi.sin();
                SurfacePoint {
                    position: self.centre + self.half_x * (2.0 * s - 1.0) + normal * self.radius,
                    normal,
                    density: 2.0 / self.area,
                }
            }
        }
    }

    /// The density per unit of area with which [`Self::sample`], seen from
    /// `from`, picks `point` (normal `normal`) of the light.
    pub(crate) fn density(&self, from: Vec3, point: Vec3, normal: Vec3) -> f32 {
        match self.shape {
            AreaShape::Rectangle | AreaShape::Disc => 1.0 / self.area,
            AreaShape::Sphere => {
                let distance = (self.centre - from).length();
                if distance <= self.radius * 1.000_1 {
                    return 1.0 / self.area;
                }
                let cos_max = (1.0 - (self.radius / distance).powi(2)).max(0.0).sqrt();
                let cone = TAU * (1.0 - cos_max);
                let to_point = point - from;
                let distance2 = to_point.length_squared();
                let cos = normal.dot(-to_point).max(0.0) / distance2.sqrt().max(f32::MIN_POSITIVE);
                if cone <= f32::MIN_POSITIVE || distance2 <= f32::MIN_POSITIVE {
                    return 0.0;
                }
                cos / (distance2 * cone)
            }
            AreaShape::Tube => {
                let axis = self.half_x.normalize_or(Vec3::Z);
                let side = from - self.centre;
                let towards = side - axis * side.dot(axis);
                if normal.dot(towards) >= 0.0 {
                    2.0 / self.area
                } else {
                    0.0
                }
            }
        }
    }

    /// Whether its surface with `normal` gives off light towards `towards`
    /// (unit, away from it), and the cosine there.
    pub(crate) fn emits(&self, normal: Vec3, towards: Vec3) -> Option<f32> {
        let cos = normal.dot(towards);
        if self.shape.is_flat() && self.two_sided {
            let cos = cos.abs();
            return (cos > 1e-6).then_some(cos);
        }
        (cos > 1e-6).then_some(cos)
    }

    /// Where the ray `origin + t · dir` meets it before `max_t`: the distance
    /// and the shape's normal there.
    pub(crate) fn intersect(&self, origin: Vec3, dir: Vec3, max_t: f32) -> Option<(f32, Vec3)> {
        let hit = match self.shape {
            AreaShape::Rectangle | AreaShape::Disc => {
                let plane = self.half_x.cross(self.half_y);
                let denominator = dir.dot(plane);
                if denominator.abs() <= f32::MIN_POSITIVE {
                    return None;
                }
                let t = (self.centre - origin).dot(plane) / denominator;
                let d = origin + dir * t - self.centre;
                let u = d.dot(self.half_x) / self.half_x.length_squared();
                let v = d.dot(self.half_y) / self.half_y.length_squared();
                let inside = if self.shape == AreaShape::Rectangle {
                    u.abs() <= 1.0 && v.abs() <= 1.0
                } else {
                    u * u + v * v <= 1.0
                };
                inside.then_some((t, self.facing))
            }
            AreaShape::Sphere => {
                let oc = origin - self.centre;
                let b = oc.dot(dir);
                let c = oc.length_squared() - self.radius * self.radius;
                let discriminant = b * b - c;
                if c <= 0.0 || discriminant < 0.0 {
                    return None; // from inside, or a miss
                }
                let t = -b - discriminant.sqrt();
                Some((t, (origin + dir * t - self.centre) / self.radius))
            }
            AreaShape::Tube => {
                let axis = self.half_x.normalize_or(Vec3::Z);
                let half = self.half_x.length();
                let oc = origin - self.centre;
                let (d_perp, o_perp) = (dir - axis * dir.dot(axis), oc - axis * oc.dot(axis));
                let a = d_perp.length_squared();
                let b = o_perp.dot(d_perp);
                let c = o_perp.length_squared() - self.radius * self.radius;
                let discriminant = b * b - a * c;
                if a <= f32::MIN_POSITIVE || c <= 0.0 || discriminant < 0.0 {
                    return None;
                }
                let t = (-b - discriminant.sqrt()) / a;
                let p = oc + dir * t;
                if p.dot(axis).abs() > half {
                    return None;
                }
                let radial = p - axis * p.dot(axis);
                Some((t, radial / self.radius))
            }
        }?;
        (hit.0 > 0.0 && hit.0 < max_t).then_some(hit)
    }
}

/// What the table can choose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Entry {
    /// An emitting triangle of the scene.
    Triangle(u32),
    /// An area light, by its index among the scene's area lights.
    Area(u32),
}

/// Lights with a surface, chosen by power.
#[derive(Clone, Debug, Default)]
pub(crate) struct LightTable {
    entries: Vec<Entry>,
    /// The running sum of their shares of the total power, ending at 1.
    cumulative: Vec<f32>,
    /// Per triangle of the scene, its chance (empty when none is in it).
    triangle_chance: Vec<f32>,
    /// Per area light, its chance.
    area_chance: Vec<f32>,
}

impl LightTable {
    /// The table of `entries` with their powers (`triangles` and `areas`:
    /// how many of each the scene has).
    pub(crate) fn new(entries: &[(Entry, f32)], triangles: usize, areas: usize) -> Self {
        let total: f32 = entries.iter().map(|&(_, p)| p).sum();
        let mut table = Self::default();
        if entries.is_empty() || total <= 0.0 || !total.is_finite() {
            return table;
        }
        table.area_chance = vec![0.0; areas];
        if entries.iter().any(|(e, _)| matches!(e, Entry::Triangle(_))) {
            table.triangle_chance = vec![0.0; triangles];
        }
        let mut running = 0.0;
        for &(entry, power) in entries {
            let share = power / total;
            let slot = match entry {
                Entry::Triangle(t) => usize::try_from(t).ok().and_then(|t| table.triangle_chance.get_mut(t)),
                Entry::Area(a) => usize::try_from(a).ok().and_then(|a| table.area_chance.get_mut(a)),
            };
            if let Some(slot) = slot {
                *slot = share;
            }
            running += share;
            table.entries.push(entry);
            table.cumulative.push(running);
        }
        if let Some(last) = table.cumulative.last_mut() {
            *last = 1.0;
        }
        table
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entry `u` (in [0, 1)) falls on.
    pub(crate) fn choose(&self, u: f32) -> Option<Entry> {
        let last = self.cumulative.len().checked_sub(1)?;
        let index = self.cumulative.partition_point(|&c| c <= u).min(last);
        self.entries.get(index).copied()
    }

    pub(crate) fn triangle_chance(&self, triangle: usize) -> f32 {
        self.triangle_chance.get(triangle).copied().unwrap_or(0.0)
    }

    pub(crate) fn area_chance(&self, area: usize) -> f32 {
        self.area_chance.get(area).copied().unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test code")]

    use glam::Vec3;
    use grid4d_eval::EvaluatedLight;
    use grid4d_scene::{LightDetails, NodeId};

    use super::*;

    fn area(shape: AreaShape, size: [f32; 2]) -> AreaLight {
        let light = EvaluatedLight {
            id: NodeId::default(),
            light_type: LightType::Area,
            position: Vec3::new(0.0, 100.0, 0.0),
            direction: Vec3::NEG_Y,
            x_axis: Vec3::X,
            y_axis: Vec3::NEG_Z,
            z_scale: 1.0,
            color: [1.0; 3],
            details: LightDetails {
                area_shape: shape,
                area_size: size,
                ..LightDetails::default()
            },
            shadow: LightShadow::Area,
        };
        AreaLight::new(&light).unwrap()
    }

    #[test]
    fn samples_lie_on_the_shape_and_rays_find_it() {
        for shape in AreaShape::ALL {
            let light = area(shape, [40.0, 60.0]);
            let mut rng = crate::sample::Rng::new(1, 2, 3, 4);
            for _ in 0..200 {
                let from = Vec3::new(300.0, -200.0, 100.0);
                let point = light.sample(from, rng.next_f32(), rng.next_f32());
                assert!(point.density > 0.0, "{shape:?}");
                let expected = light.density(from, point.position, point.normal);
                assert!((point.density - expected).abs() <= expected * 1e-3, "{shape:?}");
                // A ray from far along the normal towards the point meets it there.
                let origin = point.position + point.normal * 500.0;
                let (t, normal) = light
                    .intersect(origin, -point.normal, f32::INFINITY)
                    .unwrap_or_else(|| panic!("{shape:?}: {point:?}"));
                assert!((t - 500.0).abs() < 0.05, "{shape:?}: {t}");
                assert!(normal.dot(point.normal) > 0.999);
            }
        }
    }

    #[test]
    fn the_density_over_the_visible_light_adds_up() {
        // Per area, over what the point sees of the light, the density
        // integrates to 1 (estimated with points spread evenly over it).
        for shape in [AreaShape::Sphere, AreaShape::Tube] {
            let light = area(shape, [40.0, 60.0]);
            let from = Vec3::new(30.0, -100.0, 20.0);
            let mut rng = crate::sample::Rng::new(9, 8, 7, 6);
            let count = 200_000;
            let total: f32 = (0..count)
                .map(|_| {
                    // An even point over the whole surface (as 1.1-style sampling).
                    let (s, t) = (rng.next_f32(), rng.next_f32());
                    let (position, normal) = match shape {
                        AreaShape::Sphere => {
                            let z = 1.0 - 2.0 * s;
                            let ring = (1.0 - z * z).max(0.0).sqrt();
                            let n = Vec3::new(ring * (TAU * t).cos(), ring * (TAU * t).sin(), z);
                            (light.centre + n * light.radius, n)
                        }
                        _ => {
                            let axis = light.half_x.normalize();
                            let (e1, e2) = crate::sample::frame(axis);
                            let n = e1 * (TAU * t).cos() + e2 * (TAU * t).sin();
                            (light.centre + light.half_x * (2.0 * s - 1.0) + n * light.radius, n)
                        }
                    };
                    light.density(from, position, normal) * light.area
                })
                .sum::<f32>()
                / count as f32;
            // The Sphere's cone covers what is seen (just under half of it);
            // the Tube's facing half, exactly half.
            assert!((total - 1.0).abs() < 0.02, "{shape:?}: {total}");
        }
    }

    #[test]
    fn areas_match_the_sizes() {
        assert!((area(AreaShape::Rectangle, [40.0, 60.0]).area - 2400.0).abs() < 0.1);
        assert!((area(AreaShape::Disc, [40.0, 60.0]).area - PI * 400.0).abs() < 0.1);
        assert!((area(AreaShape::Sphere, [40.0, 60.0]).area - 4.0 * PI * 400.0).abs() < 0.5);
        assert!((area(AreaShape::Tube, [40.0, 60.0]).area - TAU * 20.0 * 60.0).abs() < 0.5);
    }

    #[test]
    fn one_sided_shapes_shine_one_way() {
        let light = area(AreaShape::Rectangle, [10.0, 10.0]);
        assert!(light.emits(light.facing, Vec3::NEG_Y).is_some());
        assert!(light.emits(light.facing, Vec3::Y).is_none());
        let two = AreaLight {
            two_sided: true,
            ..light
        };
        assert!(two.emits(two.facing, Vec3::Y).is_some());
    }

    #[test]
    fn the_table_chooses_by_power() {
        let table = LightTable::new(&[(Entry::Triangle(1), 1.0), (Entry::Area(0), 3.0)], 3, 1);
        assert_eq!(table.choose(0.1), Some(Entry::Triangle(1)));
        assert_eq!(table.choose(0.5), Some(Entry::Area(0)));
        assert_eq!(table.choose(0.999_99), Some(Entry::Area(0)));
        assert!((table.triangle_chance(1) - 0.25).abs() < 1e-6);
        assert_eq!(table.triangle_chance(0), 0.0);
        assert!((table.area_chance(0) - 0.75).abs() < 1e-6);
        assert!(LightTable::new(&[], 3, 0).is_empty());
    }
}
