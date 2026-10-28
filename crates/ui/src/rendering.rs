//! Final renders in the background (see `docs/reference/render-picture-viewer.md`):
//! a render runs on a worker thread over a copy of the scene and sends a picture
//! after every pass, so the Picture Viewer shows it sharpening while the editor
//! stays usable. [`Renders`] (editor state) keeps the running job and the
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

use grid4d_eval::EvaluatedScene;
use grid4d_raytrace::{CameraProjection, Progressive, RenderCamera, RenderControl, RenderOptions, RenderScene};
use grid4d_render::OrbitCamera;
use grid4d_scene::{Document, RenderSettings, Scene};

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

/// Where a finished render goes, decided when it starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveTarget {
    /// The Render Settings' file.
    File(PathBuf),
    /// Not saved, and why.
    Nowhere(&'static str),
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
    pub target: SaveTarget,
    /// What happened to the file: where it was saved, or why not. Empty while rendering.
    pub note: String,
    /// Changes with every new pass (the viewer re-uploads its texture).
    pub version: u64,
}

impl Picture {
    /// The pixels as they are shown and saved: with transparency or without.
    #[must_use]
    pub fn pixels(&self) -> (&[u8], grid4d_io::PngChannels) {
        if self.alpha {
            (&self.rgba, grid4d_io::PngChannels::Rgba)
        } else {
            (&self.rgb, grid4d_io::PngChannels::Rgb)
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
    /// A pass is done: the picture so far.
    Pass { rgba: Vec<u8>, rgb: Vec<u8>, samples: u32 },
    /// The render ended (stopped: cancelled before the last pass).
    Done { stopped: bool },
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
}

impl std::fmt::Debug for Renders {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Renders")
            .field("pictures", &self.history.len())
            .field("shown", &self.shown)
            .field("rendering", &self.is_rendering())
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

    /// How far the running render is (0…1), if one runs.
    #[must_use]
    pub fn progress(&self) -> Option<(u32, f32)> {
        let job = self.job.as_ref()?;
        let done = job.control.progress().min(job.total);
        // Display only: the precision of f32 is plenty.
        #[expect(clippy::cast_precision_loss, reason = "a progress bar")]
        let fraction = done as f32 / job.total.max(1) as f32;
        Some((job.number, fraction))
    }

    /// Starts rendering `doc` at its current frame through `view` with its
    /// Render Settings, stopping a render that runs. `document` is where the
    /// document is saved (for a relative output file). Returns the new
    /// picture's number.
    pub fn start(&mut self, doc: &Document, view: &OrbitCamera, document: Option<&std::path::Path>) -> u32 {
        self.stop();
        let settings = doc.render.clone().validated();
        let options = RenderOptions::from(&settings);
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
            target: save_target(&settings, document),
            note: String::new(),
            version: 0,
        });
        self.shown = None;

        let control = Arc::new(RenderControl::default());
        let (sender, updates) = mpsc::channel();
        let scene = doc.scene.clone();
        let frame = f64::from(doc.time.frame());
        let camera = render_camera(view);
        let worker_control = Arc::clone(&control);
        let spawned = thread::Builder::new()
            .name(format!("grid4d render {number}"))
            .spawn(move || work(&scene, frame, &camera, &options, &worker_control, &sender));
        if let Err(err) = spawned {
            log::error!("could not start the render thread: {err}");
            self.end(number, true);
            return number;
        }
        self.job = Some(Job {
            number,
            control,
            updates,
            started: Instant::now(),
            total: u64::from(settings.height) * u64::from(settings.samples),
        });
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

    /// Takes the worker's news. Returns `(changed, finished)`: whether a picture
    /// changed (repaint), and the number of a render that has just finished
    /// completely (to be saved).
    pub fn poll(&mut self) -> (bool, Option<u32>) {
        let Some(job) = &self.job else {
            return (false, None);
        };
        let (number, elapsed) = (job.number, job.started.elapsed());
        let mut changed = false;
        let mut ended = None;
        loop {
            let update = match job.updates.try_recv() {
                Ok(update) => update,
                Err(mpsc::TryRecvError::Empty) => break,
                // The worker is gone without saying so (it panicked).
                Err(mpsc::TryRecvError::Disconnected) => {
                    ended = Some(true);
                    break;
                }
            };
            match update {
                Update::Pass { rgba, rgb, samples } => {
                    if let Some(picture) = self.history.iter_mut().find(|p| p.number == number) {
                        (picture.rgba, picture.rgb, picture.samples) = (rgba, rgb, samples);
                        picture.version += 1;
                    }
                    changed = true;
                }
                Update::Done { stopped } => {
                    ended = Some(stopped);
                    break;
                }
            }
        }
        if let Some(picture) = self.history.iter_mut().find(|p| p.number == number) {
            picture.elapsed = elapsed;
        }
        let Some(stopped) = ended else {
            return (changed, None);
        };
        self.job = None;
        self.end(number, stopped);
        self.trim();
        (true, (!stopped).then_some(number))
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
            if stopped {
                picture.note = "Stopped: not saved.".to_owned();
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

/// Where a render with `settings` is saved, for a document saved at `document`.
#[must_use]
pub fn save_target(settings: &RenderSettings, document: Option<&std::path::Path>) -> SaveTarget {
    if !settings.save {
        return SaveTarget::Nowhere("Not saved: Save is off in the Render Settings.");
    }
    if settings.output.trim().is_empty() {
        return SaveTarget::Nowhere("Not saved: the Render Settings have no file.");
    }
    settings.output_path(document).map_or(
        SaveTarget::Nowhere("Not saved: the file is relative to the document's folder; save the document first."),
        SaveTarget::File,
    )
}

/// The worker: evaluates the scene for the renderer, then renders pass after
/// pass, sending each picture. Stops when cancelled or nobody listens.
fn work(
    scene: &Scene,
    frame: f64,
    camera: &RenderCamera,
    options: &RenderOptions,
    control: &RenderControl,
    sender: &Sender<Update>,
) {
    let mut evaluated = EvaluatedScene::for_render();
    evaluated.set_frame(frame);
    evaluated.update(scene);
    let scene = RenderScene::new(&evaluated);
    drop(evaluated);

    let mut progressive = Progressive::new(options);
    while !progressive.is_complete(options) {
        // 1, 1, 2, 4, … samples: a first picture at once, then fewer updates.
        let pass = progressive.samples().clamp(1, MAX_PASS);
        if !progressive.add_samples(&scene, camera, options, pass, control) {
            let _ = sender.send(Update::Done { stopped: true });
            return;
        }
        let image = progressive.image();
        let update = Update::Pass {
            rgba: image.to_rgba8(),
            rgb: image.to_rgb8(),
            samples: progressive.samples(),
        };
        if sender.send(update).is_err() {
            return;
        }
    }
    let _ = sender.send(Update::Done { stopped: false });
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Polls until the running render ends (at most a few seconds); returns
    /// the number of a render that finished completely.
    pub(crate) fn wait(renders: &mut Renders) -> Option<u32> {
        let deadline = Instant::now() + Duration::from_secs(20);
        while renders.is_rendering() {
            assert!(Instant::now() < deadline, "the render did not end");
            if let (_, Some(number)) = renders.poll() {
                return Some(number);
            }
            thread::sleep(Duration::from_millis(2));
        }
        None
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
        let number = renders.start(&doc, &OrbitCamera::default(), None);
        assert_eq!(number, 1);
        assert!(renders.is_rendering());
        assert_eq!(renders.shown_picture().map(|p| p.state), Some(RenderState::Rendering));

        assert_eq!(wait(&mut renders), Some(1));
        let picture = renders.shown_picture().unwrap();
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
        renders.start(&doc, &OrbitCamera::default(), None);
        renders.stop();
        assert!(!renders.is_rendering());
        assert_eq!(renders.picture(1).map(|p| p.state), Some(RenderState::Stopped));

        renders.start(&doc, &OrbitCamera::default(), None);
        renders.start(&doc, &OrbitCamera::default(), None);
        assert_eq!(
            renders.picture(2).map(|p| p.state),
            Some(RenderState::Stopped),
            "replaced"
        );
        assert_eq!(renders.shown_picture().map(|p| p.number), Some(3));
        renders.stop();
        assert_eq!(wait(&mut renders), None);
        assert_eq!(renders.history().len(), 3);
    }

    #[test]
    fn where_renders_are_saved() {
        let document = std::path::Path::new("/shots/a.g4d");
        let mut settings = RenderSettings::default();
        assert!(matches!(save_target(&settings, Some(document)), SaveTarget::Nowhere(_)));
        settings.output = "out".to_owned();
        assert_eq!(
            save_target(&settings, Some(document)),
            SaveTarget::File(PathBuf::from("/shots/out.png"))
        );
        assert!(matches!(save_target(&settings, None), SaveTarget::Nowhere(_)));
        settings.save = false;
        assert!(matches!(save_target(&settings, Some(document)), SaveTarget::Nowhere(_)));
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
}
