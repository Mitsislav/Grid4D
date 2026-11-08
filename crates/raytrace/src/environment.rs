//! What surrounds the scene, as the path tracer sees it (see
//! `docs/reference/render-environment.md`): a Sky (its model and its sun) or
//! an Environment picture, met by rays that leave the scene and sampled as a
//! light — the sun as a small disc, the rest by importance, from a table of
//! its brightness over an equirectangular grid of directions, so a bright
//! patch (a window, the sun in a photograph) is chosen often and its
//! shadows come out clean.

use std::{
    f32::consts::{PI, TAU},
    sync::Arc,
};

use glam::{Vec2, Vec3};
use grid4d_eval::{EvaluatedEnvironment, PictureSurround, SkyModel, Sun, Surround, sky};

use crate::{Picture, Pictures, sample::in_cone, shade::luminance};

/// The importance table's widest side, in cells (a larger picture is read
/// in blocks, each cell its block's brightest texel).
const MAX_COLUMNS: u32 = 1024;
/// The Sky's table: its model is smooth.
const SKY_COLUMNS: u32 = 128;

/// Where the surround's light comes from.
#[derive(Debug)]
enum Source {
    Sky(SkyModel),
    Picture {
        picture: Arc<Picture>,
        intensity: f32,
        surround: PictureSurround,
    },
    /// No picture: the same light from everywhere.
    Uniform(Vec3),
}

/// A distribution over the cells of a `columns` × `rows` grid by their
/// weights: rows chosen by their sums, then a cell within the row.
#[derive(Debug)]
struct Table {
    columns: u32,
    rows: u32,
    /// Per row, the running sum of its cells (`columns + 1` each, from 0).
    within: Vec<f32>,
    /// The running sum of the rows' sums (`rows + 1`, from 0).
    across: Vec<f32>,
}

impl Table {
    /// `None` when every weight is 0 (or not a number).
    fn new(columns: u32, rows: u32, weight: impl Fn(u32, u32) -> f32) -> Option<Self> {
        let width = usize::try_from(columns).ok()?;
        let mut within = Vec::with_capacity((width + 1) * usize::try_from(rows).ok()?);
        let mut across = Vec::with_capacity(usize::try_from(rows).ok()? + 1);
        across.push(0.0f32);
        for row in 0..rows {
            let mut sum = 0.0f32;
            within.push(0.0);
            for column in 0..columns {
                let w = weight(column, row);
                sum += if w.is_finite() { w.max(0.0) } else { 0.0 };
                within.push(sum);
            }
            let total = across.last().copied().unwrap_or(0.0) + sum;
            across.push(total);
        }
        let total = across.last().copied().unwrap_or(0.0);
        (total > 0.0 && total.is_finite()).then_some(Self {
            columns,
            rows,
            within,
            across,
        })
    }

    fn total(&self) -> f32 {
        self.across.last().copied().unwrap_or(0.0)
    }

    /// The row of `cells`' running sums (`columns + 1` values).
    fn row(&self, row: usize) -> &[f32] {
        let width = usize::try_from(self.columns).unwrap_or(0) + 1;
        self.within.get(row * width..(row + 1) * width).unwrap_or(&[])
    }

    /// A point of the unit square by the cells' weights (`s` picks the row,
    /// `t` the cell in it, both then spread over the cell), and its density
    /// over the square.
    fn sample(&self, s: f32, t: f32) -> Option<(Vec2, f32)> {
        let total = self.total();
        let (row, s_in) = pick(&self.across, s * total)?;
        let cells = self.row(row);
        let row_sum = cells.last().copied().unwrap_or(0.0);
        let (column, t_in) = pick(cells, t * row_sum)?;
        let (w, h) = (count(self.columns), count(self.rows));
        let uv = Vec2::new((to_f32(column) + t_in) / w, (to_f32(row) + s_in) / h);
        Some((uv, self.density(column, row)))
    }

    /// The density over the unit square of cell (`column`, `row`).
    fn density(&self, column: usize, row: usize) -> f32 {
        let cells = self.row(row);
        let (Some(a), Some(b)) = (cells.get(column), cells.get(column + 1)) else {
            return 0.0;
        };
        (b - a) / self.total() * count(self.columns) * count(self.rows)
    }

    /// The density over the unit square at `uv`.
    fn density_at(&self, uv: Vec2) -> f32 {
        let column = cell(uv.x, self.columns);
        let row = cell(uv.y, self.rows);
        self.density(column, row)
    }
}

/// The interval of running sums `sums` that `target` falls in, and where in
/// it (0 – 1); empty intervals are never chosen.
fn pick(sums: &[f32], target: f32) -> Option<(usize, f32)> {
    let last = sums.len().checked_sub(2)?;
    // The first end above the target.
    let end = sums.partition_point(|&v| v <= target).clamp(1, last + 1);
    let mut index = end - 1;
    // Rounding at the top: step back over empty intervals.
    while index > 0 && sums[index + 1] <= sums[index] {
        index -= 1;
    }
    let (a, b) = (sums[index], sums[index + 1]);
    if b <= a {
        return None;
    }
    Some((index, ((target - a) / (b - a)).clamp(0.0, 0.999_999)))
}

/// The cell of `n` that coordinate `x` (0 – 1) is in.
fn cell(x: f32, n: u32) -> usize {
    let i = (x.clamp(0.0, 0.999_999) * count(n)).floor();
    // A cell index below 2²⁴: exact in an f32.
    let i = if i.is_finite() { i.max(0.0) } else { 0.0 };
    usize::try_from(n.saturating_sub(1)).map_or(0, |last| (i as usize).min(last))
}

fn count(n: u32) -> f32 {
    to_f32(usize::try_from(n).unwrap_or(0))
}

/// A small count as a float (table sizes stay under 2¹⁶).
fn to_f32(n: usize) -> f32 {
    u16::try_from(n).map_or(f32::from(u16::MAX), f32::from)
}

/// The surround of one render.
#[derive(Debug)]
pub(crate) struct EnvironmentLight {
    source: Source,
    /// Camera rays that miss everything see it.
    pub visible: bool,
    /// Importance over the equirectangular grid (the sun left out); `None`:
    /// even over the sphere.
    table: Option<Table>,
    sun: Option<Sun>,
    /// The chance of sampling the sun rather than the rest.
    sun_chance: f32,
}

impl EnvironmentLight {
    /// The surround `environment` describes; an Environment reads its
    /// picture from `pictures` (without it, or while it is missing: the
    /// uniform grey).
    pub(crate) fn new(environment: &EvaluatedEnvironment, pictures: Option<&Pictures>) -> Self {
        let source = match environment.surround {
            Surround::Sky(model) => Source::Sky(model),
            Surround::Picture(surround) => match surround.texture.and_then(|id| pictures?.get(id)) {
                Some(picture) => Source::Picture {
                    picture: Arc::clone(picture),
                    intensity: surround.intensity,
                    surround,
                },
                None => Source::Uniform(surround.plain()),
            },
        };
        let table = match &source {
            Source::Sky(model) => Table::new(SKY_COLUMNS, SKY_COLUMNS / 2, |column, row| {
                let uv = centre(column, row, SKY_COLUMNS, SKY_COLUMNS / 2);
                luminance(model.radiance(sky::equirect_direction(uv))) * (uv.y * PI).sin()
            }),
            Source::Picture { picture, .. } => picture_table(picture),
            Source::Uniform(_) => None,
        };
        let sun = match &source {
            Source::Sky(model) => model.sun(),
            Source::Picture { .. } | Source::Uniform(_) => None,
        };
        // By how bright they make a white surface facing them: the sun's
        // against the rest's (about its mean brightness; a cell's weight
        // carries its row's sine, whose mean is 2 / π).
        let rest = match (&source, &table) {
            (Source::Uniform(light), _) => luminance(*light),
            (Source::Sky(_), Some(table)) => table.total() / (count(table.columns) * count(table.rows)) * PI / 2.0,
            (Source::Picture { intensity, .. }, Some(table)) => {
                table.total() / (count(table.columns) * count(table.rows)) * intensity * PI / 2.0
            }
            (_, None) => 0.0,
        };
        let sun_chance = sun.map_or(0.0, |sun| {
            let own = luminance(sun.irradiance);
            if own + rest > 0.0 {
                (own / (own + rest)).clamp(0.1, 0.9)
            } else {
                0.5
            }
        });
        Self {
            source,
            visible: environment.visible,
            table,
            sun,
            sun_chance,
        }
    }

    /// The light coming from `direction` (unit length, world), the sun's
    /// disc included.
    pub(crate) fn radiance(&self, direction: Vec3) -> Vec3 {
        let around = match &self.source {
            Source::Sky(model) => model.radiance(direction),
            Source::Picture {
                picture,
                intensity,
                surround,
            } => picture.sample(surround.uv(direction)).truncate() * *intensity,
            Source::Uniform(light) => *light,
        };
        match self.sun {
            Some(sun) if direction.dot(sun.direction) >= sun.cos_radius => around + sun.radiance,
            _ => around,
        }
    }

    /// A direction towards the surround, chosen with `u` (sun or rest), `s`
    /// and `t`; its light and its density per solid angle.
    pub(crate) fn sample(&self, u: f32, s: f32, t: f32) -> Option<(Vec3, Vec3, f32)> {
        let direction = match self.sun {
            Some(sun) if u < self.sun_chance => in_cone(sun.direction, sun.cos_radius.acos(), s, t),
            _ => match &self.table {
                Some(table) => {
                    let (uv, _) = table.sample(s, t)?;
                    self.to_world(sky::equirect_direction(uv))
                }
                None => {
                    // Even over the sphere.
                    let z = 1.0 - 2.0 * s;
                    let ring = (1.0 - z * z).max(0.0).sqrt();
                    Vec3::new(ring * (TAU * t).cos(), z, ring * (TAU * t).sin())
                }
            },
        };
        let density = self.density(direction);
        (density > 0.0 && density.is_finite()).then(|| (direction, self.radiance(direction), density))
    }

    /// The density per solid angle with which [`Self::sample`] picks
    /// `direction`.
    pub(crate) fn density(&self, direction: Vec3) -> f32 {
        let sun = match self.sun {
            Some(sun) if direction.dot(sun.direction) >= sun.cos_radius => self.sun_chance / sun.solid_angle(),
            _ => 0.0,
        };
        let rest = match &self.table {
            Some(table) => {
                let uv = sky::equirect_uv(self.to_table(direction));
                let sin_theta = (uv.y * PI).sin();
                if sin_theta <= 1e-6 {
                    0.0
                } else {
                    table.density_at(uv) / (2.0 * PI * PI * sin_theta)
                }
            }
            None => 1.0 / (4.0 * PI),
        };
        sun + (1.0 - self.sun_chance) * rest
    }

    /// World direction → the table's frame (the picture's own; the world's
    /// for a sky).
    fn to_table(&self, direction: Vec3) -> Vec3 {
        match &self.source {
            Source::Picture { surround, .. } => surround.to_picture * direction,
            Source::Sky(_) | Source::Uniform(_) => direction,
        }
    }

    fn to_world(&self, direction: Vec3) -> Vec3 {
        match &self.source {
            Source::Picture { surround, .. } => surround.to_picture.transpose() * direction,
            Source::Sky(_) | Source::Uniform(_) => direction,
        }
    }
}

/// The centre of cell (`column`, `row`) of a `columns` × `rows` grid over
/// the unit square.
fn centre(column: u32, row: u32, columns: u32, rows: u32) -> Vec2 {
    let c = |i: u32, n: u32| (count(i) + 0.5) / count(n);
    Vec2::new(c(column, columns), c(row, rows))
}

/// The importance table of an equirectangular picture: each cell the
/// brightest texel of its block and of its neighbours' (the picture is read
/// between texels, so a texel's light reaches into the next cells), weighed
/// by the area its row covers, with a little for every cell so no direction
/// is never chosen.
fn picture_table(picture: &Picture) -> Option<Table> {
    let (width, height) = picture.size();
    let columns = width.min(MAX_COLUMNS);
    let rows = height.clamp(1, MAX_COLUMNS / 2);
    let (bx, by) = (width.div_ceil(columns), height.div_ceil(rows));
    let mut brightest = vec![0.0f32; usize::try_from(u64::from(columns) * u64::from(rows)).ok()?];
    let width_usize = usize::try_from(columns).ok()?;
    for y in 0..height {
        let row = usize::try_from((y / by).min(rows - 1)).ok()?;
        for x in 0..width {
            let column = usize::try_from((x / bx).min(columns - 1)).ok()?;
            let texel = picture.texel_at(x, y);
            let value = luminance(texel.truncate());
            let slot = &mut brightest[row * width_usize + column];
            if value > *slot {
                *slot = value;
            }
        }
    }
    let height_usize = usize::try_from(rows).ok()?;
    let at = |column: usize, row: usize| brightest.get(row * width_usize + column).copied().unwrap_or(0.0);
    // The 3 × 3 neighbourhood's brightest, round the picture's seam.
    let mut spread = vec![0.0f32; brightest.len()];
    for row in 0..height_usize {
        for column in 0..width_usize {
            let mut most = 0.0f32;
            for r in row.saturating_sub(1)..=(row + 1).min(height_usize - 1) {
                for c in [column + width_usize - 1, column, column + 1] {
                    most = most.max(at(c % width_usize, r));
                }
            }
            spread[row * width_usize + column] = most;
        }
    }
    let mean = spread.iter().sum::<f32>() / count(columns) / count(rows);
    let floor = mean * 1e-3 + 1e-6;
    Table::new(columns, rows, |column, row| {
        let index = usize::try_from(row).unwrap_or(0) * width_usize + usize::try_from(column).unwrap_or(0);
        let v = (count(row) + 0.5) / count(rows);
        (spread.get(index).copied().unwrap_or(0.0) + floor) * (v * PI).sin()
    })
}

#[cfg(test)]
mod tests {
    use glam::Mat3;
    use grid4d_scene::{Environment, NodeId, Sky, SunPlacement};

    use super::*;
    use crate::{Texels, sample::Rng};

    fn environment(surround: Surround) -> EvaluatedEnvironment {
        EvaluatedEnvironment {
            id: NodeId::default(),
            visible: true,
            surround,
        }
    }

    /// A picture `w` × `h`, black but for a bright texel at (`x`, `y`).
    fn one_bright(w: u32, h: u32, x: u32, y: u32, value: f32) -> Picture {
        let mut values = vec![half::f16::ZERO; usize::try_from(w * h * 4).unwrap()];
        for i in 0..usize::try_from(w * h).unwrap() {
            values[i * 4 + 3] = half::f16::ONE;
        }
        let i = usize::try_from((y * w + x) * 4).unwrap();
        for c in 0..3 {
            values[i + c] = half::f16::from_f32(value);
        }
        Picture::new(w, h, Texels::Half(values.into())).unwrap()
    }

    /// A picture `w` × `h` brighter towards its right and top.
    fn gradient(w: u32, h: u32) -> Picture {
        let mut values = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let v = 0.1 + f32::from(u16::try_from(x + (h - y) * 2).unwrap()) * 0.05;
                values.extend([v, v, v, 1.0].map(half::f16::from_f32));
            }
        }
        Picture::new(w, h, Texels::Half(values.into())).unwrap()
    }

    fn picture_light(picture: Picture, rotation: f32) -> EnvironmentLight {
        let id = grid4d_scene::Textures::default().insert(grid4d_scene::Texture::linked("sky.exr".into()));
        let mut pictures = Pictures::new();
        pictures.insert(id, Arc::new(picture));
        let surround = PictureSurround::new(
            &Environment {
                texture: Some(id),
                rotation,
                ..Environment::default()
            },
            Mat3::IDENTITY,
        );
        EnvironmentLight::new(&environment(Surround::Picture(surround)), Some(&pictures))
    }

    /// Estimates ∫ radiance · weight over the sphere by sampling `light`.
    fn integral(light: &EnvironmentLight, weight: impl Fn(Vec3) -> f32, count: u32) -> Vec3 {
        let mut rng = Rng::new(5, 6, 7, 8);
        let mut sum = Vec3::ZERO;
        for _ in 0..count {
            if let Some((d, l, pdf)) = light.sample(rng.next_f32(), rng.next_f32(), rng.next_f32()) {
                sum += l * weight(d) / pdf;
            }
        }
        sum / f32::from(u16::try_from(count / 100).unwrap()) / 100.0
    }

    #[test]
    fn densities_match_where_samples_go_and_add_up_to_one() {
        let sky = Sky {
            placement: SunPlacement::Angles,
            elevation: 0.5,
            ..Sky::default()
        };
        let lights = [
            EnvironmentLight::new(&environment(Surround::Sky(SkyModel::new(&sky, Vec3::Z))), None),
            picture_light(gradient(64, 32), 0.7),
        ];
        for light in &lights {
            // ∫ density = 1: the mean of 1 / density over sampled directions
            // is the sphere's 4π only if densities are exact.
            let mut rng = Rng::new(1, 2, 3, 4);
            let n = 200_000;
            let (mut area, mut found) = (0.0f64, 0u32);
            for _ in 0..n {
                // None only at the poles, where the density is 0.
                if let Some((_, _, pdf)) = light.sample(rng.next_f32(), rng.next_f32(), rng.next_f32()) {
                    area += f64::from(1.0 / pdf);
                    found += 1;
                }
            }
            assert!(found > n - 10);
            let area = area / f64::from(found);
            assert!((area - 4.0 * std::f64::consts::PI).abs() < 0.2, "{area}");
        }
    }

    #[test]
    fn a_uniform_picture_gives_its_value_from_everywhere() {
        let mut values = vec![half::f16::from_f32(0.6); 32 * 16 * 4];
        for i in 0..32 * 16 {
            values[i * 4 + 3] = half::f16::ONE;
        }
        let light = picture_light(Picture::new(32, 16, Texels::Half(values.into())).unwrap(), 0.0);
        // ∫ light × cos over the sky above a surface facing up: π × 0.6, so
        // a white surface shows 0.6 (the furnace).
        let up = integral(&light, |d| d.y.max(0.0), 100_000);
        assert!((up.x - PI * 0.6).abs() < PI * 0.6 * 0.02, "{up}");
        // Without a picture: the uniform grey of half the intensity.
        let plain = EnvironmentLight::new(
            &environment(Surround::Picture(PictureSurround::new(
                &Environment::default(),
                Mat3::IDENTITY,
            ))),
            None,
        );
        assert_eq!(plain.radiance(Vec3::X), Vec3::splat(0.5));
    }

    #[test]
    fn one_bright_texel_is_found_by_importance() {
        let light = picture_light(one_bright(64, 32, 16, 8, 1000.0), 0.0);
        // Most samples go towards the bright texel.
        let towards = sky::equirect_direction(centre(16, 8, 64, 32));
        let mut rng = Rng::new(9, 9, 9, 9);
        let near = (0..1000)
            .filter_map(|_| light.sample(rng.next_f32(), rng.next_f32(), rng.next_f32()))
            .filter(|(d, ..)| d.dot(towards) > 0.99)
            .count();
        assert!(near > 800, "{near}");
        // The light it gives, the same as the texel's solid angle says.
        let v = (8.0 + 0.5) / 32.0;
        let solid_angle = (TAU / 64.0) * (PI / 32.0) * (v * PI).sin();
        let along = |d: Vec3| d.dot(towards).max(0.0);
        let estimate = integral(&light, along, 20_000).x;
        let expected = 1000.0 * solid_angle;
        assert!((estimate - expected).abs() < expected * 0.1, "{estimate} vs {expected}");
    }

    #[test]
    fn the_sun_is_seen_inside_its_disc_and_sampled() {
        let sky = Sky {
            placement: SunPlacement::Angles,
            elevation: 0.8,
            ..Sky::default()
        };
        let model = SkyModel::new(&sky, Vec3::Z);
        let light = EnvironmentLight::new(&environment(Surround::Sky(model)), None);
        let sun = model.sun().unwrap();
        assert!(light.radiance(sun.direction).y > 1000.0);
        assert!(light.radiance(Vec3::Y).y < 10.0);
        // A white surface facing the sun: the sun's light, plus the sky's (a
        // fifth or so on a clear day).
        let facing = integral(&light, |d| d.dot(sun.direction).max(0.0), 40_000) / PI;
        assert!(
            facing.y > sun.irradiance.y * 1.05 && facing.y < sun.irradiance.y * 1.5,
            "{facing} vs {}",
            sun.irradiance
        );
    }

    #[test]
    fn importance_sampling_is_far_less_noisy_than_even_sampling() {
        let mut light = picture_light(one_bright(64, 32, 16, 8, 1000.0), 0.0);
        let towards = sky::equirect_direction(centre(16, 8, 64, 32));
        // The spread of 16-sample estimates of the light on a surface
        // facing the bright texel.
        let spread = |light: &EnvironmentLight| {
            let mut rng = Rng::new(3, 1, 4, 1);
            let estimates: Vec<f32> = (0..300)
                .map(|_| {
                    (0..16)
                        .filter_map(|_| light.sample(rng.next_f32(), rng.next_f32(), rng.next_f32()))
                        .map(|(d, l, pdf)| l.x * d.dot(towards).max(0.0) / pdf)
                        .sum::<f32>()
                        / 16.0
                })
                .collect();
            let mean = estimates.iter().sum::<f32>() / 300.0;
            (estimates.iter().map(|e| (e - mean).powi(2)).sum::<f32>() / 300.0).sqrt()
        };
        let importance = spread(&light);
        light.table = None;
        let even = spread(&light);
        assert!(even > importance * 10.0, "{even} vs {importance}");
    }

    #[test]
    fn rotation_turns_the_picture() {
        let still = picture_light(one_bright(64, 32, 16, 8, 1000.0), 0.0);
        let turned = picture_light(one_bright(64, 32, 16, 8, 1000.0), PI / 2.0);
        let d = sky::equirect_direction(centre(16, 8, 64, 32));
        let quarter = Mat3::from_rotation_y(PI / 2.0) * d;
        assert!(still.radiance(d).x > 100.0 && still.radiance(quarter).x < 1.0);
        assert!((turned.radiance(quarter).x - still.radiance(d).x).abs() < 1e-3);
    }
}
