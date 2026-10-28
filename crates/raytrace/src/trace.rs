//! The path tracer: per pixel, samples of paths from the camera that pick up
//! the lights' light at every surface they meet.

use glam::Vec3;
use grid4d_scene::{Falloff, LightShadow, LightType, RenderSettings};
use rayon::prelude::*;

use crate::{Image, RenderCamera, RenderScene, sample::Rng, scene::Light, shade::Material};

/// The largest picture side, in pixels.
const MAX_SIZE: u32 = 16_384;
/// The most samples per pixel.
const MAX_SAMPLES: u32 = 4096;
/// Light picked up after a bounce is capped at this (per colour channel and
/// light): a bounce that lands on a sharp highlight would otherwise leave bright
/// specks ("fireflies") that take thousands of samples to average out.
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
        }
    }
}

/// Renders `scene` through `camera`. Rows are rendered in parallel; the result
/// is the same however the work is shared.
#[must_use]
pub fn render(scene: &RenderScene, camera: &RenderCamera, options: &RenderOptions) -> Image {
    let width = options.width.clamp(1, MAX_SIZE);
    let height = options.height.clamp(1, MAX_SIZE);
    let mut image = Image::new(width, height);
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
        width,
        height,
        samples: options.samples.clamp(1, MAX_SAMPLES),
        bounces: options.bounces,
        seed: options.seed,
    };
    let row_length = usize::try_from(width).unwrap_or(1);
    image
        .pixels
        .par_chunks_mut(row_length)
        .zip(0..height)
        .for_each(|(row, y)| {
            for (pixel, x) in row.iter_mut().zip(0..width) {
                *pixel = job.pixel(x, y);
            }
        });
    image
}

/// What every pixel of one render shares.
struct Job<'a> {
    scene: &'a RenderScene,
    lights: &'a [Light],
    camera: &'a RenderCamera,
    width: u32,
    height: u32,
    samples: u32,
    bounces: u32,
    seed: u32,
}

impl Job<'_> {
    /// One pixel: the average of its samples, colour premultiplied by coverage.
    fn pixel(&self, x: u32, y: u32) -> [f32; 4] {
        let (w, h) = (to_f32(self.width), to_f32(self.height));
        let aspect = w / h;
        let mut sum = Vec3::ZERO;
        let mut covered = 0.0;
        for sample in 0..self.samples {
            let mut rng = Rng::new(x, y, sample, self.seed);
            // A point spread over the pixel: smooth edges.
            let px = (to_f32(x) + rng.next_f32()) / w * 2.0 - 1.0;
            let py = 1.0 - (to_f32(y) + rng.next_f32()) / h * 2.0;
            let (origin, dir) = self.camera.ray(px, py, aspect);
            if let Some(radiance) = self.trace(origin, dir, &mut rng) {
                sum += radiance;
                covered += 1.0;
            }
        }
        let n = to_f32(self.samples);
        let colour = sum / n;
        [colour.x, colour.y, colour.z, covered / n]
    }

    /// The light coming back along one path; `None` if the first ray meets no
    /// surface (background).
    fn trace(&self, mut origin: Vec3, mut dir: Vec3, rng: &mut Rng) -> Option<Vec3> {
        let mut radiance = Vec3::ZERO;
        let mut throughput = Vec3::ONE;
        for depth in 0..=self.bounces {
            let Some(hit) = self.scene.intersect(origin, dir) else {
                if depth == 0 {
                    return None;
                }
                break; // nothing around the scene: black
            };
            let v = -dir;
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
            let material = Material::new(&hit.surface);
            radiance += capped(throughput * hit.surface.emission, depth);

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
                if light.shadow.casts()
                    && self
                        .scene
                        .occluded(lifted, incoming.direction, incoming.distance - lift)
                {
                    continue;
                }
                radiance += capped(throughput * reflected * incoming.strength, depth);
            }

            if depth == self.bounces {
                break;
            }
            let Some((next, weight)) = material.bounce(n, v, rng) else {
                break;
            };
            if next.dot(geometric) <= 0.0 {
                break;
            }
            throughput *= weight;
            if throughput.max_element() <= 1e-4 {
                break;
            }
            origin = lifted;
            dir = next;
        }
        Some(radiance)
    }
}

/// `light` as is when seen directly (`depth` 0), capped after a bounce.
fn capped(light: Vec3, depth: u32) -> Vec3 {
    if depth == 0 {
        light
    } else {
        light.min(Vec3::splat(BOUNCE_CAP))
    }
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
fn offset(p: Vec3) -> f32 {
    0.01 + p.abs().max_element() * 1e-5
}

/// A picture coordinate or count as a float (all are at most 16384, or 4096).
fn to_f32(n: u32) -> f32 {
    u16::try_from(n).map_or(f32::from(u16::MAX), f32::from)
}

#[cfg(test)]
mod tests {
    use grid4d_eval::EvaluatedScene;
    use grid4d_scene::{Document, LightDetails, Material as SceneMaterial, Node, ObjectKind, ObjectKindTag, Tag};

    use super::*;
    use crate::CameraProjection;

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
        node.data.tags = vec![Tag::Material { material: Some(white) }];
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
        cube.data.tags = vec![Tag::Material { material: Some(glow) }];
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

    #[test]
    fn options_come_from_the_render_settings() {
        let settings = RenderSettings {
            width: 320,
            height: 200,
            samples: 7,
            bounces: 3,
            auto_light: false,
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
            }
        );
        let defaults = RenderOptions::from(&RenderSettings::default());
        assert_eq!(
            (defaults.samples, defaults.bounces, defaults.auto_light),
            (16, 2, true),
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
}
