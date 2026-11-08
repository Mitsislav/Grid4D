//! Material previews (see `docs/reference/material-channels.md`): each
//! material rendered on a ball by the path tracer, small, on a worker thread,
//! one at a time, so the interface never waits. [`MaterialPreviews::update`]
//! runs once a frame (from `commands::apply_pending`): it takes a finished
//! picture, starts the next material whose picture is out of date, and forgets
//! materials the document no longer has. A material changed while its picture
//! is being made is simply rendered again afterwards, so a drag on a slider
//! ends with a picture of its last value; so is one whose texture pictures
//! arrive or change (the texture cache's versions are part of what a picture
//! shows). Previews never change the document.

use std::{
    collections::HashMap,
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use egui::{ColorImage, Painter, Rect, TextureHandle, TextureOptions};
use glam::{Mat4, Vec3};
use grid4d_eval::EvaluatedScene;
use grid4d_raytrace::{CameraProjection, Image, Pictures, RenderCamera, RenderOptions, RenderScene, Texturing};
use grid4d_scene::{
    Channel, Document, LightDetails, LightShadow, LightType, Material, MaterialId, MaterialTag, Materials, Node,
    ObjectKind, Tag, Textures,
};

use crate::textures::TextureCache;

/// A preview's side, in pixels.
pub const PREVIEW_SIZE: u32 = 96;

/// The picture of `material` on the preview ball: a sphere seen from the
/// front, lit from the upper left with a dimmer light from the lower right,
/// inside a soft grey surrounding (so metals have something to reflect),
/// over a transparent background (linear, premultiplied). Its textures read
/// `pictures` (by the ball's own coordinates).
#[must_use]
pub fn render_preview(material: &Material, pictures: &Pictures) -> Image {
    let mut doc = Document::new("Preview");
    let id = doc.scene.materials_mut().insert(material.clone());
    let mut ball = Node::new(
        "Ball",
        ObjectKind::Sphere {
            radius: 50.0,
            segments: 32,
        },
    );
    ball.data.tags = vec![Tag::Material(MaterialTag::new(Some(id)))];
    doc.scene.insert(ball);
    for (name, position, intensity) in [
        ("Key", Vec3::new(-160.0, 170.0, 220.0), 1.0),
        ("Fill", Vec3::new(220.0, -80.0, 160.0), 0.3),
    ] {
        let light = ObjectKind::Light {
            color: [1.0; 3],
            intensity,
            light_type: LightType::Omni,
            details: LightDetails::default(),
            shadow: LightShadow::None,
        };
        doc.scene.insert(Node::new(name, light).with_position(position));
    }
    let camera = RenderCamera {
        world: Mat4::from_translation(Vec3::new(0.0, 0.0, 220.0)),
        projection: CameraProjection::Perspective { fov_y: 0.5 },
    };
    let options = RenderOptions {
        width: PREVIEW_SIZE,
        height: PREVIEW_SIZE,
        samples: 16,
        bounces: 2,
        seed: 0,
        auto_light: false,
        clamp_indirect: true,
        emissive_lights: true,
    };
    let render = |doc: &Document| {
        let mut evaluated = EvaluatedScene::for_render();
        evaluated.update(&doc.scene);
        let texturing = Texturing {
            materials: doc.scene.materials(),
            pictures,
            camera,
            aspect: 1.0,
            cameras: &[],
        };
        grid4d_raytrace::render(&RenderScene::textured(&evaluated, &texturing), &camera, &options)
    };
    // Where the ball is, without the surrounding (cut-outs show through).
    let mask = render(&doc);
    let mut surrounding = Material::new("Surrounding");
    (surrounding.color, surrounding.emission) = ([0.0; 3], [0.22; 3]);
    let surrounding = doc.scene.materials_mut().insert(surrounding);
    let mut sky = Node::new(
        "Surrounding",
        ObjectKind::Sphere {
            radius: 2000.0,
            segments: 16,
        },
    );
    sky.data.tags = vec![Tag::Material(MaterialTag::new(Some(surrounding)))];
    doc.scene.insert(sky);
    let mut image = render(&doc);
    for (pixel, covered) in image.pixels.iter_mut().zip(&mask.pixels) {
        let alpha = covered[3];
        *pixel = [pixel[0] * alpha, pixel[1] * alpha, pixel[2] * alpha, alpha];
    }
    image
}

/// What a preview shows: a material, and the versions of its channels'
/// texture pictures (`None`: a channel without texture, or one the cache does
/// not know).
#[derive(Clone, Debug, PartialEq)]
struct Shown {
    material: Material,
    pictures: [Option<u64>; 10],
}

impl Shown {
    fn of(material: &Material, cache: &TextureCache) -> Self {
        Self {
            material: material.clone(),
            pictures: Channel::ALL.map(|c| material.map(c).and_then(|slot| cache.version(slot.texture))),
        }
    }

    /// Whether this is what `material` looks like now (no allocation).
    fn is(&self, material: &Material, cache: &TextureCache) -> bool {
        self.material == *material
            && Channel::ALL
                .iter()
                .zip(&self.pictures)
                .all(|(&c, &version)| material.map(c).and_then(|slot| cache.version(slot.texture)) == version)
    }
}

#[derive(Default)]
struct Entry {
    /// What its picture shows (`None`: no picture yet).
    rendered: Option<Shown>,
    picture: Option<ColorImage>,
    /// Bumped with every new picture.
    version: u64,
    /// The picture on the GPU, and the version it holds.
    handle: Option<(u64, TextureHandle)>,
    used: bool,
}

impl std::fmt::Debug for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Entry")
            .field("version", &self.version)
            .field("pictured", &self.picture.is_some())
            .finish_non_exhaustive()
    }
}

struct Job {
    id: MaterialId,
    shown: Shown,
    pictures: Pictures,
}

struct Done {
    id: MaterialId,
    shown: Shown,
    picture: ColorImage,
}

#[derive(Debug)]
struct Worker {
    jobs: Sender<Job>,
    done: Receiver<Done>,
}

/// The preview pictures of the document's materials (editor state, never
/// saved).
#[derive(Debug, Default)]
pub struct MaterialPreviews {
    entries: HashMap<MaterialId, Entry>,
    worker: Option<Worker>,
    /// The material being rendered.
    busy: Option<MaterialId>,
}

impl MaterialPreviews {
    /// Takes a finished picture, starts the next one and forgets materials
    /// that are gone. `textures` is the document's table, `cache` its
    /// pictures. Returns `true` while a picture is being made (keep
    /// repainting).
    pub fn update(&mut self, materials: &Materials, textures: &Textures, cache: &TextureCache) -> bool {
        self.receive();
        for entry in self.entries.values_mut() {
            entry.used = false;
        }
        let mut next = None;
        for (id, material) in materials.iter() {
            let entry = self.entries.entry(id).or_default();
            entry.used = true;
            if next.is_none() && !entry.rendered.as_ref().is_some_and(|r| r.is(material, cache)) {
                next = Some((id, material));
            }
        }
        self.entries.retain(|_, entry| entry.used);
        if self.busy.is_none()
            && let Some((id, material)) = next
        {
            let job = Job {
                id,
                shown: Shown::of(material, cache),
                pictures: cache.pictures(textures),
            };
            self.start(job);
        }
        self.busy.is_some()
    }

    /// Whether a picture is being made or one of `materials` has none of its
    /// current state yet (for tests and pictures that wait for them).
    #[must_use]
    pub fn pending(&self, materials: &Materials, cache: &TextureCache) -> bool {
        self.busy.is_some()
            || materials.iter().any(|(id, m)| {
                self.entries
                    .get(&id)
                    .is_none_or(|e| !e.rendered.as_ref().is_some_and(|r| r.is(m, cache)))
            })
    }

    /// The preview of `id` for drawing, uploaded on first use after each new
    /// picture; `None` before the first picture.
    pub fn texture(&mut self, ctx: &egui::Context, id: MaterialId) -> Option<egui::TextureId> {
        let entry = self.entries.get_mut(&id)?;
        let picture = entry.picture.as_ref()?;
        if entry
            .handle
            .as_ref()
            .is_none_or(|(version, _)| *version != entry.version)
        {
            let handle = ctx.load_texture(
                format!("grid4d.material_preview.{id:?}"),
                picture.clone(),
                TextureOptions::LINEAR,
            );
            entry.handle = Some((entry.version, handle));
        }
        entry.handle.as_ref().map(|(_, handle)| handle.id())
    }

    /// Draws `id`'s preview in `rect`, or a painted ball of `material`'s
    /// colour until there is one.
    pub fn paint(&mut self, painter: &Painter, rect: Rect, id: MaterialId, material: &Material) {
        match self.texture(painter.ctx(), id) {
            Some(texture) => {
                let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                painter.image(texture, rect, uv, egui::Color32::WHITE);
            }
            None => crate::icons::paint_material_ball(painter, rect, material.color),
        }
    }

    fn start(&mut self, job: Job) {
        self.busy = Some(job.id);
        if self.worker.is_none() {
            self.worker = spawn();
        }
        let job = match &self.worker {
            Some(worker) => match worker.jobs.send(job) {
                Ok(()) => return,
                Err(mpsc::SendError(job)) => {
                    self.worker = None;
                    job
                }
            },
            None => job,
        };
        // No worker thread: render here (slower, but never stuck).
        let done = make(job);
        self.finish(done);
    }

    fn receive(&mut self) {
        let Some(worker) = &self.worker else { return };
        let finished: Vec<Done> = worker.done.try_iter().collect();
        for done in finished {
            self.finish(done);
        }
    }

    fn finish(&mut self, done: Done) {
        if self.busy == Some(done.id) {
            self.busy = None;
        }
        let Some(entry) = self.entries.get_mut(&done.id) else {
            return;
        };
        entry.rendered = Some(done.shown);
        entry.picture = Some(done.picture);
        entry.version += 1;
    }

    /// Waits (up to twenty seconds) until every picture is up to date: for
    /// tests.
    #[cfg(test)]
    pub(crate) fn wait(&mut self, materials: &Materials, textures: &Textures, cache: &TextureCache) {
        let start = std::time::Instant::now();
        while self.update(materials, textures, cache) && start.elapsed() < std::time::Duration::from_secs(20) {
            thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    /// Whether `id` has a picture of the material as it is now: for tests.
    #[cfg(test)]
    pub(crate) fn is_current(&self, id: MaterialId, material: &Material, cache: &TextureCache) -> bool {
        self.entries
            .get(&id)
            .is_some_and(|e| e.rendered.as_ref().is_some_and(|r| r.is(material, cache)))
    }
}

/// Renders a job's picture.
fn make(job: Job) -> Done {
    let image = render_preview(&job.shown.material, &job.pictures);
    let size = [PREVIEW_SIZE as usize; 2];
    let picture = ColorImage::from_rgba_unmultiplied(size, &image.to_rgba8());
    Done {
        id: job.id,
        shown: job.shown,
        picture,
    }
}

/// The worker thread: renders jobs one after another until the previews are
/// dropped.
fn spawn() -> Option<Worker> {
    let (jobs, inbox) = mpsc::channel::<Job>();
    let (outbox, done) = mpsc::channel::<Done>();
    thread::Builder::new()
        .name("grid4d-previews".to_owned())
        .spawn(move || {
            for job in inbox {
                if outbox.send(make(job)).is_err() {
                    break;
                }
            }
        })
        .ok()?;
    Some(Worker { jobs, done })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test code")]

    use super::*;

    #[test]
    fn a_preview_shows_the_material_on_a_ball() {
        let mut red = Material::new("Red");
        red.color = [0.8, 0.05, 0.05];
        red.roughness = 0.9;
        let image = render_preview(&red, &Pictures::new());
        assert_eq!((image.width, image.height), (PREVIEW_SIZE, PREVIEW_SIZE));
        let centre = image.pixel(PREVIEW_SIZE / 2, PREVIEW_SIZE / 2).unwrap();
        assert_eq!(centre[3], 1.0, "the ball covers the centre");
        assert!(centre[0] > centre[1] * 4.0 && centre[0] > 0.1, "red: {centre:?}");
        assert_eq!(image.pixel(0, 0).unwrap()[3], 0.0, "a transparent corner");
        // Lit from the upper left.
        let (near, far) = (PREVIEW_SIZE * 11 / 32, PREVIEW_SIZE * 21 / 32);
        let (upper_left, lower_right) = (image.pixel(near, near).unwrap(), image.pixel(far, far).unwrap());
        assert!(upper_left[0] > lower_right[0], "{upper_left:?} {lower_right:?}");
        // Glowing materials glow.
        let mut glow = Material::new("Glow");
        (glow.color, glow.emission, glow.emission_strength) = ([0.0; 3], [0.0, 0.0, 1.0], 2.0);
        let blue = render_preview(&glow, &Pictures::new()).pixel(far, far).unwrap();
        assert!(blue[2] > 0.9 && blue[0] < 0.05, "{blue:?}");
    }

    #[test]
    fn previews_follow_the_materials_one_at_a_time() {
        let mut materials = Materials::default();
        let a = materials.insert(Material::new("A"));
        let b = materials.insert(Material::new("B"));
        let (textures, cache) = (Textures::default(), TextureCache::default());
        let mut previews = MaterialPreviews::default();
        assert!(
            previews.update(&materials, &textures, &cache),
            "the first picture is on its way"
        );
        assert!(previews.busy.is_some());
        previews.wait(&materials, &textures, &cache);
        assert!(previews.is_current(a, materials.get(a).unwrap(), &cache));
        assert!(previews.is_current(b, materials.get(b).unwrap(), &cache));
        assert!(!previews.pending(&materials, &cache));
        let version = previews.entries[&a].version;

        // An edit makes a new picture; the old one stays until it comes.
        materials.get_mut(a).unwrap().roughness = 0.1;
        assert!(previews.update(&materials, &textures, &cache));
        assert!(previews.entries[&a].picture.is_some());
        previews.wait(&materials, &textures, &cache);
        assert!(previews.is_current(a, materials.get(a).unwrap(), &cache));
        assert_eq!(previews.entries[&a].version, version + 1);
        assert!(!previews.update(&materials, &textures, &cache), "nothing to do");

        // A removed material is forgotten.
        materials.remove(b);
        previews.update(&materials, &textures, &cache);
        assert!(!previews.entries.contains_key(&b));
    }

    #[test]
    fn previews_show_texture_pictures_once_they_arrive() {
        use grid4d_scene::{ColorSpace, Texture, TextureSlot, TextureSource};
        // A red picture on a white ball's Base Color.
        let red = grid4d_io::png_bytes(2, 2, grid4d_io::Channels::Rgba, &[255, 0, 0, 255].repeat(4)).unwrap();
        let mut textures = Textures::default();
        let texture = textures.insert(Texture {
            name: "Red".to_owned(),
            source: TextureSource::Embedded {
                file_name: "red.png".to_owned(),
                bytes: red.into(),
            },
            color_space: ColorSpace::Srgb,
        });
        let mut materials = Materials::default();
        let mut white = Material::new("White");
        (white.color, white.roughness) = ([1.0; 3], 1.0);
        *white.map_mut(Channel::BaseColor) = Some(TextureSlot::new(texture));
        let id = materials.insert(white);
        let mut cache = TextureCache::default();
        let mut previews = MaterialPreviews::default();
        // The picture still loading: the ball in the channel's value (white),
        // then again once it is there (red).
        previews.update(&materials, &textures, &cache);
        cache.wait(&textures);
        assert!(previews.pending(&materials, &cache), "the picture's arrival is news");
        previews.wait(&materials, &textures, &cache);
        assert!(previews.is_current(id, materials.get(id).unwrap(), &cache));
        let picture = previews.entries[&id].picture.as_ref().unwrap();
        let centre = picture.pixels[picture.width() * picture.height() / 2 + picture.width() / 2];
        assert!(centre.r() > 150 && centre.g() < 70 && centre.b() < 70, "{centre:?}");
    }
}
