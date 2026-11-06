//! The document's texture pictures, loaded in the background (see
//! `docs/reference/material-textures.md`): one worker thread reads and decodes
//! them (`grid4d_io::decode_texture`, mip-maps included) while the interface
//! keeps drawing. [`TextureCache::update`] runs once a frame (from
//! `commands::apply_pending`): it starts loads for new or changed textures,
//! notices files that changed, appeared or vanished on disk (checked at most
//! once a second each), and forgets textures the document no longer has.
//! Loading never changes the document.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

use grid4d_io::TextureImage;
use grid4d_scene::{ColorSpace, Texture, TextureId, TextureSource, Textures};

/// How often a linked file is checked for changes.
const CHECK_EVERY: Duration = Duration::from_secs(1);

/// What the cache knows of a texture's picture.
#[derive(Clone, Copy, Debug)]
pub enum Status<'a> {
    /// Not there yet (being read and decoded).
    Loading,
    /// Decoded; `version` changes whenever the picture does.
    Ready { image: &'a TextureImage, version: u64 },
    /// A linked file that does not exist.
    Missing,
    /// The file exists but cannot be read as a picture.
    Failed(&'a str),
}

/// Where a load reads from.
#[derive(Clone, Debug)]
enum Input {
    File(PathBuf),
    Bytes(Arc<[u8]>),
}

impl Input {
    fn of(source: &TextureSource) -> Self {
        match source {
            TextureSource::Linked(path) => Self::File(path.clone()),
            TextureSource::Embedded { bytes, .. } => Self::Bytes(Arc::clone(bytes)),
        }
    }

    /// `true` if this is still where `source` reads from (no allocation).
    fn matches(&self, source: &TextureSource) -> bool {
        match (self, source) {
            (Self::File(a), TextureSource::Linked(b)) => a == b,
            (Self::Bytes(a), TextureSource::Embedded { bytes, .. }) => Arc::ptr_eq(a, bytes),
            (Self::File(_), TextureSource::Embedded { .. }) | (Self::Bytes(_), TextureSource::Linked(_)) => false,
        }
    }
}

#[derive(Debug)]
enum State {
    Loading,
    Ready(Arc<TextureImage>),
    Missing,
    Failed(String),
}

#[derive(Debug)]
struct Entry {
    input: Input,
    color_space: ColorSpace,
    state: State,
    /// The file's modification time when it was last read (`None`: no file).
    modified: Option<SystemTime>,
    checked: Instant,
    /// Bumped with every result, so views can tell a new picture.
    version: u64,
    /// The latest load asked for, to ignore results of older ones.
    ticket: u64,
    /// The load whose result is shown (≠ `ticket`: one is in flight).
    shown: u64,
    used: bool,
}

struct Job {
    id: TextureId,
    ticket: u64,
    input: Input,
    color_space: ColorSpace,
}

struct Done {
    id: TextureId,
    ticket: u64,
    modified: Option<SystemTime>,
    state: State,
}

#[derive(Debug)]
struct Worker {
    jobs: Sender<Job>,
    done: Receiver<Done>,
}

impl std::fmt::Debug for Job {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Job").field("id", &self.id).finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Done {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Done").field("id", &self.id).finish_non_exhaustive()
    }
}

/// The decoded pictures of the document's textures (editor state, never saved).
#[derive(Debug, Default)]
pub struct TextureCache {
    entries: HashMap<TextureId, Entry>,
    worker: Option<Worker>,
    tickets: u64,
}

impl TextureCache {
    /// Takes finished loads, starts new ones and forgets textures that are
    /// gone. Returns `true` while something is loading (keep repainting).
    pub fn update(&mut self, textures: &Textures) -> bool {
        self.receive();
        for entry in self.entries.values_mut() {
            entry.used = false;
        }
        let now = Instant::now();
        for (id, texture) in textures.iter() {
            let current = self
                .entries
                .get(&id)
                .is_some_and(|e| e.input.matches(&texture.source) && e.color_space == texture.color_space);
            if !current {
                self.start(id, texture, now);
                continue;
            }
            let Some(entry) = self.entries.get_mut(&id) else {
                continue;
            };
            entry.used = true;
            // A linked file is looked at again now and then: changed,
            // appeared or vanished, it is read again.
            let mut changed = false;
            if let Input::File(path) = &entry.input
                && !matches!(entry.state, State::Loading)
                && now.duration_since(entry.checked) >= CHECK_EVERY
            {
                entry.checked = now;
                let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
                changed = modified != entry.modified;
            }
            if changed {
                self.start(id, texture, now);
            }
        }
        self.entries.retain(|_, entry| entry.used);
        self.entries.values().any(|e| e.shown != e.ticket)
    }

    /// The picture of `texture` (`id` in the document), as far as it is known.
    #[must_use]
    pub fn status(&self, id: TextureId, texture: &Texture) -> Status<'_> {
        match self.entries.get(&id) {
            Some(entry) if entry.input.matches(&texture.source) && entry.color_space == texture.color_space => {
                match &entry.state {
                    State::Loading => Status::Loading,
                    State::Ready(image) => Status::Ready {
                        image,
                        version: entry.version,
                    },
                    State::Missing => Status::Missing,
                    State::Failed(message) => Status::Failed(message),
                }
            }
            _ => Status::Loading,
        }
    }

    /// How many of `textures` are linked to files that do not exist.
    #[must_use]
    pub fn missing(&self, textures: &Textures) -> usize {
        textures
            .iter()
            .filter(|&(id, texture)| matches!(self.status(id, texture), Status::Missing))
            .count()
    }

    /// Asks the worker to load `texture` (a fresh ticket: older results are
    /// ignored); the entry keeps its last picture until the new one arrives.
    fn start(&mut self, id: TextureId, texture: &Texture, now: Instant) {
        self.tickets += 1;
        let ticket = self.tickets;
        let input = Input::of(&texture.source);
        let job = Job {
            id,
            ticket,
            input: input.clone(),
            color_space: texture.color_space,
        };
        let same = self
            .entries
            .get(&id)
            .is_some_and(|e| e.input.matches(&texture.source) && e.color_space == texture.color_space);
        let entry = self.entries.entry(id).or_insert_with(|| Entry {
            input: input.clone(),
            color_space: texture.color_space,
            state: State::Loading,
            modified: None,
            checked: now,
            version: 0,
            ticket,
            shown: 0,
            used: true,
        });
        if !same {
            // Another file or colour space: what was shown no longer applies.
            entry.input = input;
            entry.color_space = texture.color_space;
            entry.state = State::Loading;
        }
        entry.ticket = ticket;
        entry.checked = now;
        entry.used = true;
        if let Some(job) = self.send(job) {
            // No worker thread: load here (slower, but never stuck).
            let done = load(job);
            self.finish(done);
        }
    }

    /// Hands `job` to the worker (started on first use); gives it back if
    /// there is no worker.
    fn send(&mut self, job: Job) -> Option<Job> {
        if self.worker.is_none() {
            self.worker = spawn();
        }
        let Some(worker) = &self.worker else {
            return Some(job);
        };
        match worker.jobs.send(job) {
            Ok(()) => None,
            Err(mpsc::SendError(job)) => {
                self.worker = None;
                Some(job)
            }
        }
    }

    fn receive(&mut self) {
        let mut finished = Vec::new();
        if let Some(worker) = &self.worker {
            while let Ok(done) = worker.done.try_recv() {
                finished.push(done);
            }
        }
        for done in finished {
            self.finish(done);
        }
    }

    fn finish(&mut self, done: Done) {
        let Some(entry) = self.entries.get_mut(&done.id) else {
            return;
        };
        if entry.ticket != done.ticket {
            return; // an older load of something since changed
        }
        entry.state = done.state;
        entry.modified = done.modified;
        entry.shown = done.ticket;
        entry.version += 1;
    }

    /// Waits (up to ten seconds) until nothing is loading: for tests.
    #[cfg(test)]
    pub(crate) fn wait(&mut self, textures: &Textures) {
        let start = Instant::now();
        while self.update(textures) && start.elapsed() < Duration::from_secs(10) {
            thread::sleep(Duration::from_millis(2));
        }
    }
}

/// The worker thread: loads jobs one after another until the cache is dropped.
fn spawn() -> Option<Worker> {
    let (jobs, inbox) = mpsc::channel::<Job>();
    let (outbox, done) = mpsc::channel::<Done>();
    thread::Builder::new()
        .name("grid4d-textures".to_owned())
        .spawn(move || {
            while let Ok(job) = inbox.recv() {
                if outbox.send(load(job)).is_err() {
                    break;
                }
            }
        })
        .map_err(|err| log::warn!("texture loader not started: {err}"))
        .ok()?;
    Some(Worker { jobs, done })
}

/// Reads and decodes one picture.
fn load(job: Job) -> Done {
    let Job {
        id,
        ticket,
        input,
        color_space,
    } = job;
    let (modified, bytes) = match &input {
        Input::Bytes(bytes) => (None, Ok(Arc::clone(bytes))),
        Input::File(path) => {
            let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
            (modified, grid4d_io::read_file(path).map(Arc::<[u8]>::from))
        }
    };
    let state = match bytes {
        Err(grid4d_io::IoError::Io(err)) if err.kind() == std::io::ErrorKind::NotFound => State::Missing,
        Err(err) => State::Failed(err.to_string()),
        Ok(bytes) => match grid4d_io::decode_texture(&bytes, color_space) {
            Ok(image) => State::Ready(Arc::new(image)),
            Err(err) => State::Failed(err.to_string()),
        },
    };
    Done {
        id,
        ticket,
        modified,
        state,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test code")]

    use super::*;

    /// A fresh folder under the system's temp folder for one test.
    fn folder(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("grid4d-ui-textures-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A `width × 1` PNG.
    pub(crate) fn png(width: u32) -> Vec<u8> {
        let pixels = vec![200u8; usize::try_from(width).unwrap() * 4];
        grid4d_io::png_bytes(width, 1, grid4d_io::Channels::Rgba, &pixels).unwrap()
    }

    #[test]
    fn pictures_load_in_the_background_and_missing_files_are_told() {
        let dir = folder("load");
        let path = dir.join("wood.png");
        std::fs::write(&path, png(4)).unwrap();
        let mut textures = Textures::default();
        let wood = textures.insert(Texture::linked(path));
        let gone = textures.insert(Texture::linked(dir.join("gone.png")));
        let broken = textures.insert(Texture::linked(dir.join("broken.png")));
        std::fs::write(dir.join("broken.png"), b"not a picture").unwrap();
        let logo = textures.insert(Texture {
            name: "Logo".to_owned(),
            source: TextureSource::Embedded {
                file_name: "logo.png".to_owned(),
                bytes: png(2).into(),
            },
            color_space: ColorSpace::Srgb,
        });

        let mut cache = TextureCache::default();
        let status = |cache: &TextureCache, textures: &Textures, id| {
            let texture = textures.get(id).unwrap();
            match cache.status(id, texture) {
                Status::Ready { image, .. } => format!("ready {}", image.size().0),
                Status::Loading => "loading".to_owned(),
                Status::Missing => "missing".to_owned(),
                Status::Failed(_) => "failed".to_owned(),
            }
        };
        cache.wait(&textures);
        assert_eq!(status(&cache, &textures, wood), "ready 4");
        assert_eq!(status(&cache, &textures, gone), "missing");
        assert_eq!(status(&cache, &textures, broken), "failed");
        assert_eq!(status(&cache, &textures, logo), "ready 2");
        assert_eq!(cache.missing(&textures), 1);
        assert!(!cache.update(&textures), "nothing left to load");

        // The missing file appears: seen at the next check.
        std::fs::write(dir.join("gone.png"), png(8)).unwrap();
        cache.entries.get_mut(&gone).unwrap().checked -= CHECK_EVERY;
        assert!(cache.update(&textures), "loading again");
        cache.wait(&textures);
        assert_eq!(status(&cache, &textures, gone), "ready 8");

        // Relinked: the new file is loaded; removed: forgotten.
        textures.get_mut(wood).unwrap().source = TextureSource::Linked(dir.join("gone.png"));
        cache.wait(&textures);
        assert_eq!(status(&cache, &textures, wood), "ready 8");
        textures.remove(logo);
        cache.update(&textures);
        assert!(!cache.entries.contains_key(&logo));
        let _ = std::fs::remove_dir_all(dir);
    }
}
