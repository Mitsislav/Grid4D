//! Final renders in the background (see `docs/reference/render-picture-viewer.md`
//! and `render-animation.md`): a render runs on a worker thread over a copy of
//! the scene, frame after frame of its range, and sends a picture after every
//! pass, so the Picture Viewer shows it sharpening while the editor stays
//! usable; each finished frame is handed back to be saved. [`Renders`] (editor state) keeps the running job and the
//! session's pictures; [`Renders::poll`] takes the worker's news once a frame.

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
use grid4d_raytrace::{CameraProjection, Image, Progressive, RenderCamera, RenderControl, RenderOptions, RenderScene};
use grid4d_render::OrbitCamera;
use grid4d_scene::{Document, ImageFormat, NodeId, ObjectKind, RenderSettings, Scene};
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
    pub image: Image,
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

    /// Whether there is anything to show or save yet.
    #[must_use]
    pub fn has_pixels(&self) -> bool {
        !self.rgba.is_empty()
    }

    fn bytes(&self) -> usize {
        self.rgba.len() + self.rgb.len()
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
    },
    /// Every sample of `frame` is in.
    Frame { frame: i32, image: Image },
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
    fn at(self, scene: &Scene, evaluated: &EvaluatedScene, frame: f64) -> RenderCamera {
        let (id, fallback) = match self {
            Self::Fixed(camera) => return camera,
            Self::Object(id, fallback) => (id, fallback),
        };
        let Some(node) = scene.get(id) else {
            return fallback;
        };
        let world = evaluated
            .nodes()
            .iter()
            .find(|n| n.id == id)
            .map_or_else(|| scene.world_matrix_at(id, frame), |n| n.world);
        let projection = match node.data.kind {
            ObjectKind::Camera { fov_y_deg } => CameraProjection::Perspective {
                fov_y: fov_y_deg.to_radians(),
            },
            _ => fallback.projection,
        };
        RenderCamera { world, projection }
    }
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
    fn spawn(
        number: u32,
        scene: Scene,
        frames: Vec<i32>,
        eye: Eye,
        options: RenderOptions,
        replay: Option<Replay>,
    ) -> Option<Self> {
        let control = Arc::new(RenderControl::default());
        let (sender, updates) = mpsc::channel();
        let worker_control = Arc::clone(&control);
        let count = u64::try_from(frames.len()).unwrap_or(1).max(1);
        let spawned = thread::Builder::new()
            .name(format!("grid4d render {number}"))
            .spawn(move || work(&scene, &frames, eye, &options, replay, &worker_control, &sender));
        if let Err(err) = spawned {
            log::error!("could not start the render thread: {err}");
            return None;
        }
        let rows = options.height.clamp(1, RenderSettings::MAX_SIZE);
        let samples = options.samples.clamp(1, RenderSettings::MAX_SAMPLES);
        Some(Self {
            number,
            control,
            updates,
            started: Instant::now(),
            total: u64::from(rows) * u64::from(samples) * count,
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
                } => {
                    (picture.rgba, picture.rgb, picture.samples, picture.frame) = (rgba, rgb, samples, frame);
                    picture.version += 1;
                    changed = true;
                }
                Update::Frame { frame, image } => {
                    picture.frames_done += 1;
                    finished.push(FinishedFrame {
                        number: self.number,
                        frame,
                        image,
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
    pub fn start_view(&mut self, doc: &Document, view: &OrbitCamera, key: ViewKey, replay: Option<Replay>) {
        self.view = None;
        let settings = doc.render.clone().validated();
        let options = RenderOptions {
            width: key.size[0],
            height: key.size[1],
            ..RenderOptions::from(&settings)
        };
        let job = Job::spawn(
            0,
            doc.scene.clone(),
            vec![doc.time.frame()],
            Eye::Fixed(render_camera(view)),
            options,
            replay,
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
    /// file). Returns the new picture's number.
    pub fn start(
        &mut self,
        doc: &Document,
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
        });
        self.shown = None;

        let camera = render_camera(view);
        let eye = object.map_or(Eye::Fixed(camera), |id| Eye::Object(id, camera));
        let job = Job::spawn(number, doc.scene.clone(), frames, eye, options, replay);
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

/// The worker: for each frame, evaluates the scene for the renderer, then
/// renders pass after pass, sending each picture and the finished frame.
/// Stops when cancelled or nobody listens.
fn work(
    scene: &Scene,
    frames: &[i32],
    eye: Eye,
    options: &RenderOptions,
    mut replay: Option<Replay>,
    control: &RenderControl,
    sender: &Sender<Update>,
) {
    let mut evaluated = EvaluatedScene::for_render();
    let mut simulated = grid4d_eval::Overrides::default();
    for &frame in frames {
        // The simulation at this frame (simulated here if the cache lacks it).
        if let Some(replay) = replay.as_mut() {
            replay.overrides(frame, &mut simulated);
            evaluated.set_overrides(&simulated);
        }
        evaluated.set_frame(f64::from(frame));
        evaluated.update(scene);
        let camera = eye.at(scene, &evaluated, f64::from(frame));
        let render_scene = RenderScene::new(&evaluated);
        let mut progressive = Progressive::new(options);
        while !progressive.is_complete(options) {
            // 1, 1, 2, 4, … samples: a first picture at once, then fewer updates.
            let pass = progressive.samples().clamp(1, MAX_PASS);
            if !progressive.add_samples(&render_scene, &camera, options, pass, control) {
                let _ = sender.send(Update::Done { stopped: true });
                return;
            }
            let image = progressive.image();
            let update = Update::Pass {
                rgba: image.to_rgba8(),
                rgb: image.to_rgb8(),
                samples: progressive.samples(),
                frame,
            };
            if sender.send(update).is_err() {
                return;
            }
        }
        let image = progressive.image();
        if sender.send(Update::Frame { frame, image }).is_err() {
            return;
        }
    }
    let _ = sender.send(Update::Done { stopped: false });
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
        let number = renders.start(&doc, &OrbitCamera::default(), None, None, None);
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
    fn stopping_keeps_the_picture_and_a_new_render_replaces_the_old() {
        let mut doc = Document::with_demo_scene();
        doc.render.samples = 4096;
        let mut renders = Renders::default();
        renders.start(&doc, &OrbitCamera::default(), None, None, None);
        renders.stop();
        assert!(!renders.is_rendering());
        assert_eq!(renders.picture(1).map(|p| p.state), Some(RenderState::Stopped));

        renders.start(&doc, &OrbitCamera::default(), None, None, None);
        renders.start(&doc, &OrbitCamera::default(), None, None, None);
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
        };
        let mut renders = Renders::default();
        renders.start_view(&doc, &camera, key.clone(), None);
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

        renders.start_view(&doc, &camera, key, None);
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
        renders.start(&doc, &OrbitCamera::default(), Some(camera), None, None);
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
            renders.start(doc, &OrbitCamera::default(), None, None, replay);
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
}
