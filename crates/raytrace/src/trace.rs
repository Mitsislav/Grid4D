//! The path tracer: per pixel, samples of paths from the camera that pick up
//! the lights' light at every surface they meet.

use std::{
    f32::consts::PI,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use glam::Vec3;
use grid4d_scene::{Falloff, LightShadow, LightType, RenderSettings};
use rayon::prelude::*;

use crate::{
    Image, RenderCamera, RenderScene,
    environment::EnvironmentLight,
    lights::{Entry, LightTable},
    passes::{First, Layout, Part, Split},
    sample::{Rng, cosine_hemisphere},
    scene::{Hit, Light},
    shade::{Lobe, Material},
    subsurface,
};

/// The largest picture side, in pixels.
const MAX_SIZE: u32 = 16_384;
/// The most samples per pixel.
const MAX_SAMPLES: u32 = 4096;
/// Light picked up after a bounce is capped at this (per colour channel and
/// light) while [`RenderOptions::clamp_indirect`] is on: a bounce that lands
/// on a sharp highlight would otherwise leave bright specks ("fireflies")
/// that take thousands of samples to average out.
const BOUNCE_CAP: f32 = 2.0;

/// Adaptive sampling: where the picture is clean, pixels stop early.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Adaptive {
    /// Samples every pixel gets before any stops (at most
    /// [`RenderOptions::samples`]).
    pub min_samples: u32,
    /// A pixel stops once its estimated noise — the difference between the
    /// average of all its samples and of every other one, relative to the
    /// square root of its brightness — is below this, around it too.
    pub threshold: f32,
}

/// How a picture is rendered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderOptions {
    /// Picture size in pixels (1…16384 each).
    pub width: u32,
    pub height: u32,
    /// Samples per pixel (1…4096): more is smoother and slower.
    pub samples: u32,
    /// How many times light bounces between surfaces (0: direct light only).
    pub bounces: u32,
    /// Changes the noise pattern, nothing else.
    pub seed: u32,
    /// A scene without lights is lit by the automatic light (over the camera's
    /// shoulder, no shadows); otherwise it stays dark but for luminance.
    pub auto_light: bool,
    /// Light found after a bounce is capped (fewer specks, darker caustics).
    pub clamp_indirect: bool,
    /// Surfaces that give off light are sampled as lights (with multiple
    /// importance sampling); off: they light others only when a bounce
    /// happens to meet them, as in Grid4D 1.1.
    pub emissive_lights: bool,
    /// Samples where the picture is still noisy only, up to `samples`;
    /// `None`: every pixel gets `samples`.
    pub adaptive: Option<Adaptive>,
    /// Only the pixels from (`x0`, `y0`) up to, not including, (`x1`, `y1`)
    /// are rendered; the rest stay transparent. `None`: all.
    pub region: Option<[u32; 4]>,
    /// The most triangles displacement may make (for [`Texturing`]
    /// (crate::Texturing) when the scene is built).
    pub displacement_budget: usize,
}

impl RenderOptions {
    /// The picture's size, as rendered (kept in range).
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        (self.width.clamp(1, MAX_SIZE), self.height.clamp(1, MAX_SIZE))
    }

    /// The pixels rendered: the region inside the picture, else all of
    /// them (first column and row, one past the last).
    #[must_use]
    pub fn rendered(&self) -> [u32; 4] {
        let (w, h) = self.size();
        match self.region {
            Some([x0, y0, x1, y1]) => {
                let (x0, y0) = (x0.min(w - 1), y0.min(h - 1));
                [x0, y0, x1.clamp(x0 + 1, w), y1.clamp(y0 + 1, h)]
            }
            None => [0, 0, w, h],
        }
    }

    /// How many pixels are rendered (a render's work is this × samples).
    #[must_use]
    pub fn pixel_count(&self) -> u64 {
        let [x0, y0, x1, y1] = self.rendered();
        u64::from(x1 - x0) * u64::from(y1 - y0)
    }
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            width: 640,
            height: 360,
            samples: 16,
            bounces: 2,
            seed: 0,
            auto_light: true,
            clamp_indirect: true,
            emissive_lights: true,
            adaptive: None,
            region: None,
            displacement_budget: 20_000_000,
        }
    }
}

impl From<&RenderSettings> for RenderOptions {
    /// The document's render settings (seed 0).
    fn from(settings: &RenderSettings) -> Self {
        Self {
            width: settings.width,
            height: settings.height,
            samples: settings.samples,
            bounces: settings.bounces,
            seed: 0,
            auto_light: settings.auto_light,
            clamp_indirect: settings.clamp_indirect,
            emissive_lights: settings.emissive_lights,
            adaptive: settings.adaptive.then(|| Adaptive {
                min_samples: settings.min_samples,
                threshold: settings.noise_threshold.get(),
            }),
            // Final renders: only with Use for Final Render.
            region: settings
                .region
                .filter(|_| settings.region_final)
                .map(|region| region.pixels(settings.width, settings.height)),
            displacement_budget: usize::try_from(settings.displacement_budget)
                .unwrap_or(1)
                .saturating_mul(1_000_000),
        }
    }
}

/// Renders `scene` through `camera`. Rows are rendered in parallel; the result
/// is the same however the work is shared.
#[must_use]
pub fn render(scene: &RenderScene, camera: &RenderCamera, options: &RenderOptions) -> Image {
    let mut progressive = Progressive::new(options);
    progressive.add_samples(scene, camera, options, u32::MAX, &RenderControl::default());
    progressive.image()
}

/// Lets another thread follow a render and stop it.
#[derive(Debug, Default)]
pub struct RenderControl {
    cancelled: AtomicBool,
    progress: AtomicU64,
}

impl RenderControl {
    /// Asks the render to stop: rows not started yet are skipped.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// Work done so far, in pixels × samples (a whole render: width ×
    /// height × samples; with adaptive sampling a pixel that stops counts
    /// its remaining samples as done).
    #[must_use]
    pub fn progress(&self) -> u64 {
        self.progress.load(Ordering::Relaxed)
    }
}

/// A render built up in passes, for showing it while it sharpens: each pass
/// adds samples to every pixel (with adaptive sampling, to every pixel still
/// noisy). Samples are added in the same order as one [`render`] adds them,
/// and adaptive decisions are taken at fixed sample counts, so the finished
/// picture is exactly the same however the passes are cut.
#[derive(Clone, Debug)]
pub struct Progressive {
    width: u32,
    height: u32,
    /// Per pixel: the sum of its samples' colours, and how many met a surface.
    sums: Vec<[f32; 4]>,
    /// Samples given to every pixel still sampled.
    samples: u32,
    /// The render passes asked for: where their sums sit in `extra`, a
    /// pixel's `layout.stride` floats after another (one float per row
    /// without passes); and the light groups' names.
    layout: Option<Layout>,
    extra: Vec<f32>,
    groups: Vec<String>,
    /// Adaptive sampling's state per pixel (one per row without it or the
    /// denoiser's guides).
    adaptive: Option<Adaptive>,
    states: Vec<PixelState>,
    /// Pixels still sampled.
    active: usize,
    /// The passes the caller asked for (the layout may hold the
    /// denoiser's guides besides).
    requested: grid4d_scene::Passes,
    /// The denoiser's guides are gathered.
    guided: bool,
    /// The pixels rendered (see [`RenderOptions::rendered`]).
    region: [u32; 4],
}

/// A pixel's adaptive sampling.
#[derive(Clone, Copy, Debug, Default)]
struct PixelState {
    /// The sum of its odd-numbered samples (colour and coverage): half the
    /// samples, whose average is compared with the whole's.
    half: [f32; 4],
    /// Samples it got.
    count: u32,
    /// Still sampled.
    active: bool,
    /// Its noise when last measured.
    error: f32,
}

/// How a pixel stopped: its samples so far, the colour and coverage
/// averaged over all of them and over the odd-numbered half.
fn noise(sum: &[f32; 4], state: &PixelState) -> f32 {
    if state.count < 2 {
        return f32::INFINITY;
    }
    let n = to_f32(state.count);
    let odd = to_f32(state.count / 2);
    let all = Vec3::new(sum[0], sum[1], sum[2]) / n;
    let half = Vec3::new(state.half[0], state.half[1], state.half[2]) / odd;
    let colour = (all - half).abs().element_sum() / (1e-4 + all.element_sum().max(0.0).sqrt());
    let coverage = (sum[3] / n - state.half[3] / odd).abs();
    colour.max(coverage)
}

impl Progressive {
    /// No samples yet, at the size of `options`.
    #[must_use]
    pub fn new(options: &RenderOptions) -> Self {
        let width = options.width.clamp(1, MAX_SIZE);
        let height = options.height.clamp(1, MAX_SIZE);
        let count = usize::try_from(u64::from(width) * u64::from(height)).unwrap_or(0);
        let rows = usize::try_from(height).unwrap_or(0);
        let adaptive = options.adaptive;
        let region = options.rendered();
        let [x0, y0, x1, y1] = region;
        let inside = |i: usize| {
            let (x, y) = (
                u32::try_from(i % usize::try_from(width).unwrap_or(1)).unwrap_or(0),
                u32::try_from(i / usize::try_from(width).unwrap_or(1)).unwrap_or(0),
            );
            (x0..x1).contains(&x) && (y0..y1).contains(&y)
        };
        Self {
            width,
            height,
            sums: vec![[0.0; 4]; count],
            samples: 0,
            layout: None,
            extra: vec![0.0; rows],
            groups: Vec::new(),
            adaptive,
            states: if adaptive.is_some() {
                (0..count)
                    .map(|i| PixelState {
                        active: inside(i),
                        ..PixelState::default()
                    })
                    .collect()
            } else {
                vec![PixelState::default(); rows]
            },
            active: usize::try_from(options.pixel_count()).unwrap_or(count),
            requested: grid4d_scene::Passes::NONE,
            guided: false,
            region,
        }
    }

    /// Whether each pixel keeps its own state (adaptive sampling, or the
    /// denoiser's estimate of its noise).
    fn per_pixel(&self) -> bool {
        self.adaptive.is_some() || self.guided
    }

    /// Gathers, beside the picture, what [`denoise`](crate::denoise) needs:
    /// the surface colour, normal and depth (not returned as passes unless
    /// asked for) and each pixel's noise.
    #[must_use]
    pub fn with_denoising(mut self) -> Self {
        use grid4d_scene::Pass;
        let mut wanted = self.requested;
        for pass in [Pass::Albedo, Pass::Normal, Pass::Depth] {
            wanted.set(pass, true);
        }
        self.layout = Layout::new(wanted, self.groups.len());
        if let Some(layout) = &self.layout {
            let row = (usize::try_from(self.width).unwrap_or(1) * layout.stride).max(1);
            self.extra = vec![0.0; row * usize::try_from(self.height).unwrap_or(0)];
        }
        if self.adaptive.is_none() {
            let width = usize::try_from(self.width).unwrap_or(1);
            let [x0, y0, x1, y1] = self.region;
            self.states = (0..self.sums.len())
                .map(|i| PixelState {
                    active: u32::try_from(i % width).is_ok_and(|x| (x0..x1).contains(&x))
                        && u32::try_from(i / width).is_ok_and(|y| (y0..y1).contains(&y)),
                    ..PixelState::default()
                })
                .collect();
        }
        self.guided = true;
        self
    }

    /// No samples yet, at the size of `options`, gathering `passes` of
    /// `scene` beside the picture (see `docs/reference/render-passes.md`).
    #[must_use]
    pub fn with_passes(options: &RenderOptions, passes: grid4d_scene::Passes, scene: &RenderScene) -> Self {
        let mut progressive = Self::new(options);
        progressive.layout = Layout::new(passes, scene.light_groups().len());
        if let Some(layout) = &progressive.layout {
            let row = (usize::try_from(progressive.width).unwrap_or(1) * layout.stride).max(1);
            progressive.extra = vec![0.0; row * usize::try_from(progressive.height).unwrap_or(0)];
        }
        progressive.groups = scene.light_groups().to_vec();
        progressive.requested = passes;
        progressive
    }

    /// Samples per pixel so far (with adaptive sampling: of the pixels
    /// still sampled).
    #[must_use]
    pub fn samples(&self) -> u32 {
        self.samples
    }

    /// Whether every sample of `options` is in (with adaptive sampling: or
    /// every pixel stopped).
    #[must_use]
    pub fn is_complete(&self, options: &RenderOptions) -> bool {
        self.samples >= options.samples.clamp(1, MAX_SAMPLES) || self.active == 0
    }

    /// The next sample count at which adaptive sampling decides which
    /// pixels go on: the minimum, then every half as many again (at least 4
    /// more); always even, so the two halves compared are equal. Without
    /// adaptive sampling: `total`.
    fn next_decision(&self, total: u32) -> u32 {
        let Some(adaptive) = self.adaptive else {
            return total;
        };
        let even = |n: u32| n.saturating_add(n % 2);
        let min = even(adaptive.min_samples.clamp(2, MAX_SAMPLES));
        let next = if self.samples < min {
            min
        } else {
            even(self.samples + (self.samples / 2).max(4))
        };
        next.min(total)
    }

    /// Adds up to `count` samples per pixel (never more than `options` asks
    /// for in all). Returns `false` if `control` was cancelled before the pass
    /// was done: some rows then lack the pass, so the render must be dropped.
    pub fn add_samples(
        &mut self,
        scene: &RenderScene,
        camera: &RenderCamera,
        options: &RenderOptions,
        count: u32,
        control: &RenderControl,
    ) -> bool {
        let total = options.samples.clamp(1, MAX_SAMPLES);
        let goal = self.samples.saturating_add(count).min(total);
        let automatic = [automatic_light(camera)];
        // Auto Light only for a scene with no light at all: a Sky or an
        // Environment lights it too.
        let lights = if scene.lights().is_empty() && options.auto_light && scene.environment().is_none() {
            &automatic[..]
        } else {
            scene.lights()
        };
        // The job reads the layout while the sums are written (a few
        // numbers: cloned once per call).
        let layout = self.layout.clone();
        let job = Job {
            scene,
            lights,
            camera,
            width: self.width,
            height: self.height,
            bounces: options.bounces,
            seed: options.seed,
            clamp: options.clamp_indirect,
            table: scene.light_table(options.emissive_lights),
            moving: scene.is_moving() || camera.end.is_some(),
            layout: layout.as_ref(),
        };
        while self.samples < goal && self.active > 0 {
            let decision = self.next_decision(total);
            let stop = decision.min(goal);
            if !self.add_range(&job, self.samples..stop, control) {
                return false;
            }
            self.samples = stop;
            if self.adaptive.is_some() && stop == decision {
                self.decide(total, control);
            }
        }
        !control.is_cancelled()
    }

    /// Adds samples `range` to every pixel still sampled.
    fn add_range(&mut self, job: &Job<'_>, range: std::ops::Range<u32>, control: &RenderControl) -> bool {
        let row_length = usize::try_from(self.width).unwrap_or(1);
        let stride = self.layout.as_ref().map_or(0, |layout| layout.stride);
        let extra_row = (row_length * stride).max(1);
        let adaptive = self.per_pixel();
        let state_row = if adaptive { row_length } else { 1 };
        let pass = u64::from(range.end - range.start);
        let [x0, y0, x1, y1] = self.region;
        self.sums
            .par_chunks_mut(row_length)
            .zip(self.extra.par_chunks_mut(extra_row))
            .zip(self.states.par_chunks_mut(state_row))
            .zip(0..self.height)
            .for_each(|(((row, extra), states), y)| {
                if control.is_cancelled() || !(y0..y1).contains(&y) {
                    return;
                }
                let mut sampled = 0u64;
                for (i, (sum, x)) in row.iter_mut().zip(0..self.width).enumerate() {
                    if !(x0..x1).contains(&x) {
                        continue;
                    }
                    let state = if adaptive { states.get_mut(i) } else { None };
                    if state.as_ref().is_some_and(|s| !s.active) {
                        continue;
                    }
                    let slots = extra.get_mut(i * stride..(i + 1) * stride).unwrap_or(&mut []);
                    match state {
                        Some(state) => {
                            job.add_samples(x, y, range.clone(), sum, slots, Some(&mut state.half));
                            state.count = range.end;
                        }
                        None => job.add_samples(x, y, range.clone(), sum, slots, None),
                    }
                    sampled += 1;
                }
                control.progress.fetch_add(sampled * pass, Ordering::Relaxed);
            });
        !control.is_cancelled()
    }

    /// Stops the pixels whose noise, and their neighbours' (3 × 3), is
    /// below the threshold; counts their remaining samples as done.
    fn decide(&mut self, total: u32, control: &RenderControl) {
        let Some(adaptive) = self.adaptive else {
            return;
        };
        let (w, h) = (
            usize::try_from(self.width).unwrap_or(1),
            usize::try_from(self.height).unwrap_or(1),
        );
        // Each pixel's own estimate is noisy itself: it is averaged over
        // its 3 × 3 neighbourhood (a stopped pixel keeps its last one), and
        // a pixel stops only when that average is below the threshold all
        // around it, so pixels are not stopped by luck.
        self.states
            .par_iter_mut()
            .zip(&self.sums)
            .filter(|(state, _)| state.active)
            .for_each(|(state, sum)| state.error = noise(sum, state));
        let around = |values: &dyn Fn(usize) -> f32, x: usize, y: usize, reduce: fn(f32, f32) -> f32| {
            let mut out: Option<f32> = None;
            let mut count = 0.0f32;
            for ny in y.saturating_sub(1)..(y + 2).min(h) {
                for nx in x.saturating_sub(1)..(x + 2).min(w) {
                    let v = values(ny * w + nx);
                    out = Some(out.map_or(v, |o| reduce(o, v)));
                    count += 1.0;
                }
            }
            (out.unwrap_or(0.0), count)
        };
        let states = &self.states;
        let error = |i: usize| states.get(i).map_or(0.0, |s| s.error);
        let smoothed: Vec<f32> = (0..w * h)
            .into_par_iter()
            .map(|i| {
                let (sum, count) = around(&error, i % w, i / w, |a, b| a + b);
                sum / count
            })
            .collect();
        let smooth = |i: usize| smoothed.get(i).copied().unwrap_or(0.0);
        let stopped = AtomicU64::new(0);
        let remaining = u64::from(total.saturating_sub(self.samples));
        self.states.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            for (x, state) in row.iter_mut().enumerate() {
                if !state.active {
                    continue;
                }
                let (worst, _) = around(&smooth, x, y, f32::max);
                if worst < adaptive.threshold {
                    state.active = false;
                    stopped.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
        let stopped = stopped.into_inner();
        control.progress.fetch_add(stopped * remaining, Ordering::Relaxed);
        self.active -= usize::try_from(stopped).unwrap_or(self.active).min(self.active);
    }

    /// The picture so far: each pixel the average of its samples, colour
    /// premultiplied by coverage. Transparent before the first pass.
    #[must_use]
    pub fn image(&self) -> Image {
        let mut image = Image::new(self.width, self.height);
        if self.samples == 0 {
            return image;
        }
        let adaptive = self.per_pixel();
        let count = |i: usize| -> u32 {
            if adaptive {
                self.states.get(i).map_or(self.samples, |s| s.count)
            } else {
                self.samples
            }
        };
        for (i, (pixel, &[r, g, b, covered])) in image.pixels.iter_mut().zip(&self.sums).enumerate() {
            let n = to_f32(count(i).max(1));
            let colour = Vec3::new(r, g, b) / n;
            *pixel = [colour.x, colour.y, colour.z, covered / n];
        }
        if self.adaptive.is_some() {
            image.sample_counts = self.states.iter().map(|s| s.count).collect();
        }
        if let Some(layout) = &self.layout {
            let extra: &[f32] = if layout.stride == 0 { &[] } else { &self.extra };
            image.passes = layout.layers(&self.sums, extra, &count, &self.groups);
        }
        if self.guided {
            image.guides = Some(self.guides(&mut image.passes));
        }
        image
    }

    /// The denoiser's guides, taken out of `passes` (or copied, where the
    /// caller asked for them too), and each pixel's noise.
    fn guides(&self, passes: &mut Vec<crate::PassLayer>) -> crate::Guides {
        use grid4d_scene::Pass;
        let mut take = |pass: Pass| -> Vec<f32> {
            let Some(index) = passes.iter().position(|l| l.name == pass.name()) else {
                return Vec::new();
            };
            if self.requested.contains(pass) {
                passes[index].values.clone()
            } else {
                passes.remove(index).values
            }
        };
        let (albedo, normal, depth) = (take(Pass::Albedo), take(Pass::Normal), take(Pass::Depth));
        // The variance of the average's brightness: the squared difference
        // between the average of all samples and of the odd ones.
        let weights = Vec3::new(0.2126, 0.7152, 0.0722);
        let variance = self
            .sums
            .iter()
            .zip(&self.states)
            .map(|(sum, state)| {
                if state.count < 2 {
                    return 0.0;
                }
                let all = Vec3::new(sum[0], sum[1], sum[2]) / to_f32(state.count);
                let odd = Vec3::new(state.half[0], state.half[1], state.half[2]) / to_f32(state.count / 2);
                let d = (all - odd).dot(weights);
                d * d
            })
            .collect();
        crate::Guides {
            albedo,
            normal,
            depth,
            variance,
        }
    }
}

/// What every pixel of one render shares.
struct Job<'a> {
    scene: &'a RenderScene,
    lights: &'a [Light],
    camera: &'a RenderCamera,
    width: u32,
    height: u32,
    bounces: u32,
    seed: u32,
    /// Light after a bounce is capped.
    clamp: bool,
    /// Lights with a surface to sample: area lights, and emitting triangles
    /// when they are sampled as lights.
    table: &'a LightTable,
    /// The scene or the camera moves over the shutter interval: each sample
    /// gets a moment of it.
    moving: bool,
    /// The render passes asked for.
    layout: Option<&'a Layout>,
}

/// A surface a path meets, as lighting it needs it: the hit, its material,
/// the shading and geometric normals turned towards the path, the direction
/// back along it, the point lifted off the surface (and by how much), and
/// whether the path met its front.
struct At<'a> {
    hit: &'a Hit,
    material: &'a Material,
    n: Vec3,
    v: Vec3,
    geometric: Vec3,
    lifted: Vec3,
    lift: f32,
    entering: bool,
}

/// One sample's light as it is gathered: the total, and the passes' split
/// when they are asked for.
struct Gather<'a> {
    radiance: Vec3,
    split: Option<&'a mut Split>,
}

impl Gather<'_> {
    fn add(&mut self, part: Part, group: u16, light: Vec3) {
        self.radiance += light;
        if let Some(split) = self.split.as_deref_mut() {
            split.add(part, group, light);
        }
    }

    /// Light a surface reflects straight from a light: `diffuse` of it the
    /// matte layer's (Diffuse Direct), the rest glossy (Specular Direct).
    fn add_direct(&mut self, group: u16, light: Vec3, diffuse: Vec3) {
        self.radiance += light;
        if let Some(split) = self.split.as_deref_mut() {
            split.add(Part::DiffuseDirect, group, diffuse);
            split.add(Part::SpecularDirect, group, light - diffuse);
        }
    }
}

/// Where light found by a ray after the first bounce goes: straight from a
/// light when it is that bounce's own ray (`depth` 1), else after bounces;
/// diffuse, glossy or through glass by the first bounce's `lobe`.
const fn found_part(depth: u32, lobe: Option<Lobe>) -> Part {
    match (lobe, depth <= 1) {
        (Some(Lobe::Transmission), _) => Part::Transmission,
        (Some(Lobe::Specular), true) => Part::SpecularDirect,
        (Some(Lobe::Specular), false) => Part::SpecularIndirect,
        (Some(Lobe::Diffuse | Lobe::Subsurface) | None, true) => Part::DiffuseDirect,
        (Some(Lobe::Diffuse | Lobe::Subsurface) | None, false) => Part::DiffuseIndirect,
    }
}

/// Where light sampled from a light at a surface after the first bounce
/// goes, by the first bounce's `lobe`.
const fn sampled_part(lobe: Option<Lobe>) -> Part {
    match lobe {
        Some(Lobe::Transmission) => Part::Transmission,
        Some(Lobe::Specular) => Part::SpecularIndirect,
        Some(Lobe::Diffuse | Lobe::Subsurface) | None => Part::DiffuseIndirect,
    }
}

impl Job<'_> {
    /// Adds samples `samples` of pixel (`x`, `y`) to `sum`: their colours, and
    /// one per sample that met a surface.
    /// With `half`, the odd-numbered samples are added to it too (adaptive
    /// sampling's estimate of the noise).
    fn add_samples(
        &self,
        x: u32,
        y: u32,
        samples: std::ops::Range<u32>,
        sum: &mut [f32; 4],
        slots: &mut [f32],
        mut half: Option<&mut [f32; 4]>,
    ) {
        let (w, h) = (to_f32(self.width), to_f32(self.height));
        let aspect = w / h;
        for sample in samples {
            let mut rng = Rng::new(x, y, sample, self.seed);
            // A point spread over the pixel: smooth edges.
            let px = (to_f32(x) + rng.next_f32()) / w * 2.0 - 1.0;
            let py = 1.0 - (to_f32(y) + rng.next_f32()) / h * 2.0;
            // A moment of the shutter interval and a point on the aperture,
            // drawn only when they matter (so pictures without motion blur
            // or depth of field come out as they always have).
            let time = if self.moving { rng.next_f32() } else { 0.0 };
            let aperture = self.camera.lens.map(|_| (rng.next_f32(), rng.next_f32()));
            let (origin, dir) = self.camera.sample_ray(px, py, aspect, time, aperture);
            let mut split = self.layout.map(|_| Split::default());
            if let Some(radiance) = self.trace(origin, dir, time, &mut rng, split.as_mut()) {
                sum[0] += radiance.x;
                sum[1] += radiance.y;
                sum[2] += radiance.z;
                sum[3] += 1.0;
                if sample % 2 == 1
                    && let Some(half) = half.as_deref_mut()
                {
                    half[0] += radiance.x;
                    half[1] += radiance.y;
                    half[2] += radiance.z;
                    half[3] += 1.0;
                }
            }
            if let (Some(layout), Some(split)) = (self.layout, split) {
                layout.add(slots, &split);
            }
        }
    }

    /// The light coming back along one path; `None` if the first ray meets no
    /// surface (background).
    fn trace(
        &self,
        mut origin: Vec3,
        mut dir: Vec3,
        time: f32,
        rng: &mut Rng,
        split: Option<&mut Split>,
    ) -> Option<Vec3> {
        let wants_split = split.is_some();
        let mut gather = Gather {
            radiance: Vec3::ZERO,
            split,
        };
        // The part of the first surface the path went on through (passes).
        let mut first_lobe: Option<Lobe> = None;
        let mut throughput = Vec3::ONE;
        // The density with which the last bounce chose `dir` (reflected
        // paths only): a light with a surface it meets is weighed against
        // having been sampled directly from there. `None` for the camera ray
        // and for refracted paths, which direct sampling cannot reproduce.
        let mut last_pdf: Option<f32> = None;
        let sampling = !self.table.is_empty() || self.scene.environment().is_some();
        // With lights that are also sampled directly, the last surface's
        // bounce is followed one ray further for the light it meets alone:
        // direct sampling there is weighed against it, so without it light
        // would be missing.
        let last = self.bounces + u32::from(sampling);
        // The triangle the path last left (linked lights do not light every
        // object).
        let mut from: Option<usize> = None;
        for depth in 0..=last {
            let completing = depth > self.bounces;
            if completing && last_pdf.is_none() {
                break; // refracted: no light was weighed against this ray
            }
            let hit = self.scene.intersect(origin, dir, time);
            let reach = hit
                .as_ref()
                .map_or(f32::INFINITY, |hit| (hit.position - origin).length());
            // A visible area light in front of the surface: its light, and
            // the path ends there (its shape is opaque).
            if let Some((seen, group)) = self.area_seen(origin, dir, reach, depth, last_pdf, from) {
                let part = if depth == 0 {
                    Part::Emission
                } else {
                    found_part(depth, first_lobe)
                };
                gather.add(part, group, self.capped(throughput * seen, depth));
                break;
            }
            let Some(hit) = hit else {
                let Some(environment) = self.scene.environment() else {
                    if depth == 0 {
                        return None;
                    }
                    break; // nothing around the scene: black
                };
                if depth == 0 {
                    // The background: the surround, or transparent.
                    if !environment.visible {
                        return None;
                    }
                    gather.add(Part::Background, environment.group, environment.radiance(dir));
                    return Some(gather.radiance);
                }
                // Found by a bounce: weighed against having been sampled
                // directly (refracted paths cannot be).
                let weight = last_pdf.map_or(1.0, |bsdf| power_heuristic(bsdf, environment.density(dir)));
                gather.add(
                    found_part(depth, first_lobe),
                    environment.group,
                    self.capped(throughput * environment.radiance(dir) * weight, depth),
                );
                break;
            };
            let v = -dir;
            // The ray met the front of the surface (into a closed object).
            let entering = hit.geometric.dot(v) > 0.0;
            // Both normals turned towards where the ray came from.
            let geometric = if hit.geometric.dot(v) < 0.0 {
                -hit.geometric
            } else {
                hit.geometric
            };
            let mut n = if hit.shading.dot(geometric) < 0.0 {
                -hit.shading
            } else {
                hit.shading
            };
            if n.dot(v) <= 0.0 {
                n = geometric;
            }
            // Normal and Bump pictures.
            let bent = hit.bend.apply(n);
            if bent.dot(v) > 0.0 {
                n = bent;
            }
            let material = Material::new(&hit.surface);
            if depth == 0
                && let Some(split) = gather.split.as_deref_mut()
            {
                let eye = self.camera.world_at(time);
                let forward = -eye.z_axis.truncate().normalize_or(Vec3::Z);
                let (object, material_id) = self.scene.ids_of(hit.triangle);
                split.first = Some(First {
                    depth: (hit.position - eye.w_axis.truncate()).dot(forward),
                    normal: n,
                    position: hit.position,
                    albedo: hit.surface.color,
                    object,
                    material: material_id,
                });
            }
            if hit.surface.emission.max_element() > 0.0 {
                let weight = match last_pdf {
                    Some(bsdf) => {
                        let distance2 = reach * reach;
                        let light = self.triangle_density(hit.triangle, geometric.dot(v), distance2);
                        // The ray past the last bounce brings only light
                        // that was also sampled directly.
                        if completing && light <= 0.0 {
                            0.0
                        } else {
                            power_heuristic(bsdf, light)
                        }
                    }
                    None => 1.0,
                };
                let part = if depth == 0 {
                    Part::Emission
                } else {
                    found_part(depth, first_lobe)
                };
                gather.add(part, 0, self.capped(throughput * hit.surface.emission * weight, depth));
            }
            if completing {
                break;
            }
            // Light sampled at this surface: straight from the lights at the
            // first one (split diffuse and glossy), after bounces beyond.
            let direct = depth == 0 && wants_split;

            let lift = offset(hit.position);
            let lifted = hit.position + geometric * lift;
            let at = At {
                hit: &hit,
                material: &material,
                n,
                v,
                geometric,
                lifted,
                lift,
                entering,
            };
            self.light_at(&at, (throughput, depth, first_lobe), direct, time, rng, &mut gather);

            if depth == self.bounces && !sampling {
                break;
            }
            let Some(bounce) = material.bounce(n, v, entering, rng) else {
                break;
            };
            if bounce.lobe == Lobe::Subsurface {
                // Into the surface: a walk inside, then out wherever it
                // leaves, lit there as a white matte surface (the walk gave
                // the light its colour), and on from there.
                throughput *= bounce.weight;
                if depth == 0 {
                    first_lobe = Some(Lobe::Subsurface);
                }
                let Some(medium) = material.medium() else {
                    break;
                };
                let inside = hit.position - geometric * lift;
                let Some(exit) = subsurface::walk(self.scene, inside, bounce.direction, medium, time, rng) else {
                    break;
                };
                throughput *= exit.weight;
                if throughput.max_element() <= 1e-4 {
                    break;
                }
                let out = if exit.hit.geometric.dot(exit.direction) > 0.0 {
                    exit.hit.geometric
                } else {
                    -exit.hit.geometric
                };
                let white = Material::exit();
                let exit_lift = offset(exit.hit.position);
                let at = At {
                    hit: &exit.hit,
                    material: &white,
                    n: out,
                    v: out,
                    geometric: out,
                    lifted: exit.hit.position + out * exit_lift,
                    lift: exit_lift,
                    entering: false,
                };
                // The walk belongs to this surface: its light is not capped
                // as light after a bounce is.
                self.light_at(&at, (throughput, depth, first_lobe), false, time, rng, &mut gather);
                if depth >= self.bounces && !sampling {
                    break;
                }
                dir = cosine_hemisphere(out, rng.next_f32(), rng.next_f32());
                last_pdf = sampling.then(|| white.pdf(out, out, dir, false));
                origin = at.lifted;
                from = Some(exit.hit.triangle);
                continue;
            }
            // Reflected paths stay on the side they came from, refracted ones
            // cross the surface.
            let side = bounce.direction.dot(geometric);
            if (bounce.through && side >= 0.0) || (!bounce.through && side <= 0.0) {
                break;
            }
            throughput *= bounce.weight;
            if depth == 0 {
                first_lobe = Some(bounce.lobe);
            }
            if throughput.max_element() <= 1e-4 {
                break;
            }
            last_pdf = (sampling && !bounce.through).then(|| material.pdf(n, v, bounce.direction, entering));
            origin = if bounce.through {
                hit.position - geometric * lift
            } else {
                lifted
            };
            dir = bounce.direction;
            from = Some(hit.triangle);
        }
        Some(gather.radiance)
    }

    /// The light that reaches the surface `at` straight from the lights,
    /// glowing surfaces and the surround, reflected towards the path, added
    /// to `gather` (`throughput`: the path's weight so far; at `depth`; the
    /// first bounce's lobe for passes; `direct`: split into the matte and
    /// glossy shares).
    fn light_at(
        &self,
        at: &At<'_>,
        (throughput, depth, first_lobe): (Vec3, u32, Option<Lobe>),
        direct: bool,
        time: f32,
        rng: &mut Rng,
        gather: &mut Gather<'_>,
    ) {
        let &At {
            hit,
            material,
            n,
            v,
            geometric,
            lifted,
            lift,
            entering,
        } = at;
        for light in self.lights {
            if light.kind == LightType::Area || !self.scene.lit_by(hit.triangle, light.link) {
                continue; // sampled from the table, or not lighting this object
            }
            let Some(incoming) = light.arriving(hit.position, rng) else {
                continue;
            };
            let reflected = material.reflect(n, v, incoming.direction);
            if reflected.max_element() <= 0.0 || incoming.direction.dot(geometric) <= 0.0 {
                continue;
            }
            let passed = if light.shadow.casts() {
                self.scene.transmittance(
                    lifted,
                    incoming.direction,
                    incoming.distance - lift,
                    true,
                    light.link,
                    time,
                )
            } else {
                Vec3::ONE
            };
            if passed.max_element() <= 0.0 {
                continue;
            }
            let light_out = self.capped(throughput * reflected * incoming.strength * passed, depth);
            if direct {
                let diffuse =
                    throughput * material.reflect_diffuse(n, v, incoming.direction) * incoming.strength * passed;
                gather.add_direct(light.group, light_out, diffuse);
            } else {
                gather.add(sampled_part(first_lobe), light.group, light_out);
            }
        }
        if !self.table.is_empty() {
            let lit = self.sample_surface_lights(hit, material, n, v, geometric, lifted, entering, rng, direct);
            let light_out = self.capped(throughput * lit.total, depth);
            if direct {
                gather.add_direct(lit.group, light_out, throughput * lit.diffuse);
            } else {
                gather.add(sampled_part(first_lobe), lit.group, light_out);
            }
        }
        if let Some(environment) = self.scene.environment() {
            let lit = sample_environment(
                self.scene,
                environment,
                material,
                (n, v, geometric),
                lifted,
                entering,
                time,
                rng,
                direct,
            );
            let light_out = self.capped(throughput * lit.total, depth);
            if direct {
                gather.add_direct(environment.group, light_out, throughput * lit.diffuse);
            } else {
                gather.add(sampled_part(first_lobe), environment.group, light_out);
            }
        }
    }

    /// The light of the nearest visible area light the ray meets before
    /// `reach`, weighed against direct sampling after a reflected bounce
    /// (`last_pdf`); dark where its back is seen, or where it does not light
    /// the surface `from` (a triangle) the ray left. `None` when it meets
    /// none.
    fn area_seen(
        &self,
        origin: Vec3,
        dir: Vec3,
        reach: f32,
        depth: u32,
        last_pdf: Option<f32>,
        from: Option<usize>,
    ) -> Option<(Vec3, u16)> {
        let (index, light, t, normal) = self
            .scene
            .areas()
            .iter()
            .enumerate()
            .filter(|(_, light)| light.visible)
            .filter_map(|(i, light)| light.intersect(origin, dir, reach).map(|(t, n)| (i, light, t, n)))
            .min_by(|a, b| a.2.total_cmp(&b.2))?;
        let Some(cos) = light.emits(normal, -dir) else {
            return Some((Vec3::ZERO, light.group));
        };
        if from.is_some_and(|triangle| !self.scene.lit_by(triangle, light.link)) {
            return Some((Vec3::ZERO, light.group));
        }
        let weight = match last_pdf {
            // Lights that are not weighed are found only by direct sampling
            // after a bounce (seen directly, or through glass, they show).
            Some(_) if !light.weighed() => 0.0,
            Some(bsdf) => {
                let point = origin + dir * t;
                let density = self.table.area_chance(index) * light.density(origin, point, normal) * t * t / cos;
                power_heuristic(bsdf, density)
            }
            None if depth > 0 && !light.weighed() => 0.0,
            None => 1.0,
        };
        Some((light.radiance * weight, light.group))
    }

    /// One sample of the light that lights with a surface (area lights,
    /// emitting triangles) send to `hit` and that it reflects towards `v`,
    /// weighed against finding the same light by a bounce (the power
    /// heuristic) where bounces can find it.
    #[expect(clippy::too_many_arguments, reason = "the hit's frame, as the loop has it")]
    fn sample_surface_lights(
        &self,
        hit: &Hit,
        material: &Material,
        n: Vec3,
        v: Vec3,
        geometric: Vec3,
        lifted: Vec3,
        entering: bool,
        rng: &mut Rng,
        diffuse: bool,
    ) -> Lit {
        let (u, s, t) = (rng.next_f32(), rng.next_f32(), rng.next_f32());
        let Some(sample) = self
            .scene
            .sample_surface_light(self.table, hit.position, (u, s, t), hit.time)
        else {
            return Lit::NONE;
        };
        if sample.source == Entry::Triangle(u32::try_from(hit.triangle).unwrap_or(u32::MAX)) {
            return Lit::NONE;
        }
        let point = sample.point;
        let to_light = point.position - hit.position;
        let distance2 = to_light.length_squared();
        if distance2 <= f32::MIN_POSITIVE {
            return Lit::NONE;
        }
        let l = to_light / distance2.sqrt();
        if l.dot(geometric) <= 0.0 {
            return Lit::NONE;
        }
        // What the light gives off towards the surface, whether bounces can
        // find it, whether things in the way stop it.
        let (emitted, cos_light, weighed, shadowed, link, group) = match sample.source {
            Entry::Triangle(triangle) => {
                let cos = point.normal.dot(l).abs();
                let triangle = usize::try_from(triangle).unwrap_or(usize::MAX);
                (
                    self.scene.emission_at(triangle, point.position, hit.time),
                    cos,
                    true,
                    true,
                    None,
                    0,
                )
            }
            Entry::Area(index) => {
                let Some(light) = usize::try_from(index).ok().and_then(|i| self.scene.areas().get(i)) else {
                    return Lit::NONE;
                };
                if !self.scene.lit_by(hit.triangle, light.link) {
                    return Lit::NONE;
                }
                let Some(cos) = light.emits(point.normal, -l) else {
                    return Lit::NONE;
                };
                (
                    light.radiance,
                    cos,
                    light.weighed(),
                    light.shadow.casts(),
                    light.link,
                    light.group,
                )
            }
        };
        if cos_light <= 1e-6 || emitted.max_element() <= 0.0 {
            return Lit::NONE;
        }
        let reflected = material.reflect(n, v, l);
        if reflected.max_element() <= 0.0 {
            return Lit::NONE;
        }
        let passed = if shadowed {
            // Up to the light, not into it: from the lifted point to one
            // lifted off the light on this side (a ray from the lifted point
            // along `l` would meet an emitting triangle's plane before its
            // end at grazing angles). Glass stops it for lights bounces can
            // find (refracted paths bring their light), and tints it for the
            // others.
            let facing = if point.normal.dot(l) > 0.0 {
                -point.normal
            } else {
                point.normal
            };
            let end = point.position + facing * offset(point.position);
            let shadow = end - lifted;
            let reach = shadow.length();
            if reach <= f32::MIN_POSITIVE {
                return Lit::NONE;
            }
            self.scene
                .transmittance(lifted, shadow / reach, reach, !weighed, link, hit.time)
        } else {
            Vec3::ONE
        };
        if passed.max_element() <= 0.0 {
            return Lit::NONE;
        }
        // The chance of this direction: per area, turned into per solid angle.
        let light_pdf = point.density * distance2 / cos_light;
        let weight = if weighed {
            power_heuristic(light_pdf, material.pdf(n, v, l, entering))
        } else {
            1.0
        };
        // `reflect` is the reflectance × π × n·l.
        let factor = emitted * passed * (weight / (PI * light_pdf));
        Lit {
            total: reflected * factor,
            diffuse: if diffuse {
                material.reflect_diffuse(n, v, l) * factor
            } else {
                Vec3::ZERO
            },
            group,
        }
    }

    /// The density per solid angle with which direct sampling picks the point
    /// of `triangle` seen at `distance2` (squared) and at cosine `cos` to its
    /// normal (0 when it is not in the table).
    fn triangle_density(&self, triangle: usize, cos: f32, distance2: f32) -> f32 {
        let area = self.scene.triangle_area(triangle);
        let cos = cos.abs();
        if area <= f32::MIN_POSITIVE || cos <= 1e-6 {
            return 0.0;
        }
        self.table.triangle_chance(triangle) / area * distance2 / cos
    }

    /// `light` as is when seen directly (`depth` 0), capped after a bounce
    /// while the clamp is on.
    fn capped(&self, light: Vec3, depth: u32) -> Vec3 {
        if depth == 0 || !self.clamp {
            light
        } else {
            light.min(Vec3::splat(BOUNCE_CAP))
        }
    }
}

/// One sample of the light that the surround sends to a surface at `lifted`
/// and that it reflects towards `v`, weighed against finding it by a bounce.
#[expect(clippy::too_many_arguments, reason = "the hit's frame, as the loop has it")]
fn sample_environment(
    scene: &RenderScene,
    environment: &EnvironmentLight,
    material: &Material,
    (n, v, geometric): (Vec3, Vec3, Vec3),
    lifted: Vec3,
    entering: bool,
    time: f32,
    rng: &mut Rng,
    diffuse: bool,
) -> Lit {
    let (u, s, t) = (rng.next_f32(), rng.next_f32(), rng.next_f32());
    let Some((l, light, density)) = environment.sample(u, s, t) else {
        return Lit::NONE;
    };
    if l.dot(geometric) <= 0.0 || light.max_element() <= 0.0 {
        return Lit::NONE;
    }
    let reflected = material.reflect(n, v, l);
    if reflected.max_element() <= 0.0 {
        return Lit::NONE;
    }
    // Out to infinity; glass stops it (refracted paths bring its light).
    let passed = scene.transmittance(lifted, l, f32::INFINITY, false, None, time);
    if passed.max_element() <= 0.0 {
        return Lit::NONE;
    }
    let weight = power_heuristic(density, material.pdf(n, v, l, entering));
    let factor = light * passed * (weight / (PI * density));
    Lit {
        total: reflected * factor,
        diffuse: if diffuse {
            material.reflect_diffuse(n, v, l) * factor
        } else {
            Vec3::ZERO
        },
        group: environment.group,
    }
}

/// Light a sample drew from a light at a surface: all it reflects, the
/// matte layer's share (when the passes ask for it) and the light's group.
#[derive(Clone, Copy, Debug)]
struct Lit {
    total: Vec3,
    diffuse: Vec3,
    group: u16,
}

impl Lit {
    const NONE: Self = Self {
        total: Vec3::ZERO,
        diffuse: Vec3::ZERO,
        group: 0,
    };
}

/// The power heuristic's weight for a sample drawn with density `own`
/// against another strategy that would draw it with density `other`.
fn power_heuristic(own: f32, other: f32) -> f32 {
    let (a, b) = (own * own, other * other);
    if a + b <= 0.0 || !(a + b).is_finite() {
        return if own > 0.0 { 1.0 } else { 0.0 };
    }
    a / (a + b)
}

/// The light a scene without lights is lit by: from over the camera's
/// shoulder, without shadows (like the viewport's default light).
fn automatic_light(camera: &RenderCamera) -> Light {
    let (_, right, up, forward) = camera.frame();
    let towards = (-forward * 0.6 + up * 0.7 - right * 0.4).normalize_or(Vec3::Y);
    Light {
        kind: LightType::Infinite,
        position: Vec3::ZERO,
        direction: -towards,
        color: Vec3::ONE,
        cos_outer: -1.0,
        cos_inner: -1.0,
        falloff: Falloff::None,
        radius: 1.0,
        shadow: LightShadow::None,
        link: None,
        group: 0,
    }
}

/// How far to lift a ray off a surface at `p` so it does not meet that surface
/// again (world units, cm): more far from the origin, where floats are coarser.
pub(crate) fn offset(p: Vec3) -> f32 {
    0.01 + p.abs().max_element() * 1e-5
}

/// A picture coordinate or count as a float (all are at most 16384, or 4096).
fn to_f32(n: u32) -> f32 {
    u16::try_from(n).map_or(f32::from(u16::MAX), f32::from)
}

#[cfg(test)]
mod tests {
    use grid4d_eval::EvaluatedScene;
    use std::sync::Arc;

    use glam::Mat4;
    use grid4d_scene::{
        Channel, Document, LightDetails, Material as SceneMaterial, MaterialId, MaterialTag, Node, NodeId, ObjectKind,
        ObjectKindTag, Projection, Tag, Texture, TextureId, TextureSlot,
    };

    use super::*;
    use crate::{CameraProjection, Picture, Pictures, Texels, Texturing};

    /// Looking straight down at the origin from 1000 cm, seeing 600 cm across.
    fn top_camera() -> RenderCamera {
        RenderCamera {
            world: glam::camera::rh::view::look_at_mat4(Vec3::new(0.0, 1000.0, 0.0), Vec3::ZERO, Vec3::NEG_Z).inverse(),
            projection: CameraProjection::Parallel { half_height: 300.0 },
            lens: None,
            end: None,
        }
    }

    fn options(size: u32, samples: u32, bounces: u32) -> RenderOptions {
        RenderOptions {
            width: size,
            height: size,
            samples,
            bounces,
            seed: 0,
            auto_light: true,
            clamp_indirect: true,
            emissive_lights: true,
            adaptive: None,
            region: None,
            displacement_budget: 20_000_000,
        }
    }

    fn scene_of(doc: &Document) -> RenderScene {
        let mut eval = EvaluatedScene::for_render();
        eval.update(&doc.scene);
        RenderScene::new(&eval)
    }

    /// A white floor (400 cm square) with a 100 cm cube on it at x = −100, and
    /// a light straight above shining down.
    fn floor_scene(shadow: LightShadow) -> Document {
        let mut doc = Document::new("floor");
        let floor = ObjectKind::Plane {
            width: 400.0,
            depth: 400.0,
            width_segments: 1,
            depth_segments: 1,
        };
        let mut white = SceneMaterial::new("White");
        (white.color, white.roughness) = ([1.0; 3], 1.0);
        let white = doc.scene.materials_mut().insert(white);
        let mut node = Node::new("Floor", floor);
        node.data.tags = vec![Tag::Material(MaterialTag::new(Some(white)))];
        doc.scene.insert(node);
        let cube = ObjectKind::Cube {
            size: Vec3::splat(100.0),
            segments: [1, 1, 1],
        };
        doc.scene
            .insert(Node::new("Cube", cube).with_position(Vec3::new(-100.0, 50.0, 0.0)));
        let sun = ObjectKind::Light {
            color: [1.0; 3],
            intensity: 1.0,
            light_type: LightType::Infinite,
            details: LightDetails::default(),
            shadow,
            linking: grid4d_scene::Linking::default(),
            group: String::new(),
        };
        // Turned to shine straight down (−Z → −Y).
        doc.scene.insert(
            Node::new("Sun", sun)
                .with_transform(grid4d_scene::Transform::default().with_rotation_deg(Vec3::new(0.0, -90.0, 0.0))),
        );
        doc
    }

    /// The pixel over the world point (x, z) of a 60 × 60 top view.
    fn at(image: &Image, x: f32, z: f32) -> [f32; 4] {
        let px = ((x / 300.0 + 1.0) * 30.0).floor();
        let py = ((z / 300.0 + 1.0) * 30.0).floor();
        let index = |v: f32| (0..60u32).find(|&i| f32::from(u16::try_from(i).unwrap()) >= v).unwrap();
        image.pixel(index(px), index(py)).unwrap()
    }

    #[test]
    fn a_lit_floor_shows_its_colour_and_the_cube_its_shadow() {
        let image = render(
            &scene_of(&floor_scene(LightShadow::Raytraced)),
            &top_camera(),
            &options(60, 4, 0),
        );
        let open = at(&image, 100.0, 0.0);
        assert!(
            (open[0] - 1.0).abs() < 0.05 && open[3] == 1.0,
            "white under a unit light: {open:?}"
        );
        // The light is straight above: the cube's top is lit too.
        let top = at(&image, -100.0, 0.0);
        assert!(top[3] == 1.0 && top[0] > 0.3, "the cube's top is lit: {top:?}");
        let outside = at(&image, 280.0, 280.0);
        assert!(outside[3] < 1.0, "beyond the floor: transparent background {outside:?}");
    }

    #[test]
    fn shadows_follow_the_light_s_shadow_setting() {
        // A sun from the side (x+) at 45°: the cube shades the floor at x < −150.
        let shaded = |shadow| {
            let mut doc = floor_scene(shadow);
            let sun = doc.scene.iter().find(|(_, n)| n.data.name == "Sun").unwrap().0;
            doc.scene.get_mut(sun).unwrap().data.transform =
                grid4d_scene::Transform::default().with_rotation_deg(Vec3::new(90.0, -45.0, 0.0));
            let image = render(&scene_of(&doc), &top_camera(), &options(60, 8, 0));
            (at(&image, -180.0, 0.0)[0], at(&image, 150.0, 0.0)[0])
        };
        let (behind, open) = shaded(LightShadow::Raytraced);
        assert!(behind < 0.05 && open > 0.6, "hard shadow: {behind} vs {open}");
        let (through, _) = shaded(LightShadow::None);
        assert!(through > 0.6, "no shadow: the light passes {through}");
    }

    #[test]
    fn a_scene_without_lights_has_the_automatic_light_and_luminance_glows() {
        let mut doc = Document::new("glow");
        let mut glow = SceneMaterial::new("Glow");
        (glow.color, glow.emission) = ([0.0; 3], [1.0, 0.5, 0.0]);
        let glow = doc.scene.materials_mut().insert(glow);
        let mut cube = Node::new("Glowing", ObjectKindTag::Cube.default_kind());
        cube.data.tags = vec![Tag::Material(MaterialTag::new(Some(glow)))];
        doc.scene.insert(cube);
        doc.scene
            .insert(Node::new("Grey", ObjectKindTag::Sphere.default_kind()).with_position(Vec3::new(200.0, 0.0, 0.0)));
        let image = render(&scene_of(&doc), &top_camera(), &options(60, 2, 0));
        let lit = at(&image, 200.0, 0.0);
        assert!(lit[0] > 0.2, "the automatic light: {lit:?}");
        let glowing = at(&image, 0.0, 0.0);
        assert!(
            (glowing[0] - 1.0).abs() < 0.05 && (glowing[1] - 0.5).abs() < 0.05 && glowing[2] < 0.02,
            "{glowing:?}"
        );

        // Auto Light off: dark, but the luminance and the coverage stay.
        let dark = RenderOptions {
            auto_light: false,
            ..options(60, 2, 0)
        };
        let image = render(&scene_of(&doc), &top_camera(), &dark);
        let unlit = at(&image, 200.0, 0.0);
        assert!(unlit[0] == 0.0 && unlit[3] == 1.0, "no light at all: {unlit:?}");
        // Exactly the luminance: no highlight from a light either.
        assert_eq!(at(&image, 0.0, 0.0), [1.0, 0.5, 0.0, 1.0], "luminance needs no light");
    }

    /// A white matte floor under a glowing panel (100 cm square, 150 cm up
    /// at x = 150, facing every way), no lights.
    fn panel_scene(emission: f32) -> Document {
        let mut doc = Document::new("panel");
        let mut white = SceneMaterial::new("White");
        (white.color, white.roughness) = ([1.0; 3], 1.0);
        let white = doc.scene.materials_mut().insert(white);
        let mut floor = Node::new(
            "Floor",
            ObjectKind::Plane {
                width: 400.0,
                depth: 400.0,
                width_segments: 1,
                depth_segments: 1,
            },
        );
        floor.data.tags = vec![Tag::Material(MaterialTag::new(Some(white)))];
        doc.scene.insert(floor);
        let mut glow = SceneMaterial::new("Glow");
        (glow.color, glow.emission) = ([0.0; 3], [emission; 3]);
        let glow = doc.scene.materials_mut().insert(glow);
        let mut panel = Node::new(
            "Panel",
            ObjectKind::Plane {
                width: 100.0,
                depth: 100.0,
                width_segments: 1,
                depth_segments: 1,
            },
        )
        .with_position(Vec3::new(150.0, 150.0, 0.0));
        panel.data.tags = vec![Tag::Material(MaterialTag::new(Some(glow)))];
        doc.scene.insert(panel);
        doc
    }

    /// The red values of the floor's pixels between x = −150 and −50.
    fn floor_values(image: &Image) -> Vec<f32> {
        (0..60u32)
            .flat_map(|y| (0..60u32).map(move |x| (x, y)))
            .filter(|&(x, y)| (15..25).contains(&x) && (20..40).contains(&y))
            .filter_map(|(x, y)| image.pixel(x, y))
            .map(|p| p[0])
            .collect()
    }

    fn mean_and_spread(values: &[f32]) -> (f32, f32) {
        let n = values.len() as f32;
        let mean = values.iter().sum::<f32>() / n;
        let variance = values.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / n;
        (mean, variance.sqrt())
    }

    #[test]
    fn emitting_surfaces_light_the_same_sampled_or_found() {
        let scene = scene_of(&panel_scene(4.0));
        let options = |samples, emissive_lights| RenderOptions {
            auto_light: false,
            emissive_lights,
            // Unclamped: the panel's light, capped after a bounce, would
            // otherwise differ between the two ways of finding it.
            clamp_indirect: false,
            ..options(60, samples, 1)
        };
        // Found only by bounces (1.1): unbiased but noisy; many samples.
        let (found, _) = mean_and_spread(&floor_values(&render(&scene, &top_camera(), &options(1024, false))));
        let (sampled, _) = mean_and_spread(&floor_values(&render(&scene, &top_camera(), &options(64, true))));
        assert!(found > 0.01, "the panel lights the floor: {found}");
        assert!(
            (sampled - found).abs() < found * 0.05,
            "the same light either way: {sampled} vs {found}"
        );
        // At the same samples, sampling the panel is far less noisy.
        let (_, noisy) = mean_and_spread(&floor_values(&render(&scene, &top_camera(), &options(16, false))));
        let (_, clean) = mean_and_spread(&floor_values(&render(&scene, &top_camera(), &options(16, true))));
        assert!(clean * 4.0 < noisy, "{clean} vs {noisy}");
        // The panel itself shows its emission, weighted or not.
        let image = render(&scene, &top_camera(), &options(4, true));
        assert!((at(&image, 150.0, 0.0)[0] - 4.0).abs() < 1e-3);
    }

    #[test]
    fn glass_hides_an_emitter_from_direct_sampling_but_not_from_paths() {
        // A glass slab between the panel and the floor: shadow rays to the
        // panel stop at it (refracted paths bring that light), so sampling
        // still agrees with paths alone.
        let mut doc = panel_scene(4.0);
        let mut glass = SceneMaterial::new("Glass");
        (glass.color, glass.roughness, glass.transmission) = ([1.0; 3], 0.05, 1.0);
        let glass = doc.scene.materials_mut().insert(glass);
        let mut slab = Node::new(
            "Slab",
            ObjectKind::Cube {
                size: Vec3::new(160.0, 10.0, 160.0),
                segments: [1, 1, 1],
            },
        )
        .with_position(Vec3::new(150.0, 120.0, 0.0));
        slab.data.tags = vec![Tag::Material(MaterialTag::new(Some(glass)))];
        doc.scene.insert(slab);
        let scene = scene_of(&doc);
        let options = |samples, emissive_lights| RenderOptions {
            auto_light: false,
            emissive_lights,
            // Unclamped: the panel's light, capped after a bounce, would
            // otherwise differ between the two ways of finding it.
            clamp_indirect: false,
            ..options(60, samples, 4)
        };
        let (found, _) = mean_and_spread(&floor_values(&render(&scene, &top_camera(), &options(1024, false))));
        let (sampled, _) = mean_and_spread(&floor_values(&render(&scene, &top_camera(), &options(1024, true))));
        assert!(
            (sampled - found).abs() < found * 0.08,
            "no light counted twice: {sampled} vs {found}"
        );
    }

    /// A white matte floor (400 cm) and one area light of `shape` and `size`
    /// at `height` above the origin, its −Z turned by `pitch` degrees (−90:
    /// straight down).
    fn area_scene(shape: grid4d_scene::AreaShape, size: [f32; 2], height: f32, pitch: f32) -> Document {
        let mut doc = Document::new("area");
        let mut white = SceneMaterial::new("White");
        (white.color, white.roughness, white.specular) = ([1.0; 3], 1.0, 0.0);
        let white = doc.scene.materials_mut().insert(white);
        let mut floor = Node::new(
            "Floor",
            ObjectKind::Plane {
                width: 400.0,
                depth: 400.0,
                width_segments: 1,
                depth_segments: 1,
            },
        );
        floor.data.tags = vec![Tag::Material(MaterialTag::new(Some(white)))];
        doc.scene.insert(floor);
        let light = ObjectKind::Light {
            color: [1.0; 3],
            intensity: 1.0,
            light_type: LightType::Area,
            details: LightDetails {
                area_shape: shape,
                area_size: size,
                ..LightDetails::default()
            },
            shadow: LightShadow::Area,
            linking: grid4d_scene::Linking::default(),
            group: String::new(),
        };
        doc.scene.insert(
            Node::new("Area", light).with_transform(
                grid4d_scene::Transform::from_position(Vec3::new(0.0, height, 0.0))
                    .with_rotation_deg(Vec3::new(0.0, pitch, 0.0)),
            ),
        );
        doc
    }

    fn set_area(doc: &mut Document, edit: impl FnOnce(&mut LightDetails)) {
        let id = doc.scene.iter().find(|(_, n)| n.data.name == "Area").unwrap().0;
        if let ObjectKind::Light { details, .. } = &mut doc.scene.get_mut(id).unwrap().data.kind {
            edit(details);
        }
    }

    /// The floor's brightness at (x, z), its mean over a few pixels.
    fn floor_at(doc: &Document, samples: u32, x: f32, z: f32) -> f32 {
        let options = RenderOptions {
            auto_light: false,
            clamp_indirect: false,
            ..options(60, samples, 0)
        };
        let image = render(&scene_of(doc), &top_camera(), &options);
        let around = [(0.0, 0.0), (10.0, 0.0), (-10.0, 0.0), (0.0, 10.0), (0.0, -10.0)];
        around.iter().map(|(dx, dz)| at(&image, x + dx, z + dz)[0]).sum::<f32>() / 5.0
    }

    #[test]
    fn a_small_area_light_lights_like_an_omni_light_with_inverse_square_falloff() {
        use grid4d_scene::AreaShape;
        // At the reference distance (500 cm), straight below: as bright as
        // an Omni light of the same intensity at its Radius/Decay; at half
        // the distance four times as bright — for every shape, seen face on.
        for (shape, pitch) in [
            (AreaShape::Rectangle, -90.0),
            (AreaShape::Disc, -90.0),
            (AreaShape::Sphere, 0.0),
            (AreaShape::Tube, 0.0),
        ] {
            let far = floor_at(&area_scene(shape, [2.0, 2.0], 500.0, pitch), 16, 100.0, 0.0);

            let cos3 = (500.0f32 / 100.0f32.hypot(500.0)).powi(3);
            let flat = if shape.is_flat() {
                cos3 * (500.0 / 100.0f32.hypot(500.0))
            } else {
                cos3
            };
            assert!((far - flat).abs() < flat * 0.06, "{shape:?} at 500 cm: {far} vs {flat}");
            // Beside it along X: a Tube (along Z) is seen side on.
            let near = floor_at(&area_scene(shape, [2.0, 2.0], 250.0, pitch), 16, 120.0, 0.0);
            let d = 120.0f32.hypot(250.0);
            let expected = (500.0 / d).powi(2) * (250.0 / d) * if shape.is_flat() { 250.0 / d } else { 1.0 };
            assert!(
                (near - expected).abs() < expected * 0.06,
                "{shape:?} at 250 cm: {near} vs {expected}"
            );
        }
    }

    #[test]
    fn a_larger_area_light_is_softer_not_brighter() {
        use grid4d_scene::AreaShape;
        let small = floor_at(
            &area_scene(AreaShape::Rectangle, [2.0, 2.0], 500.0, -90.0),
            64,
            100.0,
            0.0,
        );
        let large = floor_at(
            &area_scene(AreaShape::Rectangle, [200.0, 200.0], 500.0, -90.0),
            64,
            100.0,
            0.0,
        );
        assert!((large - small).abs() < small * 0.08, "{large} vs {small}");
    }

    #[test]
    fn a_one_sided_area_light_is_dark_behind_and_two_sided_shines_both_ways() {
        use grid4d_scene::AreaShape;
        // Facing up, away from the floor.
        let mut doc = area_scene(AreaShape::Rectangle, [50.0, 50.0], 200.0, 90.0);
        assert_eq!(floor_at(&doc, 8, 100.0, 0.0), 0.0, "nothing behind a one-sided light");
        set_area(&mut doc, |d| d.two_sided = true);
        assert!(floor_at(&doc, 8, 100.0, 0.0) > 0.5, "both faces shine");
    }

    #[test]
    fn a_visible_area_light_is_seen_and_lights_the_same() {
        use grid4d_scene::AreaShape;
        let mut doc = area_scene(AreaShape::Rectangle, [60.0, 60.0], 200.0, -90.0);
        let options = RenderOptions {
            auto_light: false,
            clamp_indirect: false,
            ..options(60, 256, 1)
        };
        let hidden = render(&scene_of(&doc), &top_camera(), &options);
        // From above the camera sees the light's back: the floor under it.
        assert!(
            at(&hidden, 0.0, 0.0)[0] > 1.0,
            "lit floor under it: {:?}",
            at(&hidden, 0.0, 0.0)
        );
        set_area(&mut doc, |d| d.area_visible = true);
        let shown = render(&scene_of(&doc), &top_camera(), &options);
        assert_eq!(at(&shown, 0.0, 0.0)[0], 0.0, "its dark back hides the floor");
        // Seen from below (turned to face up, the camera sees its face).
        let mut up = area_scene(AreaShape::Rectangle, [60.0, 60.0], 200.0, 90.0);
        set_area(&mut up, |d| d.area_visible = true);
        let face = render(&scene_of(&up), &top_camera(), &options);
        assert!(
            at(&face, 0.0, 0.0)[0] > 10.0,
            "its bright face: {:?}",
            at(&face, 0.0, 0.0)
        );
        // Found by bounces too (multiple importance sampling), the floor beside
        // it gets the same light as when only sampled directly.
        let mean = |image: &Image| floor_values(image).iter().sum::<f32>() / 200.0;
        let (a, b) = (mean(&hidden), mean(&shown));
        assert!((a - b).abs() < a * 0.03, "{a} vs {b}");
    }

    #[test]
    fn the_indirect_clamp_caps_only_light_after_a_bounce() {
        let doc = Document::new("t");
        let scene = scene_of(&doc);
        let camera = top_camera();
        let table = LightTable::default();
        let job = |clamp| Job {
            scene: &scene,
            lights: &[],
            camera: &camera,
            width: 1,
            height: 1,
            bounces: 1,
            seed: 0,
            clamp,
            table: &table,
            moving: false,
            layout: None,
        };
        let bright = Vec3::splat(50.0);
        assert_eq!(job(true).capped(bright, 0), bright, "seen directly");
        assert_eq!(job(true).capped(bright, 1), Vec3::splat(BOUNCE_CAP));
        assert_eq!(job(false).capped(bright, 1), bright, "unclamped");
        assert!((power_heuristic(1.0, 1.0) - 0.5).abs() < 1e-6);
        assert_eq!(power_heuristic(2.0, 0.0), 1.0);
        assert_eq!(power_heuristic(0.0, 0.0), 0.0);
    }

    #[test]
    fn options_come_from_the_render_settings() {
        let settings = RenderSettings {
            width: 320,
            height: 200,
            samples: 7,
            bounces: 3,
            auto_light: false,
            clamp_indirect: false,
            emissive_lights: false,
            ..RenderSettings::default()
        };
        assert_eq!(
            RenderOptions::from(&settings),
            RenderOptions {
                width: 320,
                height: 200,
                samples: 7,
                bounces: 3,
                seed: 0,
                auto_light: false,
                clamp_indirect: false,
                emissive_lights: false,
                adaptive: None,
                region: None,
                displacement_budget: 20_000_000,
            }
        );
        let defaults = RenderOptions::from(&RenderSettings::default());
        assert_eq!(
            (
                defaults.samples,
                defaults.bounces,
                defaults.auto_light,
                defaults.clamp_indirect,
                defaults.emissive_lights
            ),
            (16, 2, true, true, true),
            "the same quality as the renderer's own defaults"
        );
    }

    #[test]
    fn renders_are_repeatable_and_bounces_brighten() {
        let scene = scene_of(&Document::with_demo_scene());
        let camera = RenderCamera::framing(scene.bounds().unwrap().0, scene.bounds().unwrap().1, 40f32.to_radians());
        let a = render(&scene, &camera, &options(24, 4, 1));
        let b = render(&scene, &camera, &options(24, 4, 1));
        assert_eq!(a, b, "the same picture every time");
        let single = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap()
            .install(|| render(&scene, &camera, &options(24, 4, 1)));
        assert_eq!(a, single, "one thread or many");
        let reseeded = render(
            &scene,
            &camera,
            &RenderOptions {
                seed: 5,
                ..options(24, 4, 1)
            },
        );
        assert_ne!(a, reseeded, "another seed, another noise");
    }

    #[test]
    fn passes_add_up_to_the_same_picture() {
        let scene = scene_of(&Document::with_demo_scene());
        let (min, max) = scene.bounds().unwrap();
        let camera = RenderCamera::framing(min, max, 40f32.to_radians());
        let options = options(20, 7, 1);
        let whole = render(&scene, &camera, &options);

        let control = RenderControl::default();
        let mut progressive = Progressive::new(&options);
        assert_eq!(progressive.image().pixels[0], [0.0; 4], "transparent before any pass");
        for count in [1, 1, 2, 100] {
            assert!(progressive.add_samples(&scene, &camera, &options, count, &control));
        }
        assert!(progressive.is_complete(&options));
        assert_eq!(progressive.samples(), 7, "never more than asked for");
        assert_eq!(progressive.image(), whole, "exactly the same picture");
        assert_eq!(control.progress(), 20 * 20 * 7, "pixels × samples");

        let mut first = Progressive::new(&options);
        first.add_samples(&scene, &camera, &options, 1, &control);
        assert_ne!(first.image(), whole, "one sample is noisier");
    }

    /// Adaptive sampling from 8 samples, up to 128.
    fn adaptive(size: u32) -> RenderOptions {
        RenderOptions {
            adaptive: Some(Adaptive {
                min_samples: 8,
                threshold: 0.01,
            }),
            ..options(size, 128, 0)
        }
    }

    #[test]
    fn adaptive_sampling_spends_samples_on_edges_only() {
        let doc = floor_scene(LightShadow::None);
        let scene = scene_of(&doc);
        // 64 pixels over 600 cm: edges run through pixels, not between them.
        let image = render(&scene, &top_camera(), &adaptive(64));
        let count = |x: f32, z: f32| {
            let px = ((x / 300.0 + 1.0) * 32.0).floor() as usize;
            let py = ((z / 300.0 + 1.0) * 32.0).floor() as usize;
            image.sample_counts[py * 64 + px]
        };
        // The evenly lit floor and the empty background stop at the minimum.
        assert_eq!(count(150.0, 0.0), 8);
        assert_eq!(count(290.0, 290.0), 8);
        // The cube's edge over the floor and the floor's over the background
        // get more.
        assert!(count(-50.0, 0.0) > 8, "{}", count(-50.0, 0.0));
        assert!(count(200.0, 100.0) > 8, "{}", count(200.0, 100.0));
        let total: u32 = image.sample_counts.iter().sum();
        assert!(total < 64 * 64 * 48, "far fewer samples than 128 each: {total}");
        // Without adaptive sampling nothing is counted per pixel.
        assert!(
            render(&scene, &top_camera(), &options(8, 4, 0))
                .sample_counts
                .is_empty()
        );
    }

    #[test]
    fn adaptive_renders_are_the_same_however_they_are_cut_and_shared() {
        let doc = floor_scene(LightShadow::Raytraced);
        let scene = scene_of(&doc);
        let options = RenderOptions {
            bounces: 1,
            ..adaptive(24)
        };
        let whole = render(&scene, &top_camera(), &options);
        let control = RenderControl::default();
        let mut progressive = Progressive::new(&options);
        for count in [1, 3, 5, 7, 1000] {
            assert!(progressive.add_samples(&scene, &top_camera(), &options, count, &control));
        }
        assert!(progressive.is_complete(&options));
        assert_eq!(progressive.image(), whole);
        assert_eq!(control.progress(), 24 * 24 * 128, "stopped pixels count as done");
        let single = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap()
            .install(|| render(&scene, &top_camera(), &options));
        assert_eq!(single, whole, "one thread or many");
    }

    #[test]
    fn a_region_renders_its_pixels_as_the_whole_picture_does() {
        let doc = floor_scene(LightShadow::Raytraced);
        let scene = scene_of(&doc);
        for adaptive in [None, adaptive(32).adaptive] {
            let whole_options = RenderOptions {
                adaptive,
                ..options(32, 16, 1)
            };
            let whole = render(&scene, &top_camera(), &whole_options);
            let options = RenderOptions {
                region: Some([5, 8, 20, 30]),
                ..whole_options
            };
            assert_eq!(options.pixel_count(), 15 * 22);
            let control = RenderControl::default();
            let mut progressive = Progressive::new(&options);
            progressive.add_samples(&scene, &top_camera(), &options, u32::MAX, &control);
            let part = progressive.image();
            for y in 0..32 {
                for x in 0..32 {
                    let i = y * 32 + x;
                    // Adaptive decisions look at neighbours, which differ at
                    // the region's edge: there only the samples are the same.
                    if (5..20).contains(&x) && (8..30).contains(&y) {
                        if adaptive.is_none() {
                            assert_eq!(part.pixels[i], whole.pixels[i], "({x}, {y}) inside");
                        } else {
                            assert!(part.sample_counts[i] >= 8, "({x}, {y}) inside");
                        }
                    } else {
                        assert_eq!(part.pixels[i], [0.0; 4], "({x}, {y}) outside");
                    }
                }
            }
            assert_eq!(control.progress(), 15 * 22 * 16, "only the region's work");
        }
        // A region past the picture is cut to it.
        let options = RenderOptions {
            region: Some([30, 30, 99, 99]),
            ..options(32, 1, 0)
        };
        assert_eq!(options.rendered(), [30, 30, 32, 32]);
    }

    #[test]
    fn a_cancelled_render_stops() {
        let scene = scene_of(&Document::with_demo_scene());
        let (min, max) = scene.bounds().unwrap();
        let camera = RenderCamera::framing(min, max, 40f32.to_radians());
        let options = options(16, 4, 1);
        let control = RenderControl::default();
        control.cancel();
        let mut progressive = Progressive::new(&options);
        assert!(!progressive.add_samples(&scene, &camera, &options, 4, &control));
        assert_eq!(progressive.samples(), 0);
        assert_eq!(control.progress(), 0, "no row was rendered");
    }

    #[test]
    fn splines_are_not_rendered() {
        let mut doc = Document::new("t");
        doc.scene
            .insert(Node::new("Circle", ObjectKindTag::Circle.default_kind()));
        assert_eq!(scene_of(&doc).triangle_count(), 0, "a curve has no surface");
    }

    #[test]
    fn an_extruded_spline_is_rendered_as_a_solid() {
        let mut doc = Document::new("t");
        let extrude = doc
            .scene
            .insert(Node::new("Extrude", ObjectKindTag::Extrude.default_kind()));
        doc.scene
            .insert_child(extrude, Node::new("Rectangle", ObjectKindTag::Rectangle.default_kind()))
            .unwrap();
        // Four walls of two triangles and two square caps of two: the curve itself adds none.
        assert_eq!(scene_of(&doc).triangle_count(), 12);
    }

    #[test]
    fn a_lathed_spline_is_rendered() {
        let mut doc = Document::new("t");
        let lathe = doc
            .scene
            .insert(Node::new("Lathe", ObjectKindTag::Lathe.default_kind()));
        let mut rectangle = Node::new("Rectangle", ObjectKindTag::Rectangle.default_kind());
        rectangle.data.transform.position.x = 400.0;
        doc.scene.insert_child(lathe, rectangle).unwrap();
        // 24 steps round, four sides of two triangles each.
        assert_eq!(scene_of(&doc).triangle_count(), 24 * 4 * 2);
    }

    #[test]
    fn a_swept_spline_is_rendered() {
        let mut doc = Document::new("t");
        let sweep = doc
            .scene
            .insert(Node::new("Sweep", ObjectKindTag::Sweep.default_kind()));
        doc.scene
            .insert_child(sweep, Node::new("Rectangle", ObjectKindTag::Rectangle.default_kind()))
            .unwrap();
        doc.scene
            .insert_child(sweep, Node::new("Helix", ObjectKindTag::Helix.default_kind()))
            .unwrap();
        assert!(scene_of(&doc).triangle_count() > 100);
    }

    #[test]
    fn a_lofted_skin_is_rendered() {
        let mut doc = Document::new("t");
        let loft = doc.scene.insert(Node::new("Loft", ObjectKindTag::Loft.default_kind()));
        for z in [0.0, 300.0] {
            let mut circle = Node::new("Circle", ObjectKindTag::Circle.default_kind());
            circle.data.transform.position.z = z;
            doc.scene.insert_child(loft, circle).unwrap();
        }
        // 29 around × 9 rings of quads, and two 29-gon caps of 27 triangles.
        assert_eq!(scene_of(&doc).triangle_count(), 29 * 9 * 2 + 2 * 27);
    }

    #[test]
    fn extruded_text_is_rendered() {
        let mut doc = Document::new("t");
        let extrude = doc
            .scene
            .insert(Node::new("Extrude", ObjectKindTag::Extrude.default_kind()));
        doc.scene
            .insert_child(extrude, Node::new("Text", ObjectKindTag::Text.default_kind()))
            .unwrap();
        assert!(scene_of(&doc).triangle_count() > 200);
    }

    #[test]
    fn a_bent_object_is_rendered_bent() {
        let mut doc = Document::new("t");
        let cube = doc.scene.insert(Node::new(
            "Cube",
            ObjectKind::Cube {
                size: Vec3::new(20.0, 200.0, 20.0),
                segments: [1, 20, 1],
            },
        ));
        let straight = scene_of(&doc).bounds().unwrap();
        let mut bend = Node::new("Bend", ObjectKindTag::Bend.default_kind());
        if let ObjectKind::Bend(b) = &mut bend.data.kind {
            b.size = Vec3::new(100.0, 200.0, 100.0);
            b.strength = std::f32::consts::FRAC_PI_2;
        }
        doc.scene.insert_child(cube, bend).unwrap();
        let bent = scene_of(&doc).bounds().unwrap();
        assert!(bent.1.x > straight.1.x + 50.0, "{bent:?} vs {straight:?}");
    }

    /// A 2 × 1 picture: `left` then `right`.
    fn two_texels(left: [u8; 4], right: [u8; 4], srgb: bool) -> Arc<Picture> {
        let bytes: Arc<[u8]> = Arc::from([left, right].concat());
        let texels = if srgb {
            Texels::Srgb8(bytes)
        } else {
            Texels::Linear8(bytes)
        };
        Arc::new(Picture::new(2, 1, texels).unwrap())
    }

    /// `doc` as the renderer sees it in its materials with `pictures`,
    /// Camera projections through `camera` (or the camera objects `cameras`).
    fn textured_scene(
        doc: &Document,
        pictures: &Pictures,
        camera: RenderCamera,
        cameras: &[(NodeId, RenderCamera)],
    ) -> RenderScene {
        let mut eval = EvaluatedScene::for_render();
        eval.update(&doc.scene);
        let texturing = Texturing {
            materials: doc.scene.materials(),
            pictures,
            camera,
            aspect: 1.0,
            cameras,
            displacement_budget: 20_000_000,
        };
        RenderScene::textured(&eval, &texturing)
    }

    /// A 400 cm floor carrying `tags`, lit from above (`tilt` degrees
    /// towards +Z from straight down).
    fn floor_with(doc: &mut Document, tags: Vec<Tag>, shadow: LightShadow, tilt: f32) {
        let floor = ObjectKind::Plane {
            width: 400.0,
            depth: 400.0,
            width_segments: 1,
            depth_segments: 1,
        };
        let mut node = Node::new("Floor", floor);
        node.data.tags = tags;
        doc.scene.insert(node);
        let sun = ObjectKind::Light {
            color: [1.0; 3],
            intensity: 1.0,
            light_type: LightType::Infinite,
            details: LightDetails::default(),
            shadow,
            linking: grid4d_scene::Linking::default(),
            group: String::new(),
        };
        doc.scene.insert(
            Node::new("Sun", sun).with_transform(grid4d_scene::Transform::default().with_rotation_deg(Vec3::new(
                0.0,
                tilt - 90.0,
                0.0,
            ))),
        );
    }

    /// A white matte material whose `channel` reads `texture`.
    fn mapped(channel: Channel, texture: TextureId) -> SceneMaterial {
        let mut material = SceneMaterial::new("Mapped");
        (material.color, material.roughness) = ([1.0; 3], 1.0);
        *material.map_mut(channel) = Some(TextureSlot::new(texture));
        material
    }

    fn tag(material: MaterialId) -> Tag {
        Tag::Material(MaterialTag::new(Some(material)))
    }

    fn texture(doc: &mut Document) -> TextureId {
        doc.scene.textures_mut().insert(Texture::linked("picture.png".into()))
    }

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const BLACK: [u8; 4] = [0, 0, 0, 255];

    #[test]
    fn pictures_colour_surfaces_through_their_coordinates() {
        let mut doc = Document::new("t");
        let texture = texture(&mut doc);
        let material = doc.scene.materials_mut().insert(mapped(Channel::BaseColor, texture));
        floor_with(&mut doc, vec![tag(material)], LightShadow::None, 0.0);
        let mut pictures = Pictures::new();
        pictures.insert(texture, two_texels(RED, BLUE, true));
        let scene = textured_scene(&doc, &pictures, top_camera(), &[]);
        let image = render(&scene, &top_camera(), &options(60, 2, 0));
        // u runs along +X: red on the left, blue on the right (the texels'
        // centres at u = 0.25 and 0.75).
        let (left, right) = (at(&image, -100.0, 0.0), at(&image, 100.0, 0.0));
        assert!(left[0] > 0.9 && left[2] < 0.05, "{left:?}");
        assert!(right[2] > 0.9 && right[0] < 0.05, "{right:?}");
        // Without the picture (missing, loading): the channel's value, white.
        let plain = textured_scene(&doc, &Pictures::new(), top_camera(), &[]);
        let white = at(&render(&plain, &top_camera(), &options(60, 2, 0)), -100.0, 0.0);
        assert!(white[0] > 0.9 && white[2] > 0.9, "{white:?}");
    }

    #[test]
    fn layers_cover_the_ones_below_where_their_opacity_lets_them() {
        let mut doc = Document::new("t");
        let mask = texture(&mut doc);
        let mut red = SceneMaterial::new("Red");
        (red.color, red.roughness) = ([1.0, 0.0, 0.0], 1.0);
        let red = doc.scene.materials_mut().insert(red);
        let mut label = mapped(Channel::Opacity, mask);
        label.color = [0.0, 0.0, 1.0];
        let label = doc.scene.materials_mut().insert(label);
        floor_with(&mut doc, vec![tag(red), tag(label)], LightShadow::None, 0.0);
        let mut pictures = Pictures::new();
        // Solid on the left, cut out on the right.
        pictures.insert(mask, two_texels(RED, BLACK, false));
        let scene = textured_scene(&doc, &pictures, top_camera(), &[]);
        let image = render(&scene, &top_camera(), &options(60, 2, 0));
        let (left, right) = (at(&image, -100.0, 0.0), at(&image, 100.0, 0.0));
        assert!(left[2] > 0.9 && left[0] < 0.05, "the label: {left:?}");
        assert!(right[0] > 0.9 && right[2] < 0.05, "the red below: {right:?}");

        // The label alone: where it is cut out, nothing is there.
        let mut alone = Document::new("t");
        let mask = texture(&mut alone);
        let label = alone.scene.materials_mut().insert(mapped(Channel::Opacity, mask));
        floor_with(&mut alone, vec![tag(label)], LightShadow::None, 0.0);
        let mut pictures = Pictures::new();
        pictures.insert(mask, two_texels(RED, BLACK, false));
        let scene = textured_scene(&alone, &pictures, top_camera(), &[]);
        let image = render(&scene, &top_camera(), &options(60, 2, 0));
        assert_eq!(at(&image, -100.0, 0.0)[3], 1.0);
        assert_eq!(at(&image, 100.0, 0.0)[3], 0.0, "the background shows through");
    }

    #[test]
    fn glass_lets_tinted_light_through() {
        let glass_cube = |transmission: f32| {
            let mut doc = Document::new("t");
            let mut white = SceneMaterial::new("White");
            (white.color, white.roughness) = ([1.0; 3], 1.0);
            let white = doc.scene.materials_mut().insert(white);
            // A little off straight down: the sun's mirror image in the
            // glass misses the camera.
            floor_with(&mut doc, vec![tag(white)], LightShadow::Raytraced, 15.0);
            let mut glass = SceneMaterial::new("Glass");
            (glass.color, glass.roughness, glass.transmission) = ([1.0, 0.4, 0.4], 0.05, transmission);
            let glass = doc.scene.materials_mut().insert(glass);
            let cube = ObjectKind::Cube {
                size: Vec3::splat(100.0),
                segments: [1, 1, 1],
            };
            let mut cube = Node::new("Cube", cube).with_position(Vec3::new(-100.0, 50.0, 0.0));
            cube.data.tags = vec![tag(glass)];
            doc.scene.insert(cube);
            let scene = textured_scene(&doc, &Pictures::new(), top_camera(), &[]);
            at(&render(&scene, &top_camera(), &options(60, 16, 3)), -100.0, 0.0)
        };
        // Through the glass: the lit floor below, tinted red on the way.
        let through = glass_cube(1.0);
        assert!(through[0] > 0.4 && through[0] > through[1] * 3.0, "{through:?}");
        // An opaque cube of the same colour shows its own top.
        let solid = glass_cube(0.0);
        assert!(solid[1] > 0.2 && solid[0] < solid[1] * 3.0, "{solid:?}");
    }

    #[test]
    fn camera_projections_look_through_their_camera_object() {
        let mut doc = Document::new("t");
        let texture = texture(&mut doc);
        let material = doc.scene.materials_mut().insert(mapped(Channel::BaseColor, texture));
        let lens = doc
            .scene
            .insert(Node::new("Lens", ObjectKindTag::Camera.default_kind()));
        let mut projected = MaterialTag::new(Some(material));
        projected.projection = Projection::Camera;
        projected.camera = Some(lens);
        floor_with(&mut doc, vec![Tag::Material(projected)], LightShadow::None, 0.0);
        let mut pictures = Pictures::new();
        pictures.insert(texture, two_texels(RED, BLUE, true));
        // The camera object looks down from x = 200: x = 100 is on the left
        // of its picture, mostly red (by UV it would be blue).
        let shifted = RenderCamera {
            world: Mat4::from_translation(Vec3::new(200.0, 0.0, 0.0)) * top_camera().world,
            ..top_camera()
        };
        let scene = textured_scene(&doc, &pictures, top_camera(), &[(lens, shifted)]);
        let image = render(&scene, &top_camera(), &options(60, 2, 0));
        let shifted_view = at(&image, 100.0, 0.0);
        assert!(shifted_view[0] > 0.7 && shifted_view[2] < 0.3, "{shifted_view:?}");
        // Without that object, the render's own camera: x = 100 on its right.
        let scene = textured_scene(&doc, &pictures, top_camera(), &[]);
        let own = at(&render(&scene, &top_camera(), &options(60, 2, 0)), 100.0, 0.0);
        assert!(own[2] > 0.7 && own[0] < 0.3, "{own:?}");
    }

    #[test]
    fn normal_and_bump_pictures_tilt_the_shading() {
        let lit = |channel: Channel, left: [u8; 4], right: [u8; 4], height: f32| {
            let mut doc = Document::new("t");
            let texture = texture(&mut doc);
            let mut material = mapped(channel, texture);
            material.bump_height = height;
            let material = doc.scene.materials_mut().insert(material);
            floor_with(&mut doc, vec![tag(material)], LightShadow::None, 0.0);
            let mut pictures = Pictures::new();
            pictures.insert(texture, two_texels(left, right, false));
            let scene = textured_scene(&doc, &pictures, top_camera(), &[]);
            at(&render(&scene, &top_camera(), &options(60, 2, 0)), 0.0, 0.0)[0]
        };
        // A normal tilted 45° (the picture's x and z equal): cos 45° of the light.
        let tilted = [218, 128, 218, 255];
        let normal = lit(Channel::Normal, tilted, tilted, 0.1);
        assert!((normal - 0.707).abs() < 0.06, "{normal}");
        let flat = [128, 128, 255, 255];
        let straight = lit(Channel::Normal, flat, flat, 0.1);
        assert!((straight - 1.0).abs() < 0.05, "{straight}");
        // A height rising 200 cm over the 200 cm between the texels' centres:
        // a 45° slope in the middle; a low one barely tilts.
        let steep = lit(Channel::Bump, BLACK, RED, 200.0);
        assert!((steep - 0.707).abs() < 0.06, "{steep}");
        let gentle = lit(Channel::Bump, BLACK, RED, 1.0);
        assert!(gentle > 0.98, "{gentle}");
    }

    /// A white matte ball (radius 150 cm) at the origin and `surround`.
    fn ball_under(surround: ObjectKind) -> Document {
        let mut doc = Document::new("ball");
        let mut white = SceneMaterial::new("White");
        (white.color, white.roughness, white.specular) = ([1.0; 3], 1.0, 0.0);
        let white = doc.scene.materials_mut().insert(white);
        let mut ball = Node::new(
            "Ball",
            ObjectKind::Sphere {
                radius: 150.0,
                segments: 48,
            },
        );
        ball.data.tags = vec![Tag::Material(MaterialTag::new(Some(white)))];
        doc.scene.insert(ball);
        doc.scene.insert(Node::new("Surround", surround));
        doc
    }

    /// The mean red of the 5 × 5 pixels around the world point (x, z) of a
    /// 60 × 60 top view.
    fn around(image: &Image, x: f32, z: f32) -> f32 {
        let mut sum = 0.0;
        for dx in [-20.0, -10.0, 0.0, 10.0, 20.0] {
            for dz in [-20.0, -10.0, 0.0, 10.0, 20.0] {
                sum += at(image, x + dx, z + dz)[0];
            }
        }
        sum / 25.0
    }

    /// An Environment without a picture: a uniform grey of half `intensity`.
    fn grey(intensity: f32, visible: bool) -> ObjectKind {
        ObjectKind::Environment(grid4d_scene::Environment {
            intensity,
            visible,
            ..grid4d_scene::Environment::default()
        })
    }

    #[test]
    fn a_white_ball_under_a_uniform_surround_shows_its_value() {
        // The furnace: 0.6 from everywhere, so a white diffuse ball is 0.6
        // wherever it is seen — direct light only or with bounces (it sees
        // nothing of itself), and no Auto Light added.
        for bounces in [0, 2] {
            let options = RenderOptions {
                clamp_indirect: false,
                ..options(60, 64, bounces)
            };
            let image = render(&scene_of(&ball_under(grey(1.2, true))), &top_camera(), &options);
            for (x, z) in [(0.0, 0.0), (80.0, 0.0), (0.0, -100.0)] {
                let p = around(&image, x, z);
                assert!((p - 0.6).abs() < 0.02, "{bounces} bounces at {x}, {z}: {p}");
            }
            // The background: the surround itself, opaque.
            let corner = at(&image, 290.0, 290.0);
            assert!((corner[0] - 0.6).abs() < 1e-4 && corner[3] == 1.0, "{corner:?}");
        }
    }

    #[test]
    fn visible_to_camera_changes_only_the_background() {
        let options = options(60, 16, 1);
        let shown = render(&scene_of(&ball_under(grey(1.2, true))), &top_camera(), &options);
        let hidden = render(&scene_of(&ball_under(grey(1.2, false))), &top_camera(), &options);
        assert_eq!(at(&hidden, 290.0, 290.0), [0.0; 4], "transparent");
        assert_eq!(at(&shown, 0.0, 0.0), at(&hidden, 0.0, 0.0), "the ball lit the same");
    }

    #[test]
    fn the_topmost_visible_surround_lights_the_scene() {
        let mut doc = ball_under(grey(1.2, true));
        // Below the first in the Object Manager: ignored while it is there.
        doc.scene.insert(Node::new("Dim", grey(0.2, true)));
        let options = RenderOptions {
            clamp_indirect: false,
            ..options(60, 32, 0)
        };
        let lit = |doc: &Document| around(&render(&scene_of(doc), &top_camera(), &options), 0.0, 0.0);
        assert!((lit(&doc) - 0.6).abs() < 0.03, "{}", lit(&doc));
        let first = doc.scene.iter().find(|(_, n)| n.data.name == "Surround").unwrap().0;
        doc.scene.get_mut(first).unwrap().data.visible_in_render = false;
        assert!((lit(&doc) - 0.1).abs() < 0.01, "{}", lit(&doc));
    }

    #[test]
    fn the_sky_lights_with_its_sun_and_its_shadow_is_filled_by_the_sky() {
        use grid4d_eval::SkyModel;
        let mut doc = floor_scene(LightShadow::None);
        let sun = doc.scene.iter().find(|(_, n)| n.data.name == "Sun").unwrap().0;
        doc.scene.remove(sun);
        // The sun 60° up in the east (+X): the cube's shadow falls west of it.
        let sky = grid4d_scene::Sky {
            placement: grid4d_scene::SunPlacement::Angles,
            elevation: 60f32.to_radians(),
            azimuth: 90f32.to_radians(),
            ..grid4d_scene::Sky::default()
        };
        doc.scene.insert(Node::new("Sky", ObjectKind::Sky(sky.clone())));
        let options = RenderOptions {
            clamp_indirect: false,
            ..options(60, 64, 0)
        };
        let image = render(&scene_of(&doc), &top_camera(), &options);
        let model = SkyModel::new(&sky, Vec3::Z);
        let direct = model.sun().unwrap().irradiance.y * 60f32.to_radians().sin();
        let lit = at(&image, 100.0, 0.0)[1];
        let shade = at(&image, -180.0, 0.0)[1];
        assert!(lit > direct && lit < direct * 1.6, "{lit} vs the sun's {direct}");
        assert!(
            shade > 0.02 && shade < lit * 0.4,
            "{shade} in the shadow, {lit} in the sun"
        );
    }

    #[test]
    fn a_linked_light_lights_only_its_objects_and_shadows_only_them() {
        use grid4d_scene::LinkMode;
        // The sun turned 45° towards −Z: the cube's shadow falls on the floor
        // beside it, where the top view sees it.
        let mut doc = floor_scene(LightShadow::Raytraced);
        let id = |doc: &Document, name: &str| doc.scene.iter().find(|(_, n)| n.data.name == name).unwrap().0;
        let sun = id(&doc, "Sun");
        doc.scene.get_mut(sun).unwrap().data.transform =
            grid4d_scene::Transform::default().with_rotation_deg(Vec3::new(0.0, -45.0, 0.0));
        let link = |doc: &mut Document, mode: LinkMode, objects: Vec<NodeId>| {
            if let ObjectKind::Light { linking, .. } = &mut doc.scene.get_mut(sun).unwrap().data.kind {
                *linking = grid4d_scene::Linking { mode, objects };
            }
        };
        let look = |doc: &Document| {
            let image = render(&scene_of(doc), &top_camera(), &options(60, 4, 0));
            // The floor in the cube's shadow, in the open, and the cube's top.
            [
                at(&image, -100.0, -110.0)[0],
                at(&image, 100.0, 0.0)[0],
                at(&image, -100.0, 0.0)[0],
            ]
        };
        let [shade, open, top] = look(&doc);
        assert!(shade < 0.01 && open > 0.6 && top > 0.2, "{shade} {open} {top}");
        // The cube left out: unlit, and its shadow gone.
        let (cube, floor) = (id(&doc, "Cube"), id(&doc, "Floor"));
        link(&mut doc, LinkMode::Exclude, vec![cube]);
        let [shade, open, top] = look(&doc);
        assert!((shade - open).abs() < 0.01 && top < 0.01, "{shade} {open} {top}");
        // Only the floor: the same.
        link(&mut doc, LinkMode::Include, vec![floor]);
        assert_eq!(look(&doc), [shade, open, top]);
        // The floor left out: dark.
        link(&mut doc, LinkMode::Exclude, vec![floor]);
        assert!(look(&doc)[1] < 0.01);
        // A deleted object listed does nothing.
        let gone = doc.scene.insert(Node::new("Gone", ObjectKind::Null));
        doc.scene.remove(gone);
        link(&mut doc, LinkMode::Exclude, vec![gone]);
        let [shade, open, _] = look(&doc);
        assert!(shade < 0.01 && open > 0.6);
    }

    /// A small glowing ball (radius 1 cm) `distance` cm in front of a
    /// perspective camera at the origin looking along −Z (40°), on black.
    fn glowing_point(distance: f32) -> (Document, RenderCamera) {
        let mut doc = Document::new("point");
        let mut glow = SceneMaterial::new("Glow");
        (glow.color, glow.emission) = ([0.0; 3], [1.0; 3]);
        let glow = doc.scene.materials_mut().insert(glow);
        let mut ball = Node::new(
            "Ball",
            ObjectKind::Sphere {
                radius: 1.0,
                segments: 12,
            },
        )
        .with_position(Vec3::new(0.0, 0.0, -distance));
        ball.data.tags = vec![Tag::Material(MaterialTag::new(Some(glow)))];
        doc.scene.insert(ball);
        let camera = RenderCamera::still(
            Mat4::IDENTITY,
            CameraProjection::Perspective {
                fov_y: 40f32.to_radians(),
            },
        );
        (doc, camera)
    }

    /// The radius (pixels) of the spot an image shows: from the brightness-
    /// weighted spread about its centre (a uniform disc's mean r² is R² / 2).
    fn spot_radius(image: &Image) -> f32 {
        let (mut total, mut cx, mut cy) = (0.0f64, 0.0f64, 0.0f64);
        let mut pixels = Vec::new();
        for y in 0..image.height {
            for x in 0..image.width {
                let v = f64::from(image.pixel(x, y).unwrap()[0]);
                pixels.push((f64::from(x) + 0.5, f64::from(y) + 0.5, v));
                total += v;
                cx += v * (f64::from(x) + 0.5);
                cy += v * (f64::from(y) + 0.5);
            }
        }
        let (cx, cy) = (cx / total, cy / total);
        let r2 = pixels
            .iter()
            .map(|(x, y, v)| v * ((x - cx).powi(2) + (y - cy).powi(2)))
            .sum::<f64>()
            / total;
        (2.0 * r2).sqrt() as f32
    }

    #[test]
    fn the_focus_plane_is_sharp_and_the_blur_follows_the_thin_lens() {
        let (doc, pinhole) = glowing_point(400.0);
        let options = RenderOptions {
            auto_light: false,
            ..options(100, 256, 0)
        };
        let lens = |focus_distance| RenderCamera {
            lens: Some(crate::ThinLens {
                radius: 10.0,
                focus_distance,
                blades: 0,
                rotation: 0.0,
            }),
            ..pinhole
        };
        let sharp = spot_radius(&render(&scene_of(&doc), &lens(400.0), &options));
        assert!(sharp < 1.0, "in focus: {sharp} px");
        // Focused at 200 cm: on the focus plane the blur is the aperture ×
        // |1 − 200 / 400| = 10 cm across, the plane 2 × 200 × tan 20° high
        // in 100 pixels.
        let blurred = spot_radius(&render(&scene_of(&doc), &lens(200.0), &options));
        let expected = 10.0 / (2.0 * 200.0 * 20f32.to_radians().tan()) * 100.0 / 2.0;
        assert!((blurred - expected).abs() < 1.0, "{blurred} vs {expected} px");
        // Without a lens the samples are drawn as before: the same picture.
        let plain = render(&scene_of(&doc), &pinhole, &options);
        let again = render(
            &scene_of(&doc),
            &RenderCamera {
                lens: None,
                ..lens(200.0)
            },
            &options,
        );
        assert_eq!(plain, again);
    }

    #[test]
    fn a_moving_object_smears_over_its_path() {
        // A 20 cm cube moving from x = −50 to x = +50 over the shutter.
        let at = |x: f32| {
            let mut doc = Document::new("move");
            doc.scene.insert(
                Node::new(
                    "Cube",
                    ObjectKind::Cube {
                        size: Vec3::splat(20.0),
                        segments: [1, 1, 1],
                    },
                )
                .with_position(Vec3::new(x, 0.0, 0.0)),
            );
            scene_of(&doc)
        };
        let (moving, still) = RenderScene::moving(vec![at(-50.0), at(0.0), at(50.0)], 1);
        assert!(moving.is_moving() && still == 0);
        let image = render(&moving, &top_camera(), &options(60, 64, 0));
        // Covered along the row through the middle: from −60 to +60 cm (12
        // pixels of 10 cm), each part for part of the time.
        let covered: Vec<u32> = (0..60).filter(|&x| image.pixel(x, 30).unwrap()[3] > 0.05).collect();
        let width = covered.last().unwrap() - covered.first().unwrap() + 1;
        assert!((11..=13).contains(&width), "{width} pixels: {covered:?}");
        let middle = image.pixel(30, 30).unwrap()[3];
        assert!(
            middle > 0.1 && middle < 0.4,
            "the middle is covered a fifth of the time: {middle}"
        );
        // A single moment: a still scene, the frame's picture.
        let (one, _) = RenderScene::moving(vec![at(0.0)], 0);
        assert!(!one.is_moving());
        assert_eq!(
            render(&one, &top_camera(), &options(60, 4, 0)),
            render(&at(0.0), &top_camera(), &options(60, 4, 0))
        );
        // A changing number of triangles: that object stays put.
        let (_, still) = RenderScene::moving(vec![at(0.0), scene_of(&Document::new("empty"))], 0);
        assert_eq!(still, 1);
    }

    /// Renders `doc` with every pass through `camera`.
    fn with_passes(doc: &Document, camera: &RenderCamera, options: &RenderOptions) -> Image {
        let scene = textured_scene(doc, &Pictures::new(), *camera, &[]);
        let all: grid4d_scene::Passes = grid4d_scene::Pass::ALL.into_iter().collect();
        let mut progressive = Progressive::with_passes(options, all, &scene);
        progressive.add_samples(&scene, camera, options, u32::MAX, &RenderControl::default());
        progressive.image()
    }

    fn layer<'a>(image: &'a Image, name: &str) -> &'a crate::PassLayer {
        image
            .passes
            .iter()
            .find(|l| l.name == name)
            .unwrap_or_else(|| panic!("no {name}"))
    }

    /// A scene with every kind of light path: a glass ball, a coated cube, a
    /// glowing panel and a lit floor under a Sky, with a light in a group.
    fn every_path() -> Document {
        let mut doc = floor_scene(LightShadow::Raytraced);
        let sun = doc.scene.iter().find(|(_, n)| n.data.name == "Sun").unwrap().0;
        if let ObjectKind::Light { group, .. } = &mut doc.scene.get_mut(sun).unwrap().data.kind {
            *group = "Key".to_owned();
        }
        let mut glass = SceneMaterial::new("Glass");
        (glass.color, glass.transmission, glass.roughness) = ([0.9, 0.95, 1.0], 1.0, 0.05);
        let glass = doc.scene.materials_mut().insert(glass);
        let mut ball = Node::new(
            "Ball",
            ObjectKind::Sphere {
                radius: 50.0,
                segments: 24,
            },
        )
        .with_position(Vec3::new(100.0, 50.0, 60.0));
        ball.data.tags = vec![Tag::Material(MaterialTag::new(Some(glass)))];
        doc.scene.insert(ball);
        let mut coat = SceneMaterial::new("Coat");
        (coat.color, coat.clearcoat, coat.roughness) = ([0.8, 0.1, 0.1], 1.0, 0.6);
        let coat = doc.scene.materials_mut().insert(coat);
        let cube = doc.scene.iter().find(|(_, n)| n.data.name == "Cube").unwrap().0;
        doc.scene.get_mut(cube).unwrap().data.tags = vec![Tag::Material(MaterialTag::new(Some(coat)))];
        let mut glow = SceneMaterial::new("Glow");
        (glow.color, glow.emission) = ([0.0; 3], [2.0, 1.5, 1.0]);
        let glow = doc.scene.materials_mut().insert(glow);
        let mut panel = Node::new(
            "Panel",
            ObjectKind::Plane {
                width: 60.0,
                depth: 60.0,
                width_segments: 1,
                depth_segments: 1,
            },
        )
        .with_position(Vec3::new(0.0, 10.0, -120.0));
        panel.data.tags = vec![Tag::Material(MaterialTag::new(Some(glow)))];
        doc.scene.insert(panel);
        let sky = grid4d_scene::Sky {
            group: "Daylight".to_owned(),
            ..grid4d_scene::Sky::default()
        };
        doc.scene.insert(Node::new("Sky", ObjectKind::Sky(sky)));
        doc
    }

    #[test]
    fn the_lighting_passes_and_the_group_passes_add_up_to_the_picture() {
        let doc = every_path();
        let options = options(40, 16, 3);
        let image = with_passes(&doc, &top_camera(), &options);
        let lighting = [
            "Emission",
            "Background",
            "DiffuseDirect",
            "DiffuseIndirect",
            "SpecularDirect",
            "SpecularIndirect",
            "Transmission",
        ];
        let groups = ["Group.Default", "Group.Key", "Group.Daylight"];
        for names in [&lighting[..], &groups[..]] {
            let layers: Vec<_> = names.iter().map(|n| layer(&image, n)).collect();
            for (i, pixel) in image.pixels.iter().enumerate() {
                for (c, &beauty) in pixel.iter().take(3).enumerate() {
                    let sum: f32 = layers.iter().map(|l| l.values[i * 3 + c]).sum();
                    assert!(
                        (sum - beauty).abs() <= 1e-3 * beauty.abs().max(1.0),
                        "{names:?} pixel {i}: {sum} vs {beauty}"
                    );
                }
            }
        }
        // Each kind of light shows somewhere.
        for name in lighting.iter().chain(&groups) {
            assert!(layer(&image, name).values.iter().any(|&v| v > 1e-3), "{name} is empty");
        }
    }

    #[test]
    fn data_passes_show_the_first_surface() {
        let doc = floor_scene(LightShadow::None);
        let image = with_passes(&doc, &top_camera(), &options(60, 4, 1));
        // Pixel over the floor at (150, 0): 1000 cm below the camera.
        let px = |x: f32, z: f32| -> usize {
            let col = ((x / 300.0 + 1.0) * 30.0).floor() as usize;
            let row = ((z / 300.0 + 1.0) * 30.0).floor() as usize;
            row * 60 + col
        };
        let i = px(150.0, 0.0);
        let depth = layer(&image, "Depth").values[i];
        assert!((depth - 1000.0).abs() < 0.1, "{depth}");
        let normal = &layer(&image, "Normal").values[i * 3..i * 3 + 3];
        assert!((normal[1] - 1.0).abs() < 1e-4, "{normal:?}");
        let position = &layer(&image, "Position").values[i * 3..i * 3 + 3];
        assert!(
            position[1].abs() < 1e-3 && (position[0] - 150.0).abs() < 6.0,
            "{position:?}"
        );
        // The cube's top is 100 cm nearer.
        let top = layer(&image, "Depth").values[px(-100.0, 0.0)];
        assert!((top - 900.0).abs() < 0.1, "{top}");
        // Outside the floor nothing: the largest depth, no object.
        let outside = px(290.0, 290.0);
        assert_eq!(layer(&image, "Depth").values[outside], f32::MAX);
        assert_eq!(layer(&image, "ObjectID").values[outside], 0.0);
        // Objects get their own IDs, named by the scene.
        let (floor, cube) = (
            layer(&image, "ObjectID").values[i],
            layer(&image, "ObjectID").values[px(-100.0, 0.0)],
        );
        assert!(floor > 0.0 && cube > 0.0 && floor != cube);
        let scene = scene_of(&doc);
        let named: Vec<u32> = scene.object_ids().iter().map(|&(id, _)| id).collect();
        assert!(named.contains(&(floor as u32)) && named.contains(&(cube as u32)));
        // Alpha is the coverage.
        assert_eq!(layer(&image, "Alpha").values[i], 1.0);
        assert_eq!(layer(&image, "Alpha").values[outside], 0.0);
    }

    /// Root mean square difference of two pictures' 8-bit sRGB values, over
    /// the pixels `only` keeps.
    fn rmse(a: &Image, b: &Image, only: impl Fn(usize) -> bool) -> f64 {
        let (a, b) = (a.to_rgb8(), b.to_rgb8());
        let (mut sum, mut n) = (0.0, 0.0);
        for (i, (pa, pb)) in a.chunks(3).zip(b.chunks(3)).enumerate() {
            if only(i) {
                for (x, y) in pa.iter().zip(pb) {
                    let d = f64::from(*x) - f64::from(*y);
                    sum += d * d;
                    n += 1.0;
                }
            }
        }
        (sum / f64::max(n, 1.0)).sqrt()
    }

    /// `doc` from above at `size` pixels: at 4 samples with the denoiser's
    /// guides, that picture denoised, and a 256-sample reference.
    fn denoised(doc: &Document, pictures: &Pictures, size: u32) -> (Image, Image, Image) {
        let scene = textured_scene(doc, pictures, top_camera(), &[]);
        let options = options(size, 4, 2);
        let mut progressive = Progressive::new(&options).with_denoising();
        progressive.add_samples(&scene, &top_camera(), &options, u32::MAX, &RenderControl::default());
        let noisy = progressive.image();
        let mut clean = noisy.clone();
        assert!(crate::denoise(&mut clean, 1.0));
        let reference = render(
            &scene,
            &top_camera(),
            &RenderOptions {
                samples: 256,
                seed: 1,
                ..options
            },
        );
        (noisy, clean, reference)
    }

    /// `doc` without its Sun, under a Sky without one: soft light from
    /// everywhere, noisy where the cube hides part of it.
    fn overcast(mut doc: Document) -> Document {
        let sun = doc.scene.iter().find(|(_, n)| n.data.name == "Sun").unwrap().0;
        doc.scene.remove(sun);
        let sky = grid4d_scene::Sky {
            sun: false,
            ..grid4d_scene::Sky::default()
        };
        doc.scene.insert(Node::new("Sky", ObjectKind::Sky(sky)));
        doc
    }

    /// Pixels where the picture changes sharply to the right or below.
    fn edges(image: &Image) -> impl Fn(usize) -> bool {
        let bytes = image.to_rgb8();
        let w = usize::try_from(image.width).unwrap();
        let luma = |i: usize| {
            bytes
                .get(i * 3..i * 3 + 3)
                .map_or(0, |p| i32::from(p[0]) + i32::from(p[1]) + i32::from(p[2]))
        };
        let sharp: Vec<bool> = (0..bytes.len() / 3)
            .map(|i| (luma(i) - luma(i + 1)).abs() > 60 || (luma(i) - luma(i + w)).abs() > 60)
            .collect();
        move |i| sharp.get(i).copied().unwrap_or(false)
    }

    #[test]
    fn the_denoiser_cleans_a_checker_and_keeps_its_edges() {
        // The floor and cube under an overcast sky, a checker of 4 × 4
        // squares as the floor's colour (8 pixels each).
        let mut doc = overcast(floor_scene(LightShadow::None));
        let texture = texture(&mut doc);
        let material = doc.scene.materials_mut().insert(mapped(Channel::BaseColor, texture));
        let floor = doc.scene.iter().find(|(_, n)| n.data.name == "Floor").unwrap().0;
        doc.scene.get_mut(floor).unwrap().data.tags = vec![tag(material)];
        let texels: Vec<u8> = (0..64 * 64)
            .flat_map(|i| {
                let (x, y) = (i % 64 / 16, i / 64 / 16);
                if (x + y) % 2 == 0 {
                    [230, 230, 230, 255]
                } else {
                    [40, 40, 40, 255]
                }
            })
            .collect();
        let mut pictures = Pictures::new();
        pictures.insert(
            texture,
            Arc::new(Picture::new(64, 64, Texels::Srgb8(Arc::from(texels))).unwrap()),
        );
        let (noisy, clean, reference) = denoised(&doc, &pictures, 48);
        // Away from the squares' edges the light's noise is gone; at the
        // edges, what is left is the 4 samples' coarse anti-aliasing, which
        // the denoiser must not make worse (it divides by the surface
        // colour, so it never blurs the squares into each other).
        let edge = edges(&reference);
        let flat = |i: usize| !edge(i);
        let (before, after) = (rmse(&noisy, &reference, flat), rmse(&clean, &reference, flat));
        assert!(after < before * 0.6, "denoised {after:.2} against noisy {before:.2}");
        let (before, after) = (rmse(&noisy, &reference, &edge), rmse(&clean, &reference, &edge));
        assert!(
            after < before,
            "at the checker's edges: denoised {after:.2} against noisy {before:.2}"
        );
    }

    #[test]
    fn the_denoiser_keeps_a_cubes_edges_and_leaves_the_background() {
        let doc = overcast(floor_scene(LightShadow::None));
        let (noisy, clean, reference) = denoised(&doc, &Pictures::new(), 48);
        let (before, after) = (rmse(&noisy, &reference, |_| true), rmse(&clean, &reference, |_| true));
        assert!(after < before * 0.6, "denoised {after:.2} against noisy {before:.2}");
        let edge = edges(&reference);
        let (before, after) = (rmse(&noisy, &reference, &edge), rmse(&clean, &reference, &edge));
        assert!(
            after < before * 1.1,
            "at the cube's edges: denoised {after:.2} against noisy {before:.2}"
        );
        // Pixels that met nothing (the Sky around the floor) are as they were.
        let guides = clean.guides.as_ref().unwrap();
        for (i, &depth) in guides.depth.iter().enumerate() {
            if depth == f32::MAX {
                assert_eq!(clean.pixels[i], noisy.pixels[i]);
            }
        }
        // Strength 0 keeps the noisy picture; the noisy one is kept as a pass.
        let mut none = noisy.clone();
        assert!(crate::denoise(&mut none, 0.0));
        assert_eq!(none.pixels, noisy.pixels);
        assert_eq!(layer(&clean, "Noisy").values[..3], noisy.pixels[0][..3]);
        // Without guides there is nothing to do.
        let mut plain = render(&scene_of(&doc), &top_camera(), &options(8, 2, 1));
        assert!(!crate::denoise(&mut plain, 1.0));
    }

    #[test]
    fn denoised_passes_still_add_up_to_the_picture() {
        let doc = every_path();
        let options = options(32, 8, 3);
        let scene = textured_scene(&doc, &Pictures::new(), top_camera(), &[]);
        let all: grid4d_scene::Passes = grid4d_scene::Pass::ALL.into_iter().collect();
        let mut progressive = Progressive::with_passes(&options, all, &scene).with_denoising();
        progressive.add_samples(&scene, &top_camera(), &options, u32::MAX, &RenderControl::default());
        let mut image = progressive.image();
        // Asked for: the guides stay passes too.
        for name in ["Albedo", "Normal", "Depth"] {
            layer(&image, name);
        }
        assert!(crate::denoise(&mut image, 0.8));
        let lighting = [
            "Emission",
            "Background",
            "DiffuseDirect",
            "DiffuseIndirect",
            "SpecularDirect",
            "SpecularIndirect",
            "Transmission",
        ];
        let groups = ["Group.Default", "Group.Key", "Group.Daylight"];
        for names in [&lighting[..], &groups[..]] {
            let layers: Vec<_> = names.iter().map(|n| layer(&image, n)).collect();
            for (i, pixel) in image.pixels.iter().enumerate() {
                for (c, &beauty) in pixel.iter().take(3).enumerate() {
                    let sum: f32 = layers.iter().map(|l| l.values[i * 3 + c]).sum();
                    assert!(
                        (sum - beauty).abs() <= 1e-3 * beauty.abs().max(1.0),
                        "{names:?} pixel {i}: {sum} vs {beauty}"
                    );
                }
            }
        }
    }

    /// A box `size` wide and deep, `height` high (its top at `height`), of a
    /// white material (`subsurface` of it scattering inside, light travelling
    /// `radius` cm), lit by a sun turned `tilt` degrees from straight down
    /// (180: from straight below).
    fn slab(size: f32, height: f32, subsurface: f32, radius: f32, tilt: f32) -> Document {
        let mut doc = Document::new("slab");
        let mut white = SceneMaterial::new("White");
        (white.color, white.roughness, white.specular) = ([0.9; 3], 1.0, 0.0);
        white.subsurface = subsurface;
        white.subsurface_radius = [radius; 3];
        let white = doc.scene.materials_mut().insert(white);
        let mut node = Node::new(
            "Slab",
            ObjectKind::Cube {
                size: Vec3::new(size, height, size),
                segments: [1, 1, 1],
            },
        )
        .with_position(Vec3::new(0.0, height / 2.0, 0.0));
        node.data.tags = vec![Tag::Material(MaterialTag::new(Some(white)))];
        doc.scene.insert(node);
        let sun = ObjectKind::Light {
            color: [1.0; 3],
            intensity: 1.0,
            light_type: LightType::Infinite,
            details: LightDetails::default(),
            shadow: LightShadow::Raytraced,
            linking: grid4d_scene::Linking::default(),
            group: String::new(),
        };
        doc.scene.insert(
            Node::new("Sun", sun).with_transform(grid4d_scene::Transform::default().with_rotation_deg(Vec3::new(
                0.0,
                tilt - 90.0,
                0.0,
            ))),
        );
        doc
    }

    /// The average colour of the middle quarter of a top view of `doc`.
    fn middle(doc: &Document, samples: u32) -> Vec3 {
        let image = render(
            &textured_scene(doc, &Pictures::new(), top_camera(), &[]),
            &top_camera(),
            &options(32, samples, 8),
        );
        let mut sum = Vec3::ZERO;
        for y in 12..20 {
            for x in 12..20 {
                let p = image.pixels[y * 32 + x];
                sum += Vec3::new(p[0], p[1], p[2]);
            }
        }
        sum / 64.0
    }

    #[test]
    fn subsurface_keeps_the_light_a_white_slab_reflects() {
        // A wide, thick slab lit from above: light that goes in comes out
        // again near where it went in, so the top is about as bright as a
        // matte one of the same colour (within the albedo relation's few %).
        let matte = middle(&slab(500.0, 200.0, 0.0, 5.0, 0.0), 16);
        let inside = middle(&slab(500.0, 200.0, 1.0, 5.0, 0.0), 64);
        let ratio = inside.x / matte.x;
        assert!((0.93..=1.05).contains(&ratio), "{inside} against {matte}");
    }

    #[test]
    fn a_thin_slab_with_subsurface_glows_lit_from_behind() {
        // Lit from straight below, a 2 cm slab's top is dark when matte and
        // glows with light that went through it when it scatters inside.
        let matte = middle(&slab(500.0, 2.0, 0.0, 5.0, 180.0), 8);
        let inside = middle(&slab(500.0, 2.0, 1.0, 5.0, 180.0), 32);
        assert!(matte.max_element() < 1e-3, "{matte}");
        assert!(inside.x > 0.1, "{inside}");
    }

    #[test]
    fn passes_change_nothing_in_the_picture() {
        let doc = every_path();
        let options = options(30, 8, 2);
        let plain = render(
            &textured_scene(&doc, &Pictures::new(), top_camera(), &[]),
            &top_camera(),
            &options,
        );
        let with = with_passes(&doc, &top_camera(), &options);
        assert_eq!(plain.pixels, with.pixels);
    }
}
