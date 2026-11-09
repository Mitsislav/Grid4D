//! Final renders in the background (see `docs/reference/render-picture-viewer.md`
//! and `render-animation.md`): a render runs on a worker thread over a copy of
//! the scene, frame after frame of its range, and sends a picture after every
//! pass, so the Picture Viewer shows it sharpening while the editor stays
//! usable; each finished frame is handed back to be saved. [`Renders`] (editor state) keeps the running job and the
//! session's pictures; [`Renders::poll`] takes the worker's news once a frame.
//! Renders show the document's materials with the texture pictures decoded
//! when they start (`TextureCache::pictures`; one still loading shows its
//! channel's value).

use std::{
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};

use crate::i18n::{key, trf};
use grid4d_eval::EvaluatedScene;
use grid4d_raytrace::{
    CameraProjection, Image, Pictures, Progressive, RenderCamera, RenderControl, RenderOptions, RenderScene, Texturing,
    ThinLens,
};
use grid4d_render::OrbitCamera;
use grid4d_scene::{
    Denoise, Document, ImageFormat, Lens, NodeId, ObjectKind, ObjectKindTag, Pass, Passes, RenderSettings, Scene,
};
use grid4d_sim::Replay;

/// The history's pictures may take this much memory together; older ones go first.
pub const HISTORY_BYTES: usize = 512 << 20;
/// Samples per pixel a pass adds at most: later passes take longer, so the
/// picture is still refreshed every so often.
const MAX_PASS: u32 = 32;

/// Where a render is, or how it ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderState {
    Rendering,
    Finished,
    Stopped,
}

/// Where a render's pictures go, decided when it starts: the Render Settings'
/// file in their format, numbered per frame for an animation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    settings: RenderSettings,
    document: Option<PathBuf>,
    numbered: bool,
}

impl Output {
    /// The file of `frame`'s picture.
    #[must_use]
    pub fn file(&self, frame: i32) -> Option<PathBuf> {
        self.settings
            .output_file(self.document.as_deref(), self.numbered.then_some(frame))
    }

    #[must_use]
    pub fn format(&self) -> ImageFormat {
        self.settings.format
    }

    /// Saved with transparency (Alpha Channel).
    #[must_use]
    pub fn alpha(&self) -> bool {
        self.settings.alpha
    }

    /// How passes are saved with PNG.
    #[must_use]
    pub fn png_passes(&self) -> grid4d_scene::PngPasses {
        self.settings.png_passes
    }
}

/// Where a render's pictures go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveTarget {
    File(Output),
    /// Not saved, and why.
    Nowhere(&'static str),
}

/// A frame the worker has finished, in linear light, to be saved.
pub struct FinishedFrame {
    /// The render it belongs to.
    pub number: u32,
    pub frame: i32,
    /// The picture and its passes.
    pub image: Image,
    /// What the Object ID and Material ID passes' numbers name: `id = name`
    /// lines (empty without those passes).
    pub objects: String,
    pub materials: String,
}

/// One render of the session: its pixels so far and how it was made.
#[derive(Clone, Debug)]
pub struct Picture {
    /// Counts the session's renders from 1.
    pub number: u32,
    pub width: u32,
    pub height: u32,
    /// sRGB with straight alpha, four bytes per pixel (empty before the first pass).
    pub rgba: Vec<u8>,
    /// sRGB over black, three bytes per pixel (empty before the first pass).
    pub rgb: Vec<u8>,
    /// Shown and saved with transparency (Render Settings ▸ Alpha Channel).
    pub alpha: bool,
    /// Samples per pixel so far, and asked for.
    pub samples: u32,
    pub total_samples: u32,
    pub elapsed: Duration,
    pub state: RenderState,
    /// The frame shown (being rendered, or the last one).
    pub frame: i32,
    /// Frames in the render, finished so far, and saved so far.
    pub frames: u32,
    pub frames_done: u32,
    pub saved: u32,
    pub target: SaveTarget,
    /// What happened to the file: where it was saved, or why not. Empty while rendering.
    pub note: String,
    /// Changes with every new pass (the viewer re-uploads its texture).
    pub version: u64,
    /// The render passes of the last finished frame.
    pub passes: Vec<grid4d_raytrace::PassLayer>,
    /// The pass shown instead of the picture (an index into `passes`).
    pub pass: Option<usize>,
    /// With adaptive sampling: the samples a pixel got on average, and the
    /// share of pixels (0…1) that got every sample.
    pub sampling: Option<(f32, f32)>,
}

impl Picture {
    /// The pixels as they are shown and saved: with transparency or without.
    #[must_use]
    pub fn pixels(&self) -> (&[u8], grid4d_io::Channels) {
        if self.alpha {
            (&self.rgba, grid4d_io::Channels::Rgba)
        } else {
            (&self.rgb, grid4d_io::Channels::Rgb)
        }
    }

    /// The pixels shown: the picture's ([`Self::pixels`]), or the shown
    /// pass mapped to 8-bit RGB.
    #[must_use]
    pub fn shown_pixels(&self) -> (std::borrow::Cow<'_, [u8]>, grid4d_io::Channels) {
        match self.pass.and_then(|i| self.passes.get(i)) {
            Some(layer) => (std::borrow::Cow::Owned(layer.to_rgb8()), grid4d_io::Channels::Rgb),
            None => {
                let (pixels, channels) = self.pixels();
                (std::borrow::Cow::Borrowed(pixels), channels)
            }
        }
    }

    /// Whether there is anything to show or save yet.
    #[must_use]
    pub fn has_pixels(&self) -> bool {
        !self.rgba.is_empty()
    }

    fn bytes(&self) -> usize {
        self.rgba.len() + self.rgb.len() + self.passes.iter().map(|p| p.values.len() * 4).sum::<usize>()
    }
}

/// What the worker tells the editor.
enum Update {
    /// A pass of `frame` is done: the picture so far.
    Pass {
        rgba: Vec<u8>,
        rgb: Vec<u8>,
        samples: u32,
        frame: i32,
        sampling: Option<(f32, f32)>,
    },
    /// Every sample of `frame` is in.
    Frame {
        frame: i32,
        image: Image,
        objects: String,
        materials: String,
    },
    /// The render ended (stopped: cancelled before the last pass).
    Done { stopped: bool },
}

/// What a render looks through.
#[derive(Clone, Copy, Debug)]
enum Eye {
    /// The view's camera as it was when the render started.
    Fixed(RenderCamera),
    /// A camera object, wherever it is at each frame (animation, Target tags);
    /// the view's camera if it is gone.
    Object(NodeId, RenderCamera),
}

impl Eye {
    /// The camera at `frame` as `evaluated` (evaluated at it) places it,
    /// with its lens when it has depth of field.
    fn at(self, scene: &Scene, evaluated: &EvaluatedScene, frame: f64) -> RenderCamera {
        let (id, fallback) = match self {
            Self::Fixed(camera) => return camera,
            Self::Object(id, fallback) => (id, fallback),
        };
        let Some(node) = scene.get(id) else {
            return fallback;
        };
        let world_of = |id: NodeId| {
            evaluated
                .nodes()
                .iter()
                .find(|n| n.id == id)
                .map_or_else(|| scene.world_matrix_at(id, frame), |n| n.world)
        };
        let world = world_of(id);
        let ObjectKind::Camera { fov_y_deg, lens } = *node.data.kind_at(frame) else {
            return RenderCamera { world, ..fallback };
        };
        let fov_y = fov_y_deg.to_radians();
        let thin = lens.depth_of_field.then(|| {
            // The focus object's distance along the camera's axis.
            let forward = -world.z_axis.truncate().normalize_or(glam::Vec3::Z);
            let focus = lens
                .focus_object
                .filter(|&target| scene.get(target).is_some())
                .map_or(lens.focus_distance, |target| {
                    (world_of(target).w_axis.truncate() - world.w_axis.truncate()).dot(forward)
                });
            ThinLens {
                radius: lens.aperture_radius(fov_y),
                focus_distance: focus.max(Lens::MIN_FOCUS_DISTANCE),
                blades: lens.blade_count(),
                rotation: lens.blade_rotation,
            }
        });
        RenderCamera {
            world,
            projection: CameraProjection::Perspective { fov_y },
            lens: thin,
            end: None,
        }
    }

    /// The shutter that opens for the frame: a camera object's, else the
    /// usual half frame, centred.
    fn shutter(self, scene: &Scene, frame: f64) -> Lens {
        match self {
            Self::Object(id, _) => match scene.get(id).map(|n| n.data.kind_at(frame).into_owned()) {
                Some(ObjectKind::Camera { lens, .. }) => lens,
                _ => Lens::default(),
            },
            Self::Fixed(_) => Lens::default(),
        }
    }
}

/// The moments of `lens`'s shutter interval around `frame` that motion blur
/// with `segments` evaluates, in order, and the one nearest the frame; just
/// the frame when the shutter does not open.
#[must_use]
pub fn shutter_moments(lens: &Lens, frame: f64, segments: u32) -> (Vec<f64>, usize) {
    let (start, end) = lens.shutter_interval(frame);
    if end <= start {
        return (vec![frame], 0);
    }
    let n = segments.clamp(RenderSettings::MIN_SEGMENTS, RenderSettings::MAX_SEGMENTS);
    let moments: Vec<f64> = (0..n)
        .map(|k| start + (end - start) * f64::from(k) / f64::from(n - 1))
        .collect();
    let base = moments
        .iter()
        .enumerate()
        .min_by(|a, b| (a.1 - frame).abs().total_cmp(&(b.1 - frame).abs()))
        .map_or(0, |(i, _)| i);
    (moments, base)
}

/// The render that runs.
struct Job {
    number: u32,
    control: Arc<RenderControl>,
    updates: Receiver<Update>,
    started: Instant,
    /// Work of the whole render, in rows × samples.
    total: u64,
}

impl Drop for Job {
    /// A dropped job (the editor closes, a new render replaces it) stops its worker.
    fn drop(&mut self) {
        self.control.cancel();
    }
}

impl Job {
    /// Starts a worker rendering `scene` at each of `frames`; `None` if no
    /// thread could start.
    fn spawn(number: u32, scene: Scene, pictures: Pictures, frames: Vec<i32>, eye: Eye, look: Look) -> Option<Self> {
        let Look {
            options,
            replay,
            segments,
            passes,
            denoising,
        } = look;
        let control = Arc::new(RenderControl::default());
        let (sender, updates) = mpsc::channel();
        let worker_control = Arc::clone(&control);
        let count = u64::try_from(frames.len()).unwrap_or(1).max(1);
        let spawned = thread::Builder::new()
            .name(format!("grid4d render {number}"))
            .spawn(move || {
                let input = Input {
                    scene: &scene,
                    pictures: &pictures,
                    frames: &frames,
                    eye,
                };
                work(
                    &input,
                    &options,
                    (segments, passes, denoising),
                    replay,
                    &worker_control,
                    &sender,
                );
            });
        if let Err(err) = spawned {
            log::error!("could not start the render thread: {err}");
            return None;
        }
        // Progress counts pixels × samples (adaptive sampling credits a
        // pixel's remaining samples when it stops).
        let pixels = options.pixel_count();
        let samples = options.samples.clamp(1, RenderSettings::MAX_SAMPLES);
        Some(Self {
            number,
            control,
            updates,
            started: Instant::now(),
            total: pixels * u64::from(samples) * count,
        })
    }

    /// Takes the worker's news into `picture`, adding finished frames to
    /// `finished`. Returns whether the picture changed, and whether the render
    /// ended (`Some(stopped)`).
    fn drain(&self, picture: &mut Picture, finished: &mut Vec<FinishedFrame>) -> (bool, Option<bool>) {
        let mut changed = false;
        let mut ended = None;
        loop {
            let update = match self.updates.try_recv() {
                Ok(update) => update,
                Err(mpsc::TryRecvError::Empty) => break,
                // The worker is gone without saying so (it panicked).
                Err(mpsc::TryRecvError::Disconnected) => {
                    ended = Some(true);
                    break;
                }
            };
            match update {
                Update::Pass {
                    rgba,
                    rgb,
                    samples,
                    frame,
                    sampling,
                } => {
                    (picture.rgba, picture.rgb, picture.samples, picture.frame) = (rgba, rgb, samples, frame);
                    picture.sampling = sampling;
                    picture.version += 1;
                    changed = true;
                }
                Update::Frame {
                    frame,
                    image,
                    objects,
                    materials,
                } => {
                    picture.frames_done += 1;
                    // The frame's passes for the viewer (a new texture only
                    // when they show something new), with adaptive sampling
                    // the samples each pixel got.
                    let mut passes = image.passes.clone();
                    if !image.sample_counts.is_empty() {
                        passes.push(grid4d_raytrace::PassLayer::samples(&image.sample_counts));
                        picture.sampling = sampling_of(&image, picture.total_samples);
                    }
                    if picture.passes != passes {
                        picture.passes = passes;
                        if picture.pass.is_some_and(|i| i >= picture.passes.len()) {
                            picture.pass = None;
                        }
                        picture.version += 1;
                    }
                    finished.push(FinishedFrame {
                        number: self.number,
                        frame,
                        image,
                        objects,
                        materials,
                    });
                }
                Update::Done { stopped } => {
                    ended = Some(stopped);
                    break;
                }
            }
        }
        picture.elapsed = self.started.elapsed();
        (changed, ended)
    }

    /// How far the render is (0…1).
    fn fraction(&self) -> f32 {
        let done = self.control.progress().min(self.total);
        // Display only: the precision of f32 is plenty.
        #[expect(clippy::cast_precision_loss, reason = "a progress bar")]
        let fraction = done as f32 / self.total.max(1) as f32;
        fraction
    }
}

/// What a Render View shows: when any of it changes, the picture is no longer
/// true and goes away.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewKey {
    /// Which view, through which camera, at which size in pixels.
    pub view: usize,
    pub camera: OrbitCamera,
    pub size: [u32; 2],
    /// The scene's revision, the render settings and the frame.
    pub revision: u64,
    pub settings: RenderSettings,
    pub frame: i32,
    /// The camera object the view looks through (its lens and shutter).
    pub object: Option<NodeId>,
}

/// The Render View: a render of the active view, drawn over it.
pub struct ViewRender {
    pub key: ViewKey,
    pub picture: Picture,
    job: Option<Job>,
}

impl ViewRender {
    #[must_use]
    pub fn is_rendering(&self) -> bool {
        self.job.is_some()
    }

    /// How far the render is (0…1), if it runs.
    #[must_use]
    pub fn progress(&self) -> Option<f32> {
        self.job.as_ref().map(Job::fraction)
    }
}

/// The session's renders (editor state): the running one and the history.
#[derive(Default)]
pub struct Renders {
    /// Oldest first.
    history: Vec<Picture>,
    /// The picture the viewer shows (`None`: the newest).
    pub shown: Option<u32>,
    /// The Picture Viewer window is open.
    pub viewer_open: bool,
    job: Option<Job>,
    last_number: u32,
    /// The Render View, if one shows.
    view: Option<ViewRender>,
}

impl std::fmt::Debug for Renders {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Renders")
            .field("pictures", &self.history.len())
            .field("shown", &self.shown)
            .field("rendering", &self.is_rendering())
            .field("view", &self.view.as_ref().map(|view| view.picture.samples))
            .finish_non_exhaustive()
    }
}

impl Renders {
    /// The session's pictures, oldest first.
    #[must_use]
    pub fn history(&self) -> &[Picture] {
        &self.history
    }

    /// The picture the viewer shows: the chosen one, else the newest.
    #[must_use]
    pub fn shown_picture(&self) -> Option<&Picture> {
        self.shown
            .and_then(|number| self.picture(number))
            .or_else(|| self.history.last())
    }

    /// Shows pass `pass` of the shown picture (an index into its passes;
    /// `None`: the picture itself).
    pub fn show_pass(&mut self, pass: Option<usize>) {
        let number = self.shown.or_else(|| self.history.last().map(|p| p.number));
        if let Some(picture) = self.history.iter_mut().find(|p| Some(p.number) == number)
            && picture.pass != pass
        {
            picture.pass = pass.filter(|&i| i < picture.passes.len());
            picture.version += 1;
        }
    }

    #[must_use]
    pub fn picture(&self, number: u32) -> Option<&Picture> {
        self.history.iter().find(|p| p.number == number)
    }

    #[must_use]
    pub fn is_rendering(&self) -> bool {
        self.job.is_some()
    }

    /// Whether any render runs (the Picture Viewer's or the Render View's).
    #[must_use]
    pub fn is_busy(&self) -> bool {
        self.job.is_some() || self.view.as_ref().is_some_and(ViewRender::is_rendering)
    }

    /// How far the running render is (0…1), if one runs.
    #[must_use]
    pub fn progress(&self) -> Option<(u32, f32)> {
        let job = self.job.as_ref()?;
        Some((job.number, job.fraction()))
    }

    /// The Render View, if one shows.
    #[must_use]
    pub fn view_render(&self) -> Option<&ViewRender> {
        self.view.as_ref()
    }

    /// Renders the view `key` describes (through `view`, at its size, with
    /// the Render Settings' quality) for the Render View, replacing one that
    /// shows. Nothing is saved or kept in the history.
    /// `replay`: the simulation (a copy of its cache), if there is one.
    /// `pictures`: the texture pictures decoded so far.
    pub fn start_view(
        &mut self,
        doc: &Document,
        pictures: Pictures,
        view: &OrbitCamera,
        key: ViewKey,
        replay: Option<Replay>,
    ) {
        self.view = None;
        let settings = doc.render.clone().validated();
        // The Render View renders the render region whenever there is one.
        let options = RenderOptions {
            width: key.size[0],
            height: key.size[1],
            region: settings.region.map(|region| region.pixels(key.size[0], key.size[1])),
            ..RenderOptions::from(&settings)
        };
        let job = Job::spawn(
            0,
            doc.scene.clone(),
            pictures,
            vec![doc.time.frame()],
            key.object.map_or(Eye::Fixed(render_camera(view)), |id| {
                Eye::Object(id, render_camera(view))
            }),
            Look {
                options,
                replay,
                segments: settings.motion_blur.then_some(settings.segments),
                // The Render View shows the picture only.
                passes: Passes::NONE,
                denoising: Denoising::view(&settings),
            },
        );
        let picture = Picture {
            number: 0,
            width: options.width,
            height: options.height,
            rgba: Vec::new(),
            rgb: Vec::new(),
            alpha: false,
            samples: 0,
            total_samples: settings.samples,
            elapsed: Duration::ZERO,
            state: if job.is_some() {
                RenderState::Rendering
            } else {
                RenderState::Stopped
            },
            frame: doc.time.frame(),
            frames: 1,
            frames_done: 0,
            saved: 0,
            target: SaveTarget::Nowhere(crate::i18n::key("The Render View is not saved.")),
            note: String::new(),
            version: 0,
            passes: Vec::new(),
            pass: None,
            sampling: None,
        };
        self.view = Some(ViewRender { key, picture, job });
    }

    /// Removes the Render View, stopping its render.
    pub fn clear_view(&mut self) {
        self.view = None;
    }

    /// Starts rendering `doc` with its Render Settings — every frame of their
    /// range — through `view`, or through camera object `object` (the one the
    /// view looks through) wherever it is at each frame; stops a render that
    /// runs. `document` is where the document is saved (for a relative output
    /// file). `pictures`: the texture pictures decoded so far. Returns the new
    /// picture's number.
    pub fn start(
        &mut self,
        doc: &Document,
        pictures: Pictures,
        view: &OrbitCamera,
        object: Option<NodeId>,
        document: Option<&std::path::Path>,
        replay: Option<Replay>,
    ) -> u32 {
        self.stop();
        let settings = doc.render.clone().validated();
        let options = RenderOptions::from(&settings);
        let frames: Vec<i32> = settings.frames(&doc.time).collect();
        let first = frames.first().copied().unwrap_or_else(|| doc.time.frame());
        let numbered = frames.len() > 1;
        self.last_number += 1;
        let number = self.last_number;
        self.history.push(Picture {
            number,
            width: settings.width,
            height: settings.height,
            rgba: Vec::new(),
            rgb: Vec::new(),
            alpha: settings.alpha,
            samples: 0,
            total_samples: settings.samples,
            elapsed: Duration::ZERO,
            state: RenderState::Rendering,
            frame: first,
            frames: u32::try_from(frames.len()).unwrap_or(u32::MAX),
            frames_done: 0,
            saved: 0,
            target: save_target(&settings, document, numbered),
            note: String::new(),
            version: 0,
            passes: Vec::new(),
            pass: None,
            sampling: None,
        });
        self.shown = None;

        let camera = render_camera(view);
        let eye = object.map_or(Eye::Fixed(camera), |id| Eye::Object(id, camera));
        let job = Job::spawn(
            number,
            doc.scene.clone(),
            pictures,
            frames,
            eye,
            Look {
                options,
                replay,
                segments: settings.motion_blur.then_some(settings.segments),
                passes: settings.passes,
                denoising: Denoising::render(&settings),
            },
        );
        if job.is_none() {
            self.end(number, true);
            return number;
        }
        self.job = job;
        self.trim();
        number
    }

    /// Stops the running render, keeping what it rendered so far.
    pub fn stop(&mut self) {
        if let Some(job) = self.job.take() {
            let number = job.number;
            drop(job);
            self.end(number, true);
        }
    }

    /// Takes the workers' news. Returns whether a picture changed (repaint),
    /// and the Picture Viewer render's frames that have just finished (to be
    /// saved, in order).
    pub fn poll(&mut self) -> (bool, Vec<FinishedFrame>) {
        let mut changed = false;
        let mut finished = Vec::new();
        if let Some(view) = &mut self.view
            && let Some(job) = &view.job
        {
            let (passed, ended) = job.drain(&mut view.picture, &mut finished);
            // The Render View is never saved.
            finished.clear();
            changed |= passed;
            if let Some(stopped) = ended {
                view.job = None;
                view.picture.state = if stopped {
                    RenderState::Stopped
                } else {
                    RenderState::Finished
                };
                changed = true;
            }
        }
        let Some(job) = &self.job else {
            return (changed, finished);
        };
        let number = job.number;
        let Some(picture) = self.history.iter_mut().find(|p| p.number == number) else {
            return (changed, finished);
        };
        let (passed, ended) = job.drain(picture, &mut finished);
        changed |= passed || !finished.is_empty();
        let Some(stopped) = ended else {
            return (changed, finished);
        };
        self.job = None;
        self.end(number, stopped);
        self.trim();
        (true, finished)
    }

    /// Notes that a frame of picture `number` was saved to `path`.
    pub fn record_saved(&mut self, number: u32, path: &std::path::Path) {
        if let Some(picture) = self.history.iter_mut().find(|p| p.number == number) {
            picture.saved += 1;
            picture.note = if picture.frames > 1 {
                trf(
                    "Saved {0} of {1} frames, the last to {2}",
                    &[&picture.saved, &picture.frames, &path.display()],
                )
            } else {
                trf("Saved to {0}", &[&path.display()])
            };
        }
    }

    /// Notes what happened to picture `number`'s file.
    pub fn set_note(&mut self, number: u32, note: String) {
        if let Some(picture) = self.history.iter_mut().find(|p| p.number == number) {
            picture.note = note;
        }
    }

    fn end(&mut self, number: u32, stopped: bool) {
        if let Some(picture) = self.history.iter_mut().find(|p| p.number == number) {
            picture.state = if stopped {
                RenderState::Stopped
            } else {
                RenderState::Finished
            };
            if stopped && picture.saved == 0 {
                picture.note = crate::i18n::tr("Stopped: not saved.").to_owned();
            } else if stopped {
                picture.note.push_str(" · stopped");
            }
        }
    }

    /// Drops the oldest pictures while the history takes more than
    /// [`HISTORY_BYTES`]; the newest and the running one always stay.
    fn trim(&mut self) {
        let running = self.job.as_ref().map(|job| job.number);
        let mut total: usize = self.history.iter().map(Picture::bytes).sum();
        while total > HISTORY_BYTES && self.history.len() > 1 {
            let Some(index) = self.history[..self.history.len() - 1]
                .iter()
                .position(|p| Some(p.number) != running)
            else {
                break;
            };
            total -= self.history.remove(index).bytes();
        }
        if self.shown.is_some_and(|number| self.picture(number).is_none()) {
            self.shown = None;
        }
    }
}

/// The camera a render looks through: the view's (the editor camera, or the
/// camera object the view looks through).
#[must_use]
pub fn render_camera(view: &OrbitCamera) -> RenderCamera {
    RenderCamera {
        world: view.eye_matrix(),
        projection: if view.is_parallel() {
            CameraProjection::Parallel {
                half_height: view.half_height(),
            }
        } else {
            CameraProjection::Perspective { fov_y: view.fov_y }
        },
        lens: None,
        end: None,
    }
}

/// Where a render with `settings` is saved, for a document saved at
/// `document`; `numbered`: an animation, one file per frame.
#[must_use]
pub fn save_target(settings: &RenderSettings, document: Option<&std::path::Path>, numbered: bool) -> SaveTarget {
    if !settings.save {
        return SaveTarget::Nowhere(key("Not saved: Save is off in the Render Settings."));
    }
    if settings.output.trim().is_empty() {
        return SaveTarget::Nowhere(key("Not saved: the Render Settings have no file."));
    }
    if settings.output_path(document).is_none() {
        return SaveTarget::Nowhere(key(
            "Not saved: the file is relative to the document's folder; save the document first.",
        ));
    }
    SaveTarget::File(Output {
        settings: settings.clone(),
        document: document.map(std::path::Path::to_path_buf),
        numbered,
    })
}

/// How a render looks: its options, the simulation it replays and, with
/// motion blur, how many moments of the shutter it evaluates.
struct Look {
    options: RenderOptions,
    replay: Option<Replay>,
    segments: Option<u32>,
    /// The render passes gathered beside the picture.
    passes: Passes,
    denoising: Denoising,
}

/// Which of a render's pictures are denoised, and how strongly (0…1).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Denoising {
    Off,
    /// The finished picture.
    Final(f32),
    /// Every picture from 4 samples on (the Render View).
    Live(f32),
}

impl Denoising {
    /// A finished render's, by `settings`.
    fn render(settings: &RenderSettings) -> Self {
        match settings.denoise {
            Denoise::Off => Self::Off,
            Denoise::Final | Denoise::FinalAndView => Self::Final(strength(settings)),
        }
    }

    /// The Render View's, by `settings`.
    fn view(settings: &RenderSettings) -> Self {
        match settings.denoise {
            Denoise::Off | Denoise::Final => Self::Off,
            Denoise::FinalAndView => Self::Live(strength(settings)),
        }
    }
}

/// The settings' Denoise Strength as a fraction.
fn strength(settings: &RenderSettings) -> f32 {
    f32::from(u16::try_from(settings.denoise_strength.min(100)).unwrap_or(100)) / 100.0
}

/// What a render worker renders: the scene, its texture pictures, its frames
/// and its eye.
struct Input<'a> {
    scene: &'a Scene,
    pictures: &'a Pictures,
    frames: &'a [i32],
    eye: Eye,
}

/// The scene's camera objects as evaluated, for Camera projections.
fn camera_objects(scene: &Scene, evaluated: &EvaluatedScene) -> Vec<(NodeId, RenderCamera)> {
    evaluated
        .nodes()
        .iter()
        .filter(|n| n.kind == ObjectKindTag::Camera)
        .filter_map(|n| match scene.get(n.id)?.data.kind {
            ObjectKind::Camera { fov_y_deg, .. } => Some((
                n.id,
                RenderCamera {
                    world: n.world,
                    projection: CameraProjection::Perspective {
                        fov_y: fov_y_deg.to_radians(),
                    },
                    lens: None,
                    end: None,
                },
            )),
            _ => None,
        })
        .collect()
}

/// The picture's width per unit of height.
fn aspect(options: &RenderOptions) -> f32 {
    let side = |n: u32| f32::from(u16::try_from(n.clamp(1, RenderSettings::MAX_SIZE)).unwrap_or(u16::MAX));
    side(options.width) / side(options.height)
}

/// The worker: for each frame, evaluates the scene for the renderer, then
/// renders pass after pass, sending each picture and the finished frame.
/// Stops when cancelled or nobody listens.
fn work(
    input: &Input<'_>,
    options: &RenderOptions,
    (segments, passes, denoising): (Option<u32>, Passes, Denoising),
    mut replay: Option<Replay>,
    control: &RenderControl,
    sender: &Sender<Update>,
) {
    let &Input {
        scene,
        pictures,
        frames,
        eye,
    } = input;
    let mut evaluated = EvaluatedScene::for_render();
    let mut simulated = grid4d_eval::Overrides::default();
    for &frame in frames {
        let at = f64::from(frame);
        // With motion blur, the scene at moments of the shutter interval.
        let (moments, base) = match segments {
            Some(segments) => shutter_moments(&eye.shutter(scene, at), at, segments),
            None => (vec![at], 0),
        };
        let mut scenes = Vec::with_capacity(moments.len());
        let mut placements = Vec::with_capacity(moments.len());
        for &time in &moments {
            // The simulation then (simulated here if the cache lacks it).
            if let Some(replay) = replay.as_mut() {
                replay.overrides_at(time, &mut simulated);
                evaluated.set_overrides(&simulated);
            }
            evaluated.set_frame(time);
            evaluated.update(scene);
            let camera = eye.at(scene, &evaluated, time);
            let cameras = camera_objects(scene, &evaluated);
            let texturing = Texturing {
                materials: scene.materials(),
                pictures,
                camera,
                aspect: aspect(options),
                cameras: &cameras,
            };
            scenes.push(RenderScene::textured(&evaluated, &texturing));
            placements.push(camera);
        }
        let (render_scene, still) = RenderScene::moving(scenes, base);
        if still > 0 {
            log::info!("frame {frame}: {still} objects not motion blurred (their shape changes over the shutter)");
        }
        // Through the camera at the frame, moving from the shutter's opening
        // to its closing.
        let (first, last) = (placements[0], placements[placements.len() - 1]);
        let camera = RenderCamera {
            world: first.world,
            end: (last.world != first.world).then_some(last.world),
            ..placements[base]
        };
        let mut progressive = Progressive::with_passes(options, passes, &render_scene);
        if denoising != Denoising::Off {
            progressive = progressive.with_denoising();
        }
        while !progressive.is_complete(options) {
            // 1, 1, 2, 4, … samples: a first picture at once, then fewer updates.
            let pass = progressive.samples().clamp(1, MAX_PASS);
            if !progressive.add_samples(&render_scene, &camera, options, pass, control) {
                let _ = sender.send(Update::Done { stopped: true });
                return;
            }
            let mut image = progressive.image();
            if let Denoising::Live(strength) = denoising
                && progressive.samples() >= 4
            {
                grid4d_raytrace::denoise(&mut image, strength);
            }
            let update = Update::Pass {
                rgba: image.to_rgba8(),
                rgb: image.to_rgb8(),
                samples: progressive.samples(),
                frame,
                sampling: sampling_of(&image, options.samples),
            };
            if sender.send(update).is_err() {
                return;
            }
        }
        let mut image = progressive.image();
        if let Denoising::Final(strength) | Denoising::Live(strength) = denoising {
            grid4d_raytrace::denoise(&mut image, strength);
        }
        // What the ID passes' numbers name.
        let objects = if passes.contains(Pass::ObjectId) {
            render_scene
                .object_ids()
                .iter()
                .filter_map(|&(id, node)| Some(format!("{id} = {}", scene.get(node)?.data.name)))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            String::new()
        };
        let materials = if passes.contains(Pass::MaterialId) {
            scene
                .materials()
                .iter()
                .enumerate()
                .map(|(i, (_, m))| format!("{} = {}", i + 1, m.name))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            String::new()
        };
        if sender
            .send(Update::Frame {
                frame,
                image,
                objects,
                materials,
            })
            .is_err()
        {
            return;
        }
    }
    let _ = sender.send(Update::Done { stopped: false });
}

/// With adaptive sampling: the samples `image`'s pixels got on average, and
/// the share that got all `total`.
fn sampling_of(image: &Image, total: u32) -> Option<(f32, f32)> {
    if image.sample_counts.is_empty() {
        return None;
    }
    // Pixels outside a render region got none.
    let rendered = image.sample_counts.iter().filter(|&&n| n > 0);
    let (pixels, sum) = rendered
        .clone()
        .fold((0u64, 0u64), |(p, s), &n| (p + 1, s + u64::from(n)));
    let full = rendered.filter(|&&n| n >= total).count();
    // Shown to one decimal: f32 is plenty.
    #[expect(clippy::cast_precision_loss, reason = "shown rounded")]
    let (average, share) = (sum as f32 / pixels.max(1) as f32, full as f32 / pixels.max(1) as f32);
    Some((average, share))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Polls until the running render ends (at most a few seconds); returns
    /// the frames it finished, in order.
    pub(crate) fn wait(renders: &mut Renders) -> Vec<FinishedFrame> {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut finished = Vec::new();
        while renders.is_rendering() {
            assert!(Instant::now() < deadline, "the render did not end");
            finished.extend(renders.poll().1);
            thread::sleep(Duration::from_millis(2));
        }
        finished
    }

    /// Polls until the Render View's render ends (at most a few seconds).
    pub(crate) fn wait_view(renders: &mut Renders) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while renders.view_render().is_some_and(ViewRender::is_rendering) {
            assert!(Instant::now() < deadline, "the render did not end");
            renders.poll();
            thread::sleep(Duration::from_millis(2));
        }
    }

    pub(crate) fn small(doc: &mut Document, samples: u32) {
        doc.render.width = 24;
        doc.render.height = 14;
        doc.render.samples = samples;
    }

    #[test]
    fn a_render_sharpens_in_passes_and_finishes() {
        let mut doc = Document::with_demo_scene();
        small(&mut doc, 5);
        let mut renders = Renders::default();
        let number = renders.start(&doc, Pictures::new(), &OrbitCamera::default(), None, None, None);
        assert_eq!(number, 1);
        assert!(renders.is_rendering());
        assert_eq!(renders.shown_picture().map(|p| p.state), Some(RenderState::Rendering));

        let finished = wait(&mut renders);
        assert_eq!(
            finished.iter().map(|f| (f.number, f.frame)).collect::<Vec<_>>(),
            [(1, 0)]
        );
        let picture = renders.shown_picture().unwrap();
        assert_eq!((picture.frames, picture.frames_done), (1, 1));
        assert_eq!(picture.state, RenderState::Finished);
        assert_eq!((picture.samples, picture.total_samples), (5, 5));
        assert_eq!(picture.rgba.len(), 24 * 14 * 4);
        assert_eq!(picture.rgb.len(), 24 * 14 * 3);
        // Passes of 1, 1, 2 then the last sample: a new version each.
        assert_eq!(picture.version, 4);
        assert!(picture.rgba.chunks(4).any(|p| p[3] == 255), "the scene is in view");

        // The same picture as one render through the same camera.
        let mut evaluated = EvaluatedScene::for_render();
        evaluated.update(&doc.scene);
        let whole = grid4d_raytrace::render(
            &RenderScene::new(&evaluated),
            &render_camera(&OrbitCamera::default()),
            &RenderOptions::from(&doc.render),
        );
        assert_eq!(picture.rgba, whole.to_rgba8());
    }

    #[test]
    fn final_renders_use_the_region_only_when_asked() {
        let mut doc = Document::with_demo_scene();
        small(&mut doc, 2);
        doc.render.region = grid4d_scene::RenderRegion::new(0, 0, 5000, 10_000);
        let render = |doc: &Document| {
            let mut renders = Renders::default();
            renders.start(doc, Pictures::new(), &OrbitCamera::default(), None, None, None);
            wait(&mut renders).remove(0).image
        };
        // Off: the whole picture (the scene's right half too).
        let whole = render(&doc);
        let right = |image: &Image| image.pixels.chunks(24).flat_map(|row| &row[12..]).any(|p| p[3] > 0.0);
        assert!(right(&whole), "the demo scene reaches the right half");
        doc.render.region_final = true;
        let part = render(&doc);
        assert!(!right(&part), "only the left half rendered");
        assert!(part.pixels.iter().any(|p| p[3] > 0.0));
    }

    #[test]
    fn new_documents_render_adaptively_and_denoised() {
        let mut doc = Document::with_demo_scene();
        doc.render = RenderSettings::for_new_documents();
        small(&mut doc, 32);
        let mut renders = Renders::default();
        renders.start(&doc, Pictures::new(), &OrbitCamera::default(), None, None, None);
        let finished = wait(&mut renders);
        let image = &finished[0].image;
        // Denoised: the picture before kept as a pass; adaptive: counts per
        // pixel, the empty background stopped at the minimum.
        assert!(image.passes.iter().any(|l| l.name == "Noisy"));
        assert_eq!(image.sample_counts.iter().min(), Some(&16));
        let picture = renders.shown_picture().unwrap();
        let (average, share) = picture.sampling.unwrap();
        assert!((16.0..=32.0).contains(&average) && (0.0..=1.0).contains(&share));
        // The viewer can show where the samples went and the picture before.
        let names: Vec<_> = picture.passes.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Noisy", "Samples"]);
        // The Render View denoises only when asked.
        assert_eq!(Denoising::view(&doc.render), Denoising::Off);
        doc.render.denoise = Denoise::FinalAndView;
        assert_eq!(Denoising::view(&doc.render), Denoising::Live(1.0));
    }

    #[test]
    fn stopping_keeps_the_picture_and_a_new_render_replaces_the_old() {
        let mut doc = Document::with_demo_scene();
        doc.render.samples = 4096;
        let mut renders = Renders::default();
        renders.start(&doc, Pictures::new(), &OrbitCamera::default(), None, None, None);
        renders.stop();
        assert!(!renders.is_rendering());
        assert_eq!(renders.picture(1).map(|p| p.state), Some(RenderState::Stopped));

        renders.start(&doc, Pictures::new(), &OrbitCamera::default(), None, None, None);
        renders.start(&doc, Pictures::new(), &OrbitCamera::default(), None, None, None);
        assert_eq!(
            renders.picture(2).map(|p| p.state),
            Some(RenderState::Stopped),
            "replaced"
        );
        assert_eq!(renders.shown_picture().map(|p| p.number), Some(3));
        renders.stop();
        assert!(wait(&mut renders).is_empty());
        assert_eq!(renders.history().len(), 3);
    }

    #[test]
    fn the_render_view_renders_the_view_and_keeps_nothing() {
        let mut doc = Document::with_demo_scene();
        small(&mut doc, 3);
        let camera = OrbitCamera::default();
        let key = ViewKey {
            view: 0,
            camera,
            size: [40, 30],
            revision: doc.scene.revision(),
            settings: doc.render.clone(),
            frame: 0,
            object: None,
        };
        let mut renders = Renders::default();
        renders.start_view(&doc, Pictures::new(), &camera, key.clone(), None);
        assert!(
            renders.is_busy() && !renders.is_rendering(),
            "not a Picture Viewer render"
        );
        wait_view(&mut renders);
        let view = renders.view_render().unwrap();
        assert_eq!(view.key, key);
        assert_eq!(view.picture.state, RenderState::Finished);
        assert_eq!(
            (view.picture.width, view.picture.height, view.picture.samples),
            (40, 30, 3)
        );
        assert!(renders.history().is_empty(), "not in the history");

        // The view's size and camera, the settings' quality.
        let mut evaluated = EvaluatedScene::for_render();
        evaluated.update(&doc.scene);
        let options = RenderOptions {
            width: 40,
            height: 30,
            ..RenderOptions::from(&doc.render)
        };
        let whole = grid4d_raytrace::render(&RenderScene::new(&evaluated), &render_camera(&camera), &options);
        assert_eq!(view.picture.rgb, whole.to_rgb8());

        renders.start_view(&doc, Pictures::new(), &camera, key, None);
        renders.clear_view();
        assert!(renders.view_render().is_none() && !renders.is_busy());
    }

    #[test]
    fn a_range_renders_every_frame_through_an_animated_camera() {
        let mut doc = Document::with_demo_scene();
        small(&mut doc, 1);
        doc.render.frames = grid4d_scene::FrameRange::Manual { from: 0, to: 20 };
        doc.render.frame_step = 10;
        // The camera moves sideways between frame 0 and 20.
        let camera = doc.scene.iter().find(|(_, n)| n.data.name == "Camera").unwrap().0;
        let data = &mut doc.scene.get_mut(camera).unwrap().data;
        data.record(grid4d_scene::Param::TRANSFORM, 0.0, 0.0);
        let mut moved = data.transform_at(20.0);
        moved.position.x += 400.0;
        data.set_transform_at(&moved, 20.0, true);

        let mut renders = Renders::default();
        renders.start(&doc, Pictures::new(), &OrbitCamera::default(), Some(camera), None, None);
        let finished = wait(&mut renders);
        let frames: Vec<i32> = finished.iter().map(|f| f.frame).collect();
        assert_eq!(frames, [0, 10, 20], "every 10th frame, in order");
        let picture = renders.shown_picture().unwrap();
        assert_eq!((picture.frames, picture.frames_done, picture.frame), (3, 3, 20));
        assert_eq!(picture.state, RenderState::Finished);
        assert_ne!(finished[0].image, finished[2].image, "the camera moved");
        assert_eq!((finished[0].image.width, finished[0].image.height), (24, 14));
    }

    #[test]
    fn renders_show_texture_pictures_and_know_the_camera_objects() {
        use std::sync::Arc;

        use grid4d_raytrace::{Picture, Texels};
        use grid4d_scene::{Channel, Material, MaterialTag, Node, Tag, Texture, TextureSlot, Transform};

        let mut doc = Document::new("textured");
        small(&mut doc, 1);
        doc.render.auto_light = true;
        let texture = doc.scene.textures_mut().insert(Texture::linked("red.png".into()));
        let mut white = Material::new("White");
        (white.color, white.roughness) = ([1.0; 3], 1.0);
        *white.map_mut(Channel::BaseColor) = Some(TextureSlot::new(texture));
        let white = doc.scene.materials_mut().insert(white);
        let mut floor = Node::new("Floor", ObjectKindTag::Plane.default_kind());
        floor.data.tags = vec![Tag::Material(MaterialTag::new(Some(white)))];
        doc.scene.insert(floor);
        let camera = doc.scene.insert(
            Node::new("Camera", ObjectKindTag::Camera.default_kind())
                .with_transform(Transform::default().with_rotation_deg(glam::Vec3::new(0.0, -90.0, 0.0)))
                .with_position(glam::Vec3::new(0.0, 300.0, 0.0)),
        );
        let mut evaluated = EvaluatedScene::for_render();
        evaluated.update(&doc.scene);
        let cameras = camera_objects(&doc.scene, &evaluated);
        assert_eq!(cameras.len(), 1);
        assert_eq!(cameras[0].0, camera);

        let centre = |pictures: Pictures| {
            let mut renders = Renders::default();
            renders.start(&doc, pictures, &OrbitCamera::default(), Some(camera), None, None);
            let finished = wait(&mut renders);
            finished[0].image.pixel(12, 7).unwrap()
        };
        let mut pictures = Pictures::new();
        let red = Texels::Srgb8(Arc::from([255u8, 0, 0, 255]));
        pictures.insert(texture, Arc::new(Picture::new(1, 1, red).unwrap()));
        let textured = centre(pictures);
        assert!(textured[0] > 0.3 && textured[1] < 0.05, "{textured:?}");
        let plain = centre(Pictures::new());
        assert!(plain[1] > 0.3, "the channel's value while loading: {plain:?}");
    }

    #[test]
    fn where_renders_are_saved() {
        let document = std::path::Path::new("/shots/a.g4d");
        let mut settings = RenderSettings::default();
        assert!(matches!(
            save_target(&settings, Some(document), false),
            SaveTarget::Nowhere(_)
        ));
        settings.output = "out".to_owned();
        let SaveTarget::File(single) = save_target(&settings, Some(document), false) else {
            panic!("saved")
        };
        assert_eq!(single.file(7), Some(PathBuf::from("/shots/out.png")));
        let SaveTarget::File(animation) = save_target(&settings, Some(document), true) else {
            panic!("saved")
        };
        assert_eq!(animation.file(7), Some(PathBuf::from("/shots/out0007.png")));
        assert!(matches!(save_target(&settings, None, false), SaveTarget::Nowhere(_)));
        settings.save = false;
        assert!(matches!(
            save_target(&settings, Some(document), false),
            SaveTarget::Nowhere(_)
        ));
    }

    #[test]
    fn old_pictures_leave_the_history_past_its_limit() {
        let mut renders = Renders::default();
        let big = vec![0; HISTORY_BYTES / 3];
        for number in 1..=4 {
            renders.history.push(Picture {
                number,
                width: 1,
                height: 1,
                rgba: big.clone(),
                rgb: Vec::new(),
                alpha: false,
                samples: 1,
                total_samples: 1,
                elapsed: Duration::ZERO,
                state: RenderState::Finished,
                frame: 0,
                frames: 1,
                frames_done: 1,
                saved: 0,
                target: SaveTarget::Nowhere(""),
                note: String::new(),
                version: 1,
                passes: Vec::new(),
                pass: None,
                sampling: None,
            });
        }
        renders.shown = Some(1);
        renders.trim();
        let numbers: Vec<u32> = renders.history().iter().map(|p| p.number).collect();
        assert_eq!(numbers, [2, 3, 4], "the oldest went");
        assert_eq!(renders.shown, None, "it showed the dropped one");
    }

    #[test]
    fn renders_show_the_simulation_at_their_frame() {
        use grid4d_scene::{Body, Node, NodeData, ObjectKindTag, Tag};
        // A big cube that falls out of the default camera's view in a second.
        let mut doc = Document::new("fall");
        small(&mut doc, 1);
        let mut cube = NodeData::new("Cube", ObjectKindTag::Cube.default_kind());
        cube.tags.push(Tag::Dynamics(Body::rigid()));
        doc.scene.insert(Node::from_data(cube));
        doc.render.frames = grid4d_scene::FrameRange::Current;
        let picture = |doc: &Document, replay: Option<Replay>| {
            let mut renders = Renders::default();
            renders.start(doc, Pictures::new(), &OrbitCamera::default(), None, None, replay);
            wait(&mut renders);
            renders.shown_picture().unwrap().rgba.clone()
        };
        let still = picture(&doc, None);
        doc.time.set_frame(40);
        let mut sim = crate::simulation::Sim::default();
        // The cache holds only the start; the render simulates the rest itself.
        let replay = sim.replay(&doc);
        assert!(replay.is_some());
        let fallen = picture(&doc, replay);
        assert_ne!(still, fallen, "the cube has fallen out of view");
        assert!(fallen.chunks(4).all(|p| p[3] == 0), "nothing left in view");
    }

    #[test]
    fn the_shutter_opens_its_moments_around_the_frame() {
        let centred = Lens::default();
        assert_eq!(shutter_moments(&centred, 10.0, 3), (vec![9.75, 10.0, 10.25], 1));
        let opening = Lens {
            shutter_position: grid4d_scene::ShutterPosition::Opening,
            ..centred
        };
        assert_eq!(shutter_moments(&opening, 10.0, 2), (vec![10.0, 10.5], 0));
        let closing = Lens {
            shutter_position: grid4d_scene::ShutterPosition::Closing,
            shutter: 1.0,
            ..centred
        };
        assert_eq!(shutter_moments(&closing, 10.0, 3), (vec![9.0, 9.5, 10.0], 2));
        // A closed shutter: the frame alone.
        let closed = Lens {
            shutter: 0.0,
            ..centred
        };
        assert_eq!(shutter_moments(&closed, 10.0, 5), (vec![10.0], 0));
    }

    /// A cube moving 1000 cm along +X per frame seen from above through a
    /// camera object with `lens`, rendered at frame 10 (`blur`: motion
    /// blur on); the picture's columns where the cube covers the middle row.
    fn smear(lens: Lens, blur: bool) -> Vec<u32> {
        let mut doc = Document::new("smear");
        doc.render.width = 60;
        doc.render.height = 20;
        doc.render.samples = 128;
        doc.render.bounces = 0;
        doc.render.motion_blur = blur;
        doc.render.segments = 3;
        doc.time.set_frame(10);
        let cube = doc.scene.insert(grid4d_scene::Node::new(
            "Cube",
            ObjectKind::Cube {
                size: glam::Vec3::splat(40.0),
                segments: [1, 1, 1],
            },
        ));
        let data = &mut doc.scene.get_mut(cube).unwrap().data;
        data.tracks.push(grid4d_scene::Track {
            param: grid4d_scene::Param::Position(grid4d_scene::Axis::X),
            curve: grid4d_scene::Curve::from_keys([
                grid4d_scene::Key {
                    interpolation: grid4d_scene::Interpolation::Linear,
                    ..grid4d_scene::Key::new(0.0, -10000.0)
                },
                grid4d_scene::Key {
                    interpolation: grid4d_scene::Interpolation::Linear,
                    ..grid4d_scene::Key::new(20.0, 10000.0)
                },
            ]),
        });
        // Looking straight down from 1000 cm: 40° shows about 728 cm high.
        let look = glam::camera::rh::view::look_at_mat4(
            glam::Vec3::new(0.0, 1000.0, 0.0),
            glam::Vec3::ZERO,
            glam::Vec3::NEG_Z,
        )
        .inverse();
        let camera = doc.scene.insert(
            grid4d_scene::Node::new("Camera", ObjectKind::Camera { fov_y_deg: 40.0, lens })
                .with_transform(grid4d_scene::Transform::from_matrix(&look)),
        );
        let mut renders = Renders::default();
        renders.start(&doc, Pictures::new(), &OrbitCamera::default(), Some(camera), None, None);
        let finished = wait(&mut renders);
        let image = &finished[0].image;
        (0..image.width)
            .filter(|&x| image.pixel(x, image.height / 2).unwrap()[3] > 0.01)
            .collect()
    }

    #[test]
    fn motion_blur_smears_over_the_shutter_and_its_position_moves_the_smear() {
        let lens = Lens::default();
        let sharp = smear(lens, false);
        let blurred = smear(lens, true);
        // 40 cm wide; the picture 60 × 728 / 20 cm per pixel ≈ 36 cm.
        assert!(sharp.len() <= 3, "{sharp:?}");
        // Half a frame: 500 cm more, about 14 pixels.
        let span = |columns: &[u32]| columns[columns.len() - 1] - columns[0] + 1;
        assert!((12..=17).contains(&span(&blurred)), "{blurred:?} vs {sharp:?}");
        let centre = |columns: &[u32]| (columns[0] + columns[columns.len() - 1]) as f32 / 2.0;
        assert!((centre(&blurred) - centre(&sharp)).abs() <= 1.0, "centred on the frame");
        let opening = smear(
            Lens {
                shutter_position: grid4d_scene::ShutterPosition::Opening,
                ..lens
            },
            true,
        );
        assert!(centre(&opening) > centre(&sharp) + 5.0, "after the frame: {opening:?}");
        // A closed shutter: the frame's picture.
        assert_eq!(smear(Lens { shutter: 0.0, ..lens }, true), sharp);
    }
}
