//! What surrounds the scene, as viewers see it (see
//! `docs/reference/render-environment.md`): the daylight sky model, the
//! mapping of an equirectangular picture onto directions, and the low-order
//! spherical harmonics that give the viewport its ambient light.
//!
//! The sky is the analytic model of Preetham, Shirley and Smits ("A
//! Practical Analytic Model for Daylight", 1999): the sky's luminance and
//! chromaticity in a direction follow the Perez formula, scaled by the
//! values at the zenith; all of them depend on the turbidity and on how far
//! the sun is from the zenith. The sun's colour is the extraterrestrial
//! light dimmed by the atmosphere's Rayleigh and haze (Ångström) scattering
//! over the air mass along its path, at three wavelengths. Pure functions:
//! no GPU, no scene.
//!
//! Units: 1.0 of irradiance is 100 000 lux (about the sun's, at noon on a
//! clear day), so an Infinite light of intensity 100 % and the Sky's sun
//! high up light a white floor about the same.

use std::f32::consts::{PI, TAU};

use glam::{Mat3, Vec2, Vec3};
use grid4d_scene::{Environment, NodeId, Sky, SunPlacement, TextureId};

/// The real sun's angular radius (radians): 0.2665°.
pub const SUN_RADIUS: f32 = 0.2665 * PI / 180.0;

/// The light of 100 000 lux, in luminance (kcd/m²) seen over a hemisphere:
/// the model's luminance × this = radiance in Grid4D's units.
const LUMINANCE_UNIT: f32 = PI / 100.0;

/// Sunlight outside the atmosphere (128 000 lux) in Grid4D's units.
const SUN_ABOVE_ATMOSPHERE: f32 = 1.28;

/// The Perez coefficients A … E as `a × T + b` for luminance Y and the
/// chromaticities x and y (the paper's appendix).
const PEREZ: [[[f32; 2]; 5]; 3] = [
    [
        [0.1787, -1.4630],
        [-0.3554, 0.4275],
        [-0.0227, 5.3251],
        [0.1206, -2.5771],
        [-0.0670, 0.3703],
    ],
    [
        [-0.0193, -0.2592],
        [-0.0665, 0.0008],
        [-0.0004, 0.2125],
        [-0.0641, -0.8989],
        [-0.0033, 0.0452],
    ],
    [
        [-0.0167, -0.2608],
        [-0.0950, 0.0092],
        [-0.0079, 0.2102],
        [-0.0441, -1.6537],
        [-0.0109, 0.0529],
    ],
];

/// The zenith chromaticities: `[T², T, 1] · M · [θs³, θs², θs, 1]`.
const ZENITH_X: [[f32; 4]; 3] = [
    [0.00166, -0.00375, 0.00209, 0.0],
    [-0.02903, 0.06377, -0.03202, 0.00394],
    [0.11693, -0.21196, 0.06052, 0.25886],
];
const ZENITH_Y: [[f32; 4]; 3] = [
    [0.00275, -0.00610, 0.00317, 0.0],
    [-0.04214, 0.08970, -0.04153, 0.00516],
    [0.15346, -0.26756, 0.06670, 0.26688],
];

/// The Perez formula: the sky's relative value at `cos_theta` from the
/// zenith and `gamma` from the sun, for coefficients `c`.
fn perez(c: &[f32; 5], cos_theta: f32, gamma: f32) -> f32 {
    let cos_gamma = gamma.cos();
    (1.0 + c[0] * (c[1] / cos_theta.max(0.01)).exp())
        * (1.0 + c[2] * (c[3] * gamma).exp() + c[4] * cos_gamma * cos_gamma)
}

/// The model's zenith luminance (kcd/m²) for turbidity `t` and the sun
/// `theta_s` from the zenith.
#[must_use]
pub fn zenith_luminance(t: f32, theta_s: f32) -> f32 {
    let chi = (4.0 / 9.0 - t / 120.0) * (PI - 2.0 * theta_s);
    (4.0453 * t - 4.9710) * chi.tan() - 0.2155 * t + 2.4192
}

/// The model's zenith chromaticity (x, y).
#[must_use]
pub fn zenith_chromaticity(t: f32, theta_s: f32) -> (f32, f32) {
    let ts = [theta_s * theta_s * theta_s, theta_s * theta_s, theta_s, 1.0];
    let tt = [t * t, t, 1.0];
    let value = |m: &[[f32; 4]; 3]| -> f32 {
        tt.iter()
            .zip(m)
            .map(|(a, row)| a * row.iter().zip(&ts).map(|(b, c)| b * c).sum::<f32>())
            .sum()
    };
    (value(&ZENITH_X), value(&ZENITH_Y))
}

/// CIE xyY → linear sRGB (Rec. 709 primaries, D65), negative parts cut.
fn xyy_to_rgb(x: f32, y: f32, luminance: f32) -> Vec3 {
    if y <= 1e-6 || luminance <= 0.0 {
        return Vec3::ZERO;
    }
    let big_x = x / y * luminance;
    let big_z = (1.0 - x - y) / y * luminance;
    Vec3::new(
        3.2406 * big_x - 1.5372 * luminance - 0.4986 * big_z,
        -0.9689 * big_x + 1.8758 * luminance + 0.0415 * big_z,
        0.0557 * big_x - 0.2040 * luminance + 1.0570 * big_z,
    )
    .max(Vec3::ZERO)
}

/// How much of the sun's light gets through the atmosphere at red, green
/// and blue (0.65, 0.55, 0.45 µm), `theta_s` from the zenith (at most 90°).
#[must_use]
pub fn sun_transmittance(turbidity: f32, theta_s: f32) -> Vec3 {
    let theta = theta_s.clamp(0.0, PI / 2.0);
    let degrees = theta.to_degrees();
    // The relative air mass (Kasten and Young).
    let mass = 1.0 / (theta.cos() + 0.15 * (93.885 - degrees).powf(-1.253));
    let beta = (0.046_08 * turbidity - 0.045_86).max(0.0);
    let through = |lambda: f32| {
        let rayleigh = (-0.008_735 * lambda.powf(-4.08) * mass).exp();
        let haze = (-beta * lambda.powf(-1.3) * mass).exp();
        rayleigh * haze
    };
    Vec3::new(through(0.65), through(0.55), through(0.45))
}

/// The sun of a Sky: a small, very bright disc.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sun {
    /// Towards the sun, unit length.
    pub direction: Vec3,
    /// The cosine of the disc's angular radius.
    pub cos_radius: f32,
    /// What the disc gives off, linear RGB.
    pub radiance: Vec3,
    /// How bright it makes a white diffuse surface facing it (radiance × the
    /// disc's solid angle ÷ π): the colour × intensity of an Infinite light
    /// that lights the same.
    pub irradiance: Vec3,
}

impl Sun {
    /// The solid angle the disc fills.
    #[must_use]
    pub fn solid_angle(&self) -> f32 {
        TAU * (1.0 - self.cos_radius)
    }
}

/// A Sky as viewers see it this frame: its model ready to evaluate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyModel {
    /// Towards the sun (unit length), wherever it is, even below the horizon.
    pub sun_direction: Vec3,
    /// Towards the sun the model is evaluated with (never below the horizon).
    model_sun: Vec3,
    /// The Perez coefficients for Y, x, y.
    coefficients: [[f32; 5]; 3],
    /// Zenith value ÷ the Perez value at the zenith, for Y, x, y.
    scale: [f32; 3],
    /// Below the horizon: a dim, uniform ground.
    ground: Vec3,
    /// The model's luminance → radiance (intensity and dusk fade included).
    gain: f32,
    sun: Option<Sun>,
}

impl SkyModel {
    /// The model of `sky`; `object_z` is the object's +Z axis in the world
    /// (its −Z is where the sun shines when the sun is placed by the
    /// object).
    #[must_use]
    pub fn new(sky: &Sky, object_z: Vec3) -> Self {
        let towards = match sky.placement {
            SunPlacement::FromObject => object_z.normalize_or(Vec3::Y),
            SunPlacement::Angles => {
                let (e, a) = (sky.elevation.clamp(Sky::MIN_ELEVATION, Sky::MAX_ELEVATION), sky.azimuth);
                Vec3::new(a.sin() * e.cos(), e.sin(), -a.cos() * e.cos())
            }
        };
        let elevation = towards.y.clamp(-1.0, 1.0).asin();
        let turbidity = if sky.turbidity.is_finite() {
            sky.turbidity.clamp(Sky::MIN_TURBIDITY, Sky::MAX_TURBIDITY)
        } else {
            3.0
        };
        let intensity = sky.intensity.clamp(0.0, Sky::MAX_INTENSITY);
        // The model holds up to the horizon: a lower sun is evaluated there,
        // the sky fading to 2 % by −10°.
        let model_sun = if towards.y >= 0.0 {
            towards
        } else {
            Vec3::new(towards.x, 0.0, towards.z).normalize_or(Vec3::X)
        };
        let theta_s = model_sun.y.clamp(-1.0, 1.0).acos();
        let fade = (1.0 + elevation / (10.0f32).to_radians()).clamp(0.02, 1.0);
        let coefficients = PEREZ.map(|channel| channel.map(|[a, b]| a * turbidity + b));
        let (zx, zy) = zenith_chromaticity(turbidity, theta_s);
        let zenith = [zenith_luminance(turbidity, theta_s).max(0.0), zx, zy];
        let scale = std::array::from_fn(|i| {
            let at_zenith = perez(&coefficients[i], 1.0, theta_s);
            if at_zenith.abs() > 1e-6 {
                zenith[i] / at_zenith
            } else {
                0.0
            }
        });
        let mut model = Self {
            sun_direction: towards,
            model_sun,
            coefficients,
            scale,
            ground: Vec3::ZERO,
            gain: LUMINANCE_UNIT * intensity * if towards.y >= 0.0 { 1.0 } else { fade },
            sun: None,
        };
        // The ground: the horizon's mean colour, dimmed.
        let horizon = (0..8)
            .map(|i| {
                let a = TAU * f32::from(u8::try_from(i).unwrap_or(0)) / 8.0;
                model.above_horizon(Vec3::new(a.cos(), 0.02, a.sin()).normalize())
            })
            .sum::<Vec3>()
            / 8.0;
        model.ground = horizon * 0.3;
        // The sun, gone 2° below the horizon.
        let sun_fade = ((elevation + (2.0f32).to_radians()) / (2.0f32).to_radians()).clamp(0.0, 1.0);
        if sky.sun && sun_fade > 0.0 {
            let size = if sky.sun_size.is_finite() {
                sky.sun_size.clamp(Sky::MIN_SUN_SIZE, Sky::MAX_SUN_SIZE)
            } else {
                1.0
            };
            let cos_radius = (SUN_RADIUS * size).cos();
            let strength =
                SUN_ABOVE_ATMOSPHERE * intensity * sky.sun_intensity.clamp(0.0, Sky::MAX_INTENSITY) * sun_fade;
            let irradiance = sun_transmittance(turbidity, theta_s) * strength;
            let solid_angle = TAU * (1.0 - cos_radius);
            if irradiance.max_element() > 0.0 && solid_angle > 0.0 {
                model.sun = Some(Sun {
                    direction: towards,
                    cos_radius,
                    radiance: irradiance * PI / solid_angle,
                    irradiance,
                });
            }
        }
        model
    }

    /// The sky's light from `direction` (unit length, world), the sun's
    /// disc left out.
    #[must_use]
    pub fn radiance(&self, direction: Vec3) -> Vec3 {
        if direction.y <= 0.0 {
            return self.ground;
        }
        self.above_horizon(direction)
    }

    fn above_horizon(&self, direction: Vec3) -> Vec3 {
        let cos_theta = direction.y.max(0.0);
        let gamma = direction.dot(self.model_sun).clamp(-1.0, 1.0).acos();
        let [luminance, x, y] = std::array::from_fn(|i| self.scale[i] * perez(&self.coefficients[i], cos_theta, gamma));
        xyy_to_rgb(x, y, luminance) * self.gain
    }

    /// The model's luminance (kcd/m²) and chromaticity (x, y) from
    /// `direction` above the horizon, before intensity: what the paper's
    /// formulas give.
    #[must_use]
    pub fn model_values(&self, direction: Vec3) -> [f32; 3] {
        let cos_theta = direction.y.max(0.0);
        let gamma = direction.dot(self.model_sun).clamp(-1.0, 1.0).acos();
        std::array::from_fn(|i| self.scale[i] * perez(&self.coefficients[i], cos_theta, gamma))
    }

    /// The colour below the horizon.
    #[must_use]
    pub const fn ground(&self) -> Vec3 {
        self.ground
    }

    /// The sun, if it is on and above −2°.
    #[must_use]
    pub const fn sun(&self) -> Option<Sun> {
        self.sun
    }

    /// What the viewport's sky shader needs: per channel (Y, x, y) the five
    /// Perez coefficients and the scale, the model's sun direction, the
    /// ground colour and the gain.
    #[must_use]
    pub const fn shader_values(&self) -> ([[f32; 5]; 3], [f32; 3], Vec3, Vec3, f32) {
        (self.coefficients, self.scale, self.model_sun, self.ground, self.gain)
    }
}

/// The direction (in the picture's own frame) that picture point `uv` shows
/// on an equirectangular picture: the centre column along −Z, `u` eastwards
/// (towards +X), `v` from the zenith (top) to the nadir.
#[must_use]
pub fn equirect_direction(uv: Vec2) -> Vec3 {
    let phi = (uv.x - 0.5) * TAU;
    let theta = uv.y.clamp(0.0, 1.0) * PI;
    let (sin_theta, cos_theta) = theta.sin_cos();
    Vec3::new(phi.sin() * sin_theta, cos_theta, -phi.cos() * sin_theta)
}

/// The picture point that shows `direction` (unit length, the picture's own
/// frame); the inverse of [`equirect_direction`], `u` in [0, 1).
#[must_use]
pub fn equirect_uv(direction: Vec3) -> Vec2 {
    let phi = direction.x.atan2(-direction.z);
    let u = phi / TAU + 0.5;
    let v = direction.y.clamp(-1.0, 1.0).acos() / PI;
    Vec2::new(u - u.floor(), v)
}

/// An Environment as viewers see it this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PictureSurround {
    /// Its texture (`None`, or one the document lacks or cannot read: the
    /// uniform grey).
    pub texture: Option<TextureId>,
    pub intensity: f32,
    /// World directions → the picture's frame (its Rotation and the object's
    /// turn undone).
    pub to_picture: Mat3,
}

impl PictureSurround {
    /// `environment` turned by the object's world `rotation` (orthonormal).
    #[must_use]
    pub fn new(environment: &Environment, rotation: Mat3) -> Self {
        let turn = if environment.rotation.is_finite() {
            environment.rotation
        } else {
            0.0
        };
        let world_from_picture = rotation * Mat3::from_rotation_y(turn);
        Self {
            texture: environment.texture,
            intensity: environment.intensity.clamp(0.0, Sky::MAX_INTENSITY),
            to_picture: world_from_picture.transpose(),
        }
    }

    /// The picture point that shows world `direction`.
    #[must_use]
    pub fn uv(&self, direction: Vec3) -> Vec2 {
        equirect_uv(self.to_picture * direction)
    }

    /// The world direction picture point `uv` shows.
    #[must_use]
    pub fn direction(&self, uv: Vec2) -> Vec3 {
        self.to_picture.transpose() * equirect_direction(uv)
    }

    /// The light all round without a picture.
    #[must_use]
    pub fn plain(&self) -> Vec3 {
        Vec3::splat(0.5 * self.intensity)
    }
}

/// What surrounds the scene.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Surround {
    Sky(SkyModel),
    Picture(PictureSurround),
}

/// The Sky or Environment that lights the scene this frame: the topmost
/// visible one in the Object Manager.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EvaluatedEnvironment {
    pub id: NodeId,
    /// Camera rays that miss everything see it.
    pub visible: bool,
    pub surround: Surround,
}

/// The rotation part of a world matrix's upper 3 × 3, its axes made unit
/// length and square to each other.
#[must_use]
pub fn rotation_of(axes: Mat3) -> Mat3 {
    let x = axes.x_axis.normalize_or(Vec3::X);
    let y = (axes.y_axis - x * axes.y_axis.dot(x)).normalize_or(Vec3::Y);
    let z = x.cross(y);
    Mat3::from_cols(x, y, z)
}

/// Low-order spherical harmonics (bands 0 – 2, nine terms) of the light all
/// round, per colour channel: enough for diffuse light.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Ambient {
    /// The radiance's coefficients.
    pub coefficients: [Vec3; 9],
}

/// The nine real spherical harmonics at unit direction `d`.
fn basis(d: Vec3) -> [f32; 9] {
    [
        0.282_095,
        0.488_603 * d.y,
        0.488_603 * d.z,
        0.488_603 * d.x,
        1.092_548 * d.x * d.y,
        1.092_548 * d.y * d.z,
        0.315_392 * (3.0 * d.z * d.z - 1.0),
        1.092_548 * d.x * d.z,
        0.546_274 * (d.x * d.x - d.y * d.y),
    ]
}

/// The convolution with a cosine lobe, per band, over π: the irradiance
/// coefficients ÷ π.
const BAND: [f32; 9] = [1.0, 2.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0, 0.25, 0.25, 0.25, 0.25, 0.25];

impl Ambient {
    /// The harmonics of `radiance` (a world direction's light), measured on
    /// a grid of `rows` × 2 `rows` directions.
    #[must_use]
    pub fn project(rows: u16, radiance: impl Fn(Vec3) -> Vec3) -> Self {
        let rows = rows.max(2);
        let columns = rows * 2;
        let mut coefficients = [Vec3::ZERO; 9];
        let (h, w) = (f32::from(rows), f32::from(columns));
        for row in 0..rows {
            let theta = (f32::from(row) + 0.5) / h * PI;
            let weight = theta.sin() * (PI / h) * (TAU / w);
            for column in 0..columns {
                let phi = (f32::from(column) + 0.5) / w * TAU;
                let d = Vec3::new(phi.cos() * theta.sin(), theta.cos(), phi.sin() * theta.sin());
                let light = radiance(d) * weight;
                for (c, b) in coefficients.iter_mut().zip(basis(d)) {
                    *c += light * b;
                }
            }
        }
        Self { coefficients }
    }

    /// The harmonics of an equirectangular picture `width` × `height` (in
    /// its own frame, see [`equirect_direction`]), every texel by the solid
    /// angle it covers.
    #[must_use]
    pub fn from_equirect(width: u32, height: u32, texel: impl Fn(u32, u32) -> Vec3) -> Self {
        let mut coefficients = [Vec3::ZERO; 9];
        let side = |n: u32| f32::from(u16::try_from(n).unwrap_or(u16::MAX));
        let (w, h) = (side(width.max(1)), side(height.max(1)));
        for y in 0..height {
            let v = (side(y) + 0.5) / h;
            let weight = (v * PI).sin() * (PI / h) * (TAU / w);
            for x in 0..width {
                let d = equirect_direction(Vec2::new((side(x) + 0.5) / w, v));
                let light = texel(x, y) * weight;
                for (c, b) in coefficients.iter_mut().zip(basis(d)) {
                    *c += light * b;
                }
            }
        }
        Self { coefficients }
    }

    /// The same light from everywhere.
    #[must_use]
    pub fn uniform(radiance: Vec3) -> Self {
        let mut coefficients = [Vec3::ZERO; 9];
        // ∫ Y₀₀ over the sphere: 2√π.
        coefficients[0] = radiance * (2.0 * PI.sqrt());
        Self { coefficients }
    }

    /// Every term × `factor`.
    #[must_use]
    pub fn scaled(&self, factor: f32) -> Self {
        Self {
            coefficients: self.coefficients.map(|c| c * factor),
        }
    }

    /// The light a white diffuse surface facing `normal` gives back: the
    /// irradiance there ÷ π.
    #[must_use]
    pub fn diffuse(&self, normal: Vec3) -> Vec3 {
        self.coefficients
            .iter()
            .zip(basis(normal))
            .zip(BAND)
            .map(|((c, b), band)| *c * b * band)
            .sum::<Vec3>()
            .max(Vec3::ZERO)
    }

    /// The coefficients the viewport's shader evaluates: the irradiance's ÷ π,
    /// in the order of [`basis`] (the shader repeats it).
    #[must_use]
    pub fn shader_coefficients(&self) -> [[f32; 4]; 9] {
        std::array::from_fn(|i| (self.coefficients[i] * BAND[i]).extend(0.0).to_array())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn angles(elevation_deg: f32, azimuth_deg: f32, turbidity: f32) -> Sky {
        Sky {
            placement: SunPlacement::Angles,
            elevation: elevation_deg.to_radians(),
            azimuth: azimuth_deg.to_radians(),
            turbidity,
            ..Sky::default()
        }
    }

    #[test]
    fn the_zenith_follows_the_published_formulas() {
        // Worked by hand from the paper's formulas: T = 3, the sun 45° from
        // the zenith — χ = (4/9 − 3/120)(π − π/2) = 0.6589, tan χ = 0.7762,
        // Yz = 7.1649 × 0.7762 − 0.6465 + 2.4192 = 7.334 kcd/m².
        let yz = zenith_luminance(3.0, PI / 4.0);
        assert!((yz - 7.334).abs() < 7.334 * 0.01, "{yz}");
        let (x, y) = zenith_chromaticity(3.0, PI / 4.0);
        // xz = [9, 3, 1] · M_x · [θ³, θ², θ, 1] at θ = π/4.
        let t = PI / 4.0;
        let ts = [t * t * t, t * t, t, 1.0];
        let row = |m: [f32; 4]| m.iter().zip(ts).map(|(a, b)| a * b).sum::<f32>();
        let expected_x = 9.0 * row([0.00166, -0.00375, 0.00209, 0.0])
            + 3.0 * row([-0.02903, 0.06377, -0.03202, 0.00394])
            + row([0.11693, -0.21196, 0.06052, 0.25886]);
        assert!((x - expected_x).abs() < expected_x * 0.01, "{x} vs {expected_x}");
        assert!(
            (0.22..0.32).contains(&x) && (0.22..0.33).contains(&y),
            "a bluish white: {x}, {y}"
        );
        // The model at the zenith is the zenith's own value.
        let model = SkyModel::new(&angles(45.0, 0.0, 3.0), Vec3::Z);
        let [luminance, mx, my] = model.model_values(Vec3::Y);
        assert!((luminance - yz).abs() < yz * 0.01, "{luminance}");
        assert!((mx - x).abs() < 1e-4 && (my - y).abs() < 1e-4);
    }

    #[test]
    fn the_horizon_follows_the_perez_formula() {
        let model = SkyModel::new(&angles(30.0, 90.0, 4.0), Vec3::Z);
        let theta_s = 60.0f32.to_radians();
        let t = 4.0;
        // Low over the horizon opposite the sun.
        let d = Vec3::new(-0.995, 0.1, 0.0).normalize();
        let gamma = d.dot(model.sun_direction).acos();
        let c: [f32; 5] = PEREZ[0].map(|[a, b]| a * t + b);
        let expected = zenith_luminance(t, theta_s) * perez(&c, d.y, gamma) / perez(&c, 1.0, theta_s);
        let [luminance, ..] = model.model_values(d);
        assert!(
            (luminance - expected).abs() < expected * 0.01,
            "{luminance} vs {expected}"
        );
        // The sky is brighter near the sun than away from it, and blue.
        let near = model.radiance(Vec3::new(0.8, 0.6, 0.0).normalize());
        let away = model.radiance(Vec3::new(-0.8, 0.6, 0.0).normalize());
        assert!(near.y > away.y * 1.5, "{near} vs {away}");
        assert!(away.z > away.x, "blue: {away}");
    }

    #[test]
    fn the_sun_reddens_low_and_in_haze() {
        let ratio = |sky: &Sky| {
            let sun = SkyModel::new(sky, Vec3::Z).sun().unwrap();
            sun.irradiance.x / sun.irradiance.z
        };
        let high = ratio(&angles(60.0, 0.0, 3.0));
        let low = ratio(&angles(5.0, 0.0, 3.0));
        let hazy = ratio(&angles(60.0, 0.0, 8.0));
        assert!(low > high * 1.5, "{low} vs {high}");
        assert!(hazy > high, "{hazy} vs {high}");
        // About 100 000 lux at noon on a clear day.
        let noon = SkyModel::new(&angles(90.0, 0.0, 3.0), Vec3::Z).sun().unwrap();
        assert!((0.8..1.1).contains(&noon.irradiance.y), "{}", noon.irradiance);
        // Gone below −2°, and off when switched off.
        assert!(SkyModel::new(&angles(-3.0, 0.0, 3.0), Vec3::Z).sun().is_none());
        let off = Sky {
            sun: false,
            ..angles(30.0, 0.0, 3.0)
        };
        assert!(SkyModel::new(&off, Vec3::Z).sun().is_none());
        // A larger disc gives the same light, spread wider.
        let big = Sky {
            sun_size: 4.0,
            ..angles(60.0, 0.0, 3.0)
        };
        let (a, b) = (
            SkyModel::new(&angles(60.0, 0.0, 3.0), Vec3::Z).sun().unwrap(),
            SkyModel::new(&big, Vec3::Z).sun().unwrap(),
        );
        assert!((a.irradiance - b.irradiance).length() < 1e-5);
        assert!(b.radiance.y < a.radiance.y / 10.0);
    }

    #[test]
    fn the_sun_is_placed_by_angles_or_by_the_object() {
        let east = SkyModel::new(&angles(0.0, 90.0, 3.0), Vec3::Z).sun_direction;
        assert!((east - Vec3::X).length() < 1e-5, "{east}");
        let north = SkyModel::new(&angles(0.0, 0.0, 3.0), Vec3::Z).sun_direction;
        assert!((north - Vec3::NEG_Z).length() < 1e-5, "{north}");
        let up = Vec3::new(0.0, 1.0, 1.0).normalize();
        let by_object = Sky {
            placement: SunPlacement::FromObject,
            ..Sky::default()
        };
        let from_object = SkyModel::new(&by_object, up).sun_direction;
        assert!((from_object - up).length() < 1e-5);
        // Below the horizon: the uniform ground, dimmer than the horizon.
        let model = SkyModel::new(&angles(30.0, 0.0, 3.0), Vec3::Z);
        assert_eq!(model.radiance(Vec3::NEG_Y), model.ground());
        assert!(model.ground().y > 0.0 && model.ground().y < model.radiance(Vec3::new(1.0, 0.05, 0.0).normalize()).y);
    }

    #[test]
    fn equirectangular_points_and_directions_match() {
        assert!((equirect_direction(Vec2::new(0.5, 0.5)) - Vec3::NEG_Z).length() < 1e-6);
        assert!((equirect_direction(Vec2::new(0.75, 0.5)) - Vec3::X).length() < 1e-6);
        assert!((equirect_direction(Vec2::new(0.3, 0.0)) - Vec3::Y).length() < 1e-6);
        for uv in [Vec2::new(0.1, 0.2), Vec2::new(0.6, 0.7), Vec2::new(0.95, 0.5)] {
            let back = equirect_uv(equirect_direction(uv));
            assert!((back - uv).length() < 1e-5, "{uv} → {back}");
        }
        // Turned a quarter about +Y, the picture's −Z looks along −X.
        let environment = Environment {
            rotation: PI / 2.0,
            ..Environment::default()
        };
        let surround = PictureSurround::new(&environment, Mat3::IDENTITY);
        assert!((surround.direction(Vec2::new(0.5, 0.5)) - Vec3::NEG_X).length() < 1e-5);
        assert!((surround.uv(Vec3::NEG_X) - Vec2::new(0.5, 0.5)).length() < 1e-5);
    }

    #[test]
    fn harmonics_give_a_uniform_surround_back_and_light_from_above_more_on_top() {
        let uniform = Ambient::project(16, |_| Vec3::splat(0.7));
        let picture = Ambient::from_equirect(64, 32, |_, _| Vec3::splat(0.7));
        assert!((picture.diffuse(Vec3::new(0.6, 0.0, 0.8)) - Vec3::splat(0.7)).length() < 0.01);
        let exact = Ambient::uniform(Vec3::splat(0.7));
        assert!((exact.diffuse(Vec3::X) - Vec3::splat(0.7)).length() < 1e-5);
        for n in [Vec3::X, Vec3::NEG_Y, Vec3::new(0.3, 0.4, -0.866)] {
            let d = uniform.diffuse(n.normalize());
            assert!((d - Vec3::splat(0.7)).abs().max_element() < 0.01, "{d}");
        }
        let above = Ambient::project(16, |d| Vec3::splat(if d.y > 0.0 { 1.0 } else { 0.0 }));
        let (top, side, bottom) = (
            above.diffuse(Vec3::Y),
            above.diffuse(Vec3::X),
            above.diffuse(Vec3::NEG_Y),
        );
        assert!(
            top.x > 0.9 && (side.x - 0.5).abs() < 0.05 && bottom.x < 0.1,
            "{top} {side} {bottom}"
        );
    }
}
