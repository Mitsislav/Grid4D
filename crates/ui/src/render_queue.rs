//! The Render Queue (Render ▸ Render Queue…): saved documents rendered one
//! after another in the background, each frame saved as Shift+R saves it
//! (see `docs/reference/render-queue.md`).
//!
//! The queue is editor state kept by the app (the binary saves
//! [`RenderQueue::saved`] in its storage), never in documents, and is not
//! undoable. A job's file is read, switched to its take and its pictures
//! decoded on a loader thread; its frames then render through
//! [`Renders`](crate::rendering::Renders), one render at a time, so a job
//! shows in the Picture Viewer's history like any render.

use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
};

use grid4d_raytrace::Pictures;
use grid4d_scene::{Document, FrameRange, NodeId, ObjectKind};
use grid4d_sim::{Replay, Simulation};
use serde::{Deserialize, Serialize};

use crate::{
    i18n::{key, trf},
    rendering::{RenderState, Renders, SaveTarget, save_target},
};

/// Where the app keeps the queue (eframe storage).
pub const STORAGE_KEY: &str = "grid4d.render_queue";

/// How far a job is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobStatus {
    #[default]
    Ready,
    /// Reading its file or rendering.
    Rendering,
    Finished,
    /// Stopped before the end; Start continues from its first missing frame.
    Stopped,
    Error,
}

impl JobStatus {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Ready => "Ready",
            Self::Rendering => "Rendering",
            Self::Finished => "Finished",
            Self::Stopped => "Stopped",
            Self::Error => "Error",
        }
    }
}

/// Which take of its document a job renders.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobTake {
    /// The document's render take (Render Settings ▸ Take), else the active one.
    #[default]
    RenderTake,
    /// Main, the document without takes' changes.
    Main,
    /// The take of this name.
    Named(String),
}

/// Which frames a job renders.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobFrames {
    /// The document's Render Settings' range.
    #[default]
    FromDocument,
    Manual {
        from: i32,
        to: i32,
        step: u32,
    },
}

/// One document to render.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueueJob {
    pub enabled: bool,
    /// The saved `.g4d` rendered (read when the job starts).
    pub path: PathBuf,
    pub take: JobTake,
    pub frames: JobFrames,
    /// Where frames are saved; `None`: the document's Render Settings' file.
    pub output: Option<String>,
    pub status: JobStatus,
    /// What happened: frames saved, the error, missing pictures.
    pub message: String,
    /// Frames saved, of how many.
    pub done: u32,
    pub total: u32,
}

impl QueueJob {
    /// A Ready job for the document at `path`.
    #[must_use]
    pub fn new(path: PathBuf, take: JobTake) -> Self {
        Self {
            enabled: true,
            path,
            take,
            frames: JobFrames::FromDocument,
            output: None,
            status: JobStatus::Ready,
            message: String::new(),
            done: 0,
            total: 0,
        }
    }

    /// The file's name, as the list shows it.
    #[must_use]
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map_or_else(|| self.path.display().to_string(), |n| n.to_string_lossy().into_owned())
    }

    /// Back to Ready (Reset).
    pub fn reset(&mut self) {
        self.status = JobStatus::Ready;
        self.message.clear();
        self.done = 0;
        self.total = 0;
    }
}

/// A job's document as the loader made it ready.
struct Loaded {
    doc: Document,
    pictures: Pictures,
    replay: Option<Replay>,
    /// The camera object renders look through.
    camera: Option<NodeId>,
    /// The frames still to render (their files missing), and whether file
    /// names carry frame numbers.
    frames: Vec<i32>,
    numbered: bool,
    /// Frames already saved (skipped), of all the range's.
    skipped: u32,
    total: u32,
    /// Pictures that could not be read.
    missing: Vec<String>,
}

/// The job running, by its place in the list.
enum Running {
    Loading {
        index: usize,
        loaded: Receiver<Result<Loaded, String>>,
    },
    Rendering {
        index: usize,
        /// The render, in the Picture Viewer's history.
        number: u32,
        skipped: u32,
        missing: Vec<String>,
    },
}

/// The queue: its jobs and the one running.
#[derive(Default)]
pub struct RenderQueue {
    pub jobs: Vec<QueueJob>,
    running: Option<Running>,
    /// Start was pressed: after a job, the next enabled one starts.
    started: bool,
}

impl std::fmt::Debug for RenderQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderQueue")
            .field("jobs", &self.jobs.len())
            .field("running", &self.running_index())
            .field("started", &self.started)
            .finish()
    }
}

impl RenderQueue {
    /// The queue as the app left it: a job that was rendering is Stopped.
    #[must_use]
    pub fn restored(mut jobs: Vec<QueueJob>) -> Self {
        for job in &mut jobs {
            if job.status == JobStatus::Rendering {
                job.status = JobStatus::Stopped;
            }
        }
        Self {
            jobs,
            running: None,
            started: false,
        }
    }

    /// The job list to keep (a job rendering is kept as Stopped).
    #[must_use]
    pub fn saved(&self) -> Vec<QueueJob> {
        Self::restored(self.jobs.clone()).jobs
    }

    /// Whether a job is loading or rendering.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// The place of the job running.
    #[must_use]
    pub fn running_index(&self) -> Option<usize> {
        match &self.running {
            Some(Running::Loading { index, .. } | Running::Rendering { index, .. }) => Some(*index),
            None => None,
        }
    }

    /// The render of the job running, in the Picture Viewer's history.
    #[must_use]
    pub fn running_render(&self) -> Option<u32> {
        match &self.running {
            Some(Running::Rendering { number, .. }) => Some(*number),
            _ => None,
        }
    }

    /// Renders the enabled jobs from the top that are not finished.
    pub fn start(&mut self) {
        self.started = true;
    }

    /// Stops the job running (its saved frames stay) and the queue.
    pub fn stop(&mut self, renders: &mut Renders) {
        self.started = false;
        let Some(running) = self.running.take() else {
            return;
        };
        let index = match running {
            Running::Loading { index, .. } => index,
            Running::Rendering { index, number, .. } => {
                if renders.running_number() == Some(number) {
                    renders.stop();
                }
                index
            }
        };
        if let Some(job) = self.jobs.get_mut(index) {
            job.status = JobStatus::Stopped;
        }
    }

    /// Removes job `index` (stopping it if it runs).
    pub fn remove(&mut self, index: usize, renders: &mut Renders) {
        if self.running_index() == Some(index) {
            let started = self.started;
            self.stop(renders);
            self.started = started;
        }
        if index < self.jobs.len() {
            self.jobs.remove(index);
            self.shift(index, |i| i - 1);
        }
    }

    /// Moves job `index` one place up (`up`) or down.
    pub fn move_job(&mut self, index: usize, up: bool) {
        let other = if up { index.checked_sub(1) } else { Some(index + 1) };
        let Some(other) = other.filter(|&o| o < self.jobs.len()) else {
            return;
        };
        self.jobs.swap(index, other);
        let swap = |i: usize| {
            if i == index {
                other
            } else if i == other {
                index
            } else {
                i
            }
        };
        if let Some(Running::Loading { index: at, .. } | Running::Rendering { index: at, .. }) = &mut self.running {
            *at = swap(*at);
        }
    }

    /// A copy of job `index` below it, Ready.
    pub fn duplicate(&mut self, index: usize) {
        if let Some(job) = self.jobs.get(index) {
            let mut copy = job.clone();
            copy.reset();
            self.jobs.insert(index + 1, copy);
            self.shift(index + 1, |i| i + 1);
        }
    }

    /// Keeps the running job's place right after a removal or insertion at
    /// `from`.
    fn shift(&mut self, from: usize, by: impl Fn(usize) -> usize) {
        if let Some(Running::Loading { index, .. } | Running::Rendering { index, .. }) = &mut self.running
            && *index >= from
            && *index > 0
        {
            *index = by(*index);
        }
    }

    /// Moves the queue on (once a frame): a loaded job starts rendering, a
    /// finished render ends its job, the next job starts. Returns whether
    /// anything changed.
    pub fn update(&mut self, renders: &mut Renders) -> bool {
        let mut changed = false;
        match &self.running {
            Some(Running::Loading { index, loaded }) => {
                let index = *index;
                match loaded.try_recv() {
                    Ok(result) => {
                        self.running = None;
                        changed = true;
                        self.begin(index, result, renders);
                    }
                    Err(TryRecvError::Empty) => {}
                    Err(TryRecvError::Disconnected) => {
                        self.running = None;
                        changed = true;
                        self.fail(index, key("The loader stopped unexpectedly").to_owned());
                    }
                }
            }
            Some(Running::Rendering {
                index,
                number,
                skipped,
                missing,
            }) => {
                let (index, number, skipped) = (*index, *number, *skipped);
                if let Some(picture) = renders.picture(number) {
                    let ended = picture.state != RenderState::Rendering;
                    let saved = skipped + picture.saved;
                    let note = picture.note.clone();
                    let missing = missing.clone();
                    if let Some(job) = self.jobs.get_mut(index) {
                        if job.done != saved {
                            job.done = saved;
                            changed = true;
                        }
                        if ended {
                            job.status = if picture.state == RenderState::Finished {
                                JobStatus::Finished
                            } else {
                                JobStatus::Stopped
                            };
                            job.message = message(saved, job.total, &note, &missing);
                        }
                    }
                    if ended {
                        self.running = None;
                        changed = true;
                    }
                } else {
                    // The render left the history (it should not): the job stops.
                    self.running = None;
                    if let Some(job) = self.jobs.get_mut(index) {
                        job.status = JobStatus::Stopped;
                    }
                    changed = true;
                }
            }
            None => {}
        }
        if self.running.is_none() && self.started {
            match self.next_job() {
                Some(index) => {
                    self.load(index);
                    changed = true;
                }
                None => self.started = false,
            }
        }
        changed
    }

    /// The first enabled job still to render.
    fn next_job(&self) -> Option<usize> {
        self.jobs
            .iter()
            .position(|job| job.enabled && matches!(job.status, JobStatus::Ready | JobStatus::Stopped))
    }

    /// Starts reading job `index`'s file on a loader thread.
    fn load(&mut self, index: usize) {
        let Some(job) = self.jobs.get_mut(index) else {
            return;
        };
        // A stopped job goes on from its first missing frame; a Ready one
        // renders them all (over files from an earlier run).
        let continuing = job.status == JobStatus::Stopped;
        job.status = JobStatus::Rendering;
        job.message = key("Reading the document…").to_owned();
        let (sender, loaded) = mpsc::channel();
        let job = job.clone();
        let spawned = std::thread::Builder::new()
            .name("grid4d-queue-loader".into())
            .spawn(move || {
                let _ = sender.send(prepare(&job, continuing));
            });
        match spawned {
            Ok(_) => self.running = Some(Running::Loading { index, loaded }),
            Err(err) => self.fail(index, err.to_string()),
        }
    }

    /// Renders job `index` from what the loader made of it.
    fn begin(&mut self, index: usize, result: Result<Loaded, String>, renders: &mut Renders) {
        let loaded = match result {
            Ok(loaded) => loaded,
            Err(message) => return self.fail(index, message),
        };
        let Some(job) = self.jobs.get_mut(index) else {
            return;
        };
        job.total = loaded.total;
        job.done = loaded.skipped;
        if loaded.frames.is_empty() {
            job.status = JobStatus::Finished;
            job.message = message(loaded.skipped, loaded.total, "", &loaded.missing);
            return;
        }
        let number = renders.start_frames(
            &loaded.doc,
            loaded.pictures,
            (&grid4d_render::OrbitCamera::default(), loaded.camera),
            Some(&job.path),
            loaded.replay,
            (loaded.frames, loaded.numbered),
        );
        job.message = trf("Rendering · {0} of {1} frames saved", &[&job.done, &job.total]);
        self.running = Some(Running::Rendering {
            index,
            number,
            skipped: loaded.skipped,
            missing: loaded.missing,
        });
    }

    fn fail(&mut self, index: usize, message: String) {
        if let Some(job) = self.jobs.get_mut(index) {
            job.status = JobStatus::Error;
            job.message = message;
        }
    }
}

/// A job's line under its name: frames saved, how the render ended, the
/// pictures that were missing.
fn message(saved: u32, total: u32, note: &str, missing: &[String]) -> String {
    let mut out = trf("{0} of {1} frames saved", &[&saved, &total]);
    if note.starts_with(
        crate::i18n::tr("Not saved: {0}")
            .split("{0}")
            .next()
            .unwrap_or_default(),
    ) {
        out.push_str(" · ");
        out.push_str(note);
    }
    if !missing.is_empty() {
        out.push_str(" · ");
        out.push_str(&trf("missing pictures: {0}", &[&missing.join(", ")]));
    }
    out
}

/// Reads `job`'s document and makes it ready to render (on the loader);
/// `continuing`: frames whose file exists are left out.
fn prepare(job: &QueueJob, continuing: bool) -> Result<Loaded, String> {
    let mut doc = grid4d_io::load(&job.path).map_err(|err| err.to_string())?;
    // The take, as Render ▸ Render Take renders it.
    let take = match &job.take {
        JobTake::RenderTake => doc.takes.take_to_render(),
        JobTake::Main => None,
        JobTake::Named(name) => Some(
            doc.takes
                .iter()
                .find(|(_, take)| take.name == *name)
                .map(|(id, _)| id)
                .ok_or_else(|| trf("The document has no take named {0}", &[name]))?,
        ),
    };
    if take != doc.takes.active() {
        doc.switch_take(take);
    }
    // The take's camera, else the first camera object.
    let camera = take
        .and_then(|t| doc.takes.get(t))
        .and_then(|t| t.camera)
        .filter(|&c| doc.scene.contains(c))
        .or_else(|| first_camera(&doc));
    if let JobFrames::Manual { from, to, step } = job.frames {
        doc.render.frames = FrameRange::Manual {
            from: from.min(to),
            to: from.max(to),
        };
        doc.render.frame_step = step.max(1);
    }
    if let Some(output) = &job.output {
        doc.render.output.clone_from(output);
        doc.render.save = true;
    }
    let settings = doc.render.clone().validated();
    let range: Vec<i32> = settings.frames(&doc.time).collect();
    let numbered = range.len() > 1;
    let output = match save_target(&settings, Some(&job.path), numbered) {
        SaveTarget::File(output) => output,
        SaveTarget::Nowhere(why) => return Err(crate::i18n::tr(why).to_owned()),
    };
    // Frames whose file exists are done (a stopped job goes on from there).
    let frames: Vec<i32> = range
        .iter()
        .copied()
        .filter(|&frame| !continuing || output.file(frame).is_none_or(|path| !path.exists()))
        .collect();
    let total = u32::try_from(range.len()).unwrap_or(u32::MAX);
    let skipped = total - u32::try_from(frames.len()).unwrap_or(total);
    let (pictures, missing) = crate::textures::decode_all(doc.scene.textures());
    let simulation = Simulation::of(&doc);
    let replay =
        (!simulation.cache().bodies().is_empty()).then(|| Replay::new(doc.clone(), simulation.cache().clone()));
    Ok(Loaded {
        doc,
        pictures,
        replay,
        camera,
        frames,
        numbered,
        skipped,
        total,
        missing,
    })
}

/// The first camera object, in the Object Manager's order.
fn first_camera(doc: &Document) -> Option<NodeId> {
    let mut found = None;
    doc.scene.walk(&mut Vec::new(), |id, node, _, _| {
        if found.is_none() && matches!(node.data.kind, ObjectKind::Camera { .. }) {
            found = Some(id);
        }
        found.is_none()
    });
    found
}

/// Whether `path` is a document the queue can render (by its extension).
#[must_use]
pub fn is_document(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("g4d") || e.eq_ignore_ascii_case("oc4d"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test code")]

    use std::time::{Duration, Instant};

    use grid4d_scene::{RenderSettings, Take, Takes};

    use super::*;
    use crate::{EditorCommand, EditorState, ViewportPanel, commands::apply_pending};

    /// A fresh folder for one test.
    fn folder(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("grid4d-queue-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The demo scene, tiny and fast, saving to `output` (`frames` frames).
    fn small(output: &str, frames: i32) -> Document {
        let mut doc = Document::with_demo_scene();
        doc.render.width = 16;
        doc.render.height = 9;
        doc.render.samples = 1;
        doc.render.output = output.to_owned();
        doc.render.frames = FrameRange::Manual {
            from: 0,
            to: frames - 1,
        };
        doc
    }

    /// `doc`'s render settings saving to `output`, `frames` frames.
    fn settings_with(doc: &Document, output: &str, frames: i32) -> RenderSettings {
        let mut settings = doc.render.clone();
        settings.output = output.to_owned();
        settings.frames = FrameRange::Manual {
            from: 0,
            to: frames - 1,
        };
        settings
    }

    /// Runs the editor's frames until the queue stops (at most a while).
    fn run(doc: &mut Document, editor: &mut EditorState) {
        let mut viewport = ViewportPanel::new(None);
        let deadline = Instant::now() + Duration::from_secs(30);
        apply_pending(doc, editor, &mut viewport);
        while editor.render_queue.is_running() || editor.render_queue.started {
            assert!(Instant::now() < deadline, "the queue did not end");
            std::thread::sleep(Duration::from_millis(2));
            apply_pending(doc, editor, &mut viewport);
        }
    }

    #[test]
    fn two_takes_render_one_after_another_and_a_broken_file_is_an_error() {
        let dir = folder("takes");
        // Two takes with render settings of their own: each saves elsewhere.
        let mut doc = small("renders/main", 1);
        let mut takes = Takes::default();
        for (name, output) in [("Wide", "renders/wide"), ("Close", "renders/close")] {
            let mut take = Take::new(name);
            take.render = Some(settings_with(&doc, output, 2));
            takes.insert(take);
        }
        doc.takes = takes;
        let path = dir.join("shot.g4d");
        grid4d_io::save(&doc, &path).unwrap();
        std::fs::write(dir.join("broken.g4d"), b"not a document").unwrap();

        let mut editor = EditorState::default();
        let mut current = Document::new("other");
        let queue = &mut editor.render_queue;
        queue
            .jobs
            .push(QueueJob::new(dir.join("broken.g4d"), JobTake::RenderTake));
        queue
            .jobs
            .push(QueueJob::new(path.clone(), JobTake::Named("Wide".to_owned())));
        queue
            .jobs
            .push(QueueJob::new(path.clone(), JobTake::Named("Close".to_owned())));
        let mut skipped = QueueJob::new(path.clone(), JobTake::Main);
        skipped.enabled = false;
        queue.jobs.push(skipped);
        queue
            .jobs
            .push(QueueJob::new(path, JobTake::Named("Nowhere".to_owned())));
        editor.commands.push(EditorCommand::StartRenderQueue);
        run(&mut current, &mut editor);

        let jobs = &editor.render_queue.jobs;
        let statuses: Vec<JobStatus> = jobs.iter().map(|job| job.status).collect();
        assert_eq!(
            statuses,
            [
                JobStatus::Error,
                JobStatus::Finished,
                JobStatus::Finished,
                JobStatus::Ready,
                JobStatus::Error
            ],
            "{:?}",
            jobs.iter().map(|job| &job.message).collect::<Vec<_>>()
        );
        assert!(jobs[4].message.contains("Nowhere"), "{}", jobs[4].message);
        assert_eq!((jobs[1].done, jobs[1].total), (2, 2));
        for name in ["wide0000.png", "wide0001.png", "close0000.png", "close0001.png"] {
            assert!(dir.join("renders").join(name).exists(), "{name}");
        }
        assert!(
            !dir.join("renders").join("main.png").exists(),
            "the disabled job did not run"
        );
        // The renders are in the Picture Viewer's history.
        assert_eq!(editor.renders.history().len(), 2);
    }

    #[test]
    fn a_stopped_job_goes_on_from_its_first_missing_frame() {
        let dir = folder("continue");
        let path = dir.join("anim.g4d");
        grid4d_io::save(&small("out/f", 4), &path).unwrap();
        // Frames 0 and 1 were saved before the job stopped.
        std::fs::create_dir_all(dir.join("out")).unwrap();
        for frame in ["f0000.png", "f0001.png"] {
            std::fs::write(dir.join("out").join(frame), b"kept").unwrap();
        }
        let mut editor = EditorState::default();
        let mut job = QueueJob::new(path.clone(), JobTake::RenderTake);
        job.status = JobStatus::Stopped;
        editor.render_queue.jobs.push(job);
        // A Ready job renders every frame of its range.
        let mut again = QueueJob::new(path, JobTake::RenderTake);
        again.frames = JobFrames::Manual {
            from: 3,
            to: 3,
            step: 1,
        };
        editor.render_queue.jobs.push(again);
        editor.render_queue.start();
        run(&mut Document::new("other"), &mut editor);
        let jobs = &editor.render_queue.jobs;
        assert_eq!(jobs[0].status, JobStatus::Finished, "{}", jobs[0].message);
        assert_eq!((jobs[0].done, jobs[0].total), (4, 4));
        for frame in ["f0000.png", "f0001.png"] {
            assert_eq!(std::fs::read(dir.join("out").join(frame)).unwrap(), b"kept");
        }
        assert!(dir.join("out").join("f0003.png").exists());
        // The second job's one frame is named alone (not numbered).
        assert_eq!(jobs[1].status, JobStatus::Finished, "{}", jobs[1].message);
        assert!(dir.join("out").join("f.png").exists());
    }

    #[test]
    fn the_queue_is_kept_without_its_running_state_and_shift_r_waits() {
        let mut queue = RenderQueue::default();
        let mut job = QueueJob::new(PathBuf::from("a.g4d"), JobTake::Named("Wide".to_owned()));
        job.status = JobStatus::Rendering;
        job.output = Some("renders/x".to_owned());
        queue.jobs.push(job);
        queue.jobs.push(QueueJob::new(PathBuf::from("b.g4d"), JobTake::Main));
        let text = serde_json::to_string(&queue.saved()).unwrap();
        let back = RenderQueue::restored(serde_json::from_str(&text).unwrap());
        assert_eq!(
            back.jobs[0].status,
            JobStatus::Stopped,
            "a job rendering comes back Stopped"
        );
        assert_eq!(back.jobs[0].output.as_deref(), Some("renders/x"));
        assert_eq!(back.jobs[1], queue.jobs[1]);
        assert!(!back.is_running());

        // While a job runs, Shift+R says so and renders nothing.
        let dir = folder("busy");
        let path = dir.join("long.g4d");
        let mut doc = small("r/long", 1);
        doc.render.samples = 4096;
        doc.render.width = 320;
        doc.render.height = 180;
        grid4d_io::save(&doc, &path).unwrap();
        let mut editor = EditorState::default();
        editor.render_queue.jobs.push(QueueJob::new(path, JobTake::RenderTake));
        editor.render_queue.start();
        let mut viewport = ViewportPanel::new(None);
        let mut current = Document::with_demo_scene();
        let deadline = Instant::now() + Duration::from_secs(20);
        while editor.render_queue.running_render().is_none() {
            assert!(Instant::now() < deadline);
            apply_pending(&mut current, &mut editor, &mut viewport);
            std::thread::sleep(Duration::from_millis(2));
        }
        let renders = editor.renders.history().len();
        editor.commands.push(EditorCommand::RenderToPictureViewer);
        apply_pending(&mut current, &mut editor, &mut viewport);
        assert!(editor.file.alert.is_some());
        assert_eq!(editor.renders.history().len(), renders, "no new render");
        // Stop: the job is Stopped and the queue idle.
        editor.commands.push(EditorCommand::StopRenderQueue);
        apply_pending(&mut current, &mut editor, &mut viewport);
        assert_eq!(editor.render_queue.jobs[0].status, JobStatus::Stopped);
        assert!(!editor.render_queue.is_running());
        assert!(editor.renders.running_number().is_none());
    }

    #[test]
    fn jobs_move_duplicate_and_go() {
        let mut queue = RenderQueue::default();
        for name in ["a", "b", "c"] {
            queue
                .jobs
                .push(QueueJob::new(PathBuf::from(format!("{name}.g4d")), JobTake::RenderTake));
        }
        queue.move_job(0, false);
        queue.move_job(2, true);
        let names: Vec<String> = queue.jobs.iter().map(QueueJob::name).collect();
        assert_eq!(names, ["b.g4d", "c.g4d", "a.g4d"]);
        queue.jobs[0].status = JobStatus::Finished;
        queue.duplicate(0);
        assert_eq!(queue.jobs[1].status, JobStatus::Ready, "a copy starts Ready");
        let mut renders = Renders::default();
        queue.remove(0, &mut renders);
        assert_eq!(queue.jobs.len(), 3);
        assert!(is_document(Path::new("x.G4D")) && !is_document(Path::new("x.png")));
    }
}
