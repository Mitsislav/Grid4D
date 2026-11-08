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
    sample::Rng,
    scene::{Hit, Light},
    shade::Material,
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

/// How a picture is rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

    /// Work done so far, in rows × samples (a whole render: height × samples).
    #[must_use]
    pub fn progress(&self) -> u64 {
        self.progress.load(Ordering::Relaxed)
    }
}

/// A render built up in passes, for showing it while it sharpens: each pass
/// adds samples to every pixel. Samples are added in the same order as one
/// [`render`] adds them, so the finished picture is exactly the same.
#[derive(Clone, Debug)]
pub struct Progressive {
    width: u32,
    height: u32,
    /// Per pixel: the sum of its samples' colours, and how many met a surface.
    sums: Vec<[f32; 4]>,
    samples: u32,
}

impl Progressive {
    /// No samples yet, at the size of `options`.
    #[must_use]
    pub fn new(options: &RenderOptions) -> Self {
        let width = options.width.clamp(1, MAX_SIZE);
        let height = options.height.clamp(1, MAX_SIZE);
        let count = usize::try_from(u64::from(width) * u64::from(height)).unwrap_or(0);
        Self {
            width,
            height,
            sums: vec![[0.0; 4]; count],
            samples: 0,
        }
    }

    /// Samples per pixel so far.
    #[must_use]
    pub fn samples(&self) -> u32 {
        self.samples
    }

    /// Whether every sample of `options` is in.
    #[must_use]
    pub fn is_complete(&self, options: &RenderOptions) -> bool {
        self.samples >= options.samples.clamp(1, MAX_SAMPLES)
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
        let first = self.samples.min(total);
        let last = first.saturating_add(count).min(total);
        if first == last {
            return !control.is_cancelled();
        }
        let automatic = [automatic_light(camera)];
        let lights = if scene.lights().is_empty() && options.auto_light {
            &automatic[..]
        } else {
            scene.lights()
        };
        let job = Job {
            scene,
            lights,
            camera,
            width: self.width,
            height: self.height,
            bounces: options.bounces,
            seed: options.seed,
            clamp: options.clamp_indirect,
            emitters: options.emissive_lights && scene.has_emitters(),
        };
        let row_length = usize::try_from(self.width).unwrap_or(1);
        let pass = u64::from(last - first);
        self.sums
            .par_chunks_mut(row_length)
            .zip(0..self.height)
            .for_each(|(row, y)| {
                if control.is_cancelled() {
                    return;
                }
                for (sum, x) in row.iter_mut().zip(0..self.width) {
                    job.add_samples(x, y, first..last, sum);
                }
                control.progress.fetch_add(pass, Ordering::Relaxed);
            });
        if control.is_cancelled() {
            return false;
        }
        self.samples = last;
        true
    }

    /// The picture so far: each pixel the average of its samples, colour
    /// premultiplied by coverage. Transparent before the first pass.
    #[must_use]
    pub fn image(&self) -> Image {
        let mut image = Image::new(self.width, self.height);
        if self.samples == 0 {
            return image;
        }
        let n = to_f32(self.samples);
        for (pixel, &[r, g, b, covered]) in image.pixels.iter_mut().zip(&self.sums) {
            let colour = Vec3::new(r, g, b) / n;
            *pixel = [colour.x, colour.y, colour.z, covered / n];
        }
        image
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
    /// Emitting surfaces are sampled as lights (and there are some).
    emitters: bool,
}

impl Job<'_> {
    /// Adds samples `samples` of pixel (`x`, `y`) to `sum`: their colours, and
    /// one per sample that met a surface.
    fn add_samples(&self, x: u32, y: u32, samples: std::ops::Range<u32>, sum: &mut [f32; 4]) {
        let (w, h) = (to_f32(self.width), to_f32(self.height));
        let aspect = w / h;
        for sample in samples {
            let mut rng = Rng::new(x, y, sample, self.seed);
            // A point spread over the pixel: smooth edges.
            let px = (to_f32(x) + rng.next_f32()) / w * 2.0 - 1.0;
            let py = 1.0 - (to_f32(y) + rng.next_f32()) / h * 2.0;
            let (origin, dir) = self.camera.ray(px, py, aspect);
            if let Some(radiance) = self.trace(origin, dir, &mut rng) {
                sum[0] += radiance.x;
                sum[1] += radiance.y;
                sum[2] += radiance.z;
                sum[3] += 1.0;
            }
        }
    }

    /// The light coming back along one path; `None` if the first ray meets no
    /// surface (background).
    fn trace(&self, mut origin: Vec3, mut dir: Vec3, rng: &mut Rng) -> Option<Vec3> {
        let mut radiance = Vec3::ZERO;
        let mut throughput = Vec3::ONE;
        // The density with which the last bounce chose `dir` (reflected
        // paths only): an emitter it meets is weighed against having been
        // sampled directly from there. `None` for the camera ray and for
        // refracted paths, which direct sampling cannot reproduce.
        let mut last_pdf: Option<f32> = None;
        for depth in 0..=self.bounces {
            let Some(hit) = self.scene.intersect(origin, dir) else {
                if depth == 0 {
                    return None;
                }
                break; // nothing around the scene: black
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
            if hit.surface.emission.max_element() > 0.0 {
                let weight = match last_pdf {
                    Some(bsdf) if self.emitters => {
                        let distance2 = (hit.position - origin).length_squared();
                        let light = self.emitter_density(hit.triangle, geometric.dot(v), distance2);
                        power_heuristic(bsdf, light)
                    }
                    _ => 1.0,
                };
                radiance += self.capped(throughput * hit.surface.emission * weight, depth);
            }

            let lift = offset(hit.position);
            let lifted = hit.position + geometric * lift;
            for light in self.lights {
                let Some(incoming) = light.arriving(hit.position, rng) else {
                    continue;
                };
                let reflected = material.reflect(n, v, incoming.direction);
                if reflected.max_element() <= 0.0 || incoming.direction.dot(geometric) <= 0.0 {
                    continue;
                }
                let passed = if light.shadow.casts() {
                    self.scene
                        .transmittance(lifted, incoming.direction, incoming.distance - lift, true)
                } else {
                    Vec3::ONE
                };
                if passed.max_element() <= 0.0 {
                    continue;
                }
                radiance += self.capped(throughput * reflected * incoming.strength * passed, depth);
            }
            if self.emitters {
                let direct = self.sample_emitters(&hit, &material, n, v, geometric, lifted, entering, rng);
                radiance += self.capped(throughput * direct, depth);
            }

            if depth == self.bounces {
                break;
            }
            let Some(bounce) = material.bounce(n, v, entering, rng) else {
                break;
            };
            // Reflected paths stay on the side they came from, refracted ones
            // cross the surface.
            let side = bounce.direction.dot(geometric);
            if (bounce.through && side >= 0.0) || (!bounce.through && side <= 0.0) {
                break;
            }
            throughput *= bounce.weight;
            if throughput.max_element() <= 1e-4 {
                break;
            }
            last_pdf = (self.emitters && !bounce.through).then(|| material.pdf(n, v, bounce.direction, entering));
            origin = if bounce.through {
                hit.position - geometric * lift
            } else {
                lifted
            };
            dir = bounce.direction;
        }
        Some(radiance)
    }

    /// One sample of the light that emitting surfaces send to `hit` and that
    /// it reflects towards `v`, weighed against finding the same light by a
    /// bounce (the power heuristic).
    #[expect(clippy::too_many_arguments, reason = "the hit's frame, as the loop has it")]
    fn sample_emitters(
        &self,
        hit: &Hit,
        material: &Material,
        n: Vec3,
        v: Vec3,
        geometric: Vec3,
        lifted: Vec3,
        entering: bool,
        rng: &mut Rng,
    ) -> Vec3 {
        let (u, s, t) = (rng.next_f32(), rng.next_f32(), rng.next_f32());
        let Some(sample) = self.scene.sample_emitter(u, s, t) else {
            return Vec3::ZERO;
        };
        if sample.triangle == hit.triangle {
            return Vec3::ZERO;
        }
        let to_light = sample.position - hit.position;
        let distance2 = to_light.length_squared();
        if distance2 <= f32::MIN_POSITIVE {
            return Vec3::ZERO;
        }
        let distance = distance2.sqrt();
        let l = to_light / distance;
        let cos_light = sample.normal.dot(l).abs();
        if cos_light <= 1e-6 || l.dot(geometric) <= 0.0 {
            return Vec3::ZERO;
        }
        let reflected = material.reflect(n, v, l);
        if reflected.max_element() <= 0.0 {
            return Vec3::ZERO;
        }
        let emitted = self.scene.emission_at(sample.triangle, sample.position);
        if emitted.max_element() <= 0.0 {
            return Vec3::ZERO;
        }
        // Up to the emitter, not into it: from the lifted point to one lifted
        // off the emitter on this side (a ray from the lifted point along `l`
        // would meet the emitter's plane before `distance` at grazing angles).
        // Glass blocks: paths refracted through it find the light themselves.
        let facing = if sample.normal.dot(l) > 0.0 {
            -sample.normal
        } else {
            sample.normal
        };
        let end = sample.position + facing * offset(sample.position);
        let shadow = end - lifted;
        let reach = shadow.length();
        if reach <= f32::MIN_POSITIVE {
            return Vec3::ZERO;
        }
        let passed = self.scene.transmittance(lifted, shadow / reach, reach, false);
        if passed.max_element() <= 0.0 {
            return Vec3::ZERO;
        }
        // The chance of this direction: per area, turned into per solid angle.
        let light_pdf = sample.density * distance2 / cos_light;
        let weight = power_heuristic(light_pdf, material.pdf(n, v, l, entering));
        // `reflect` is the reflectance × π × n·l.
        reflected * emitted * passed * (weight / (PI * light_pdf))
    }

    /// The density per solid angle with which direct sampling picks the point
    /// of `triangle` seen at `distance2` (squared) and at cosine `cos` to its
    /// normal.
    fn emitter_density(&self, triangle: usize, cos: f32, distance2: f32) -> f32 {
        let area = self.scene.triangle_area(triangle);
        let cos = cos.abs();
        if area <= f32::MIN_POSITIVE || cos <= 1e-6 {
            return 0.0;
        }
        self.scene.emitter_chance(triangle) / area * distance2 / cos
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

    #[test]
    fn the_indirect_clamp_caps_only_light_after_a_bounce() {
        let doc = Document::new("t");
        let scene = scene_of(&doc);
        let camera = top_camera();
        let job = |clamp| Job {
            scene: &scene,
            lights: &[],
            camera: &camera,
            width: 1,
            height: 1,
            bounces: 1,
            seed: 0,
            clamp,
            emitters: false,
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
        assert_eq!(control.progress(), 20 * 7, "rows × samples");

        let mut first = Progressive::new(&options);
        first.add_samples(&scene, &camera, &options, 1, &control);
        assert_ne!(first.image(), whole, "one sample is noisier");
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
}
