//! Autosave and crash recovery: while the document has unsaved changes, a
//! recovery copy is written now and then into the user data folder; it goes
//! away when the changes are saved or Grid4D closes normally. A copy left
//! behind by a Grid4D that is no longer running is offered on the next start.
//! Spec: `docs/reference/workspace-preferences.md`.
//!
//! Each running Grid4D holds a lock on `<name>.lock` beside its copy
//! (`<name>.g4d`, with the document's own path in `<name>.txt`); the system
//! releases it when the process ends, however it ends, so a copy whose lock
//! can be taken is a crashed session's.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::SystemTime;

use grid4d_scene::Document;

use crate::{DocumentFile, History};

/// A recovery copy found at startup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recovery {
    /// The copy itself.
    pub copy: PathBuf,
    /// The file the document was saved to or opened from, if any.
    pub original: Option<PathBuf>,
    pub written: SystemTime,
}

/// The answer to "Recover Unsaved Work?".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryChoice {
    Recover,
    Discard,
}

/// This session's recovery copy and the copy offered at startup.
#[derive(Debug, Default)]
pub struct Autosave {
    /// The recovery folder; `None` writes nothing (tests, no data folder).
    dir: Option<PathBuf>,
    /// This session's file name, without extension.
    name: String,
    /// Held while this session runs (see the module documentation).
    lock: Option<File>,
    /// When the unsaved changes began, or the copy was last written (seconds).
    since: Option<f64>,
    /// The history state the copy holds ([`Self::UNKNOWN`] for a recovered
    /// copy); `None`: there is no copy.
    written: Option<u64>,
    writer: Option<JoinHandle<Result<(), grid4d_io::IoError>>>,
    /// A crashed session's newest copy, until it is recovered or discarded.
    pub offer: Option<Recovery>,
}

impl Autosave {
    const COPY: &'static str = "g4d";
    const ORIGINAL: &'static str = "txt";
    const LOCK: &'static str = "lock";
    /// The state of a copy recovered from a crash: no state of this session.
    const UNKNOWN: u64 = u64::MAX;

    /// Autosave into `dir` (created when first needed), offering the newest
    /// copy a crashed session left there.
    #[must_use]
    pub fn new(dir: PathBuf) -> Self {
        /// Sessions made by this process so far: two made within the same
        /// millisecond still get names of their own.
        static SESSIONS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let started = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let session = SESSIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let offer = newest_orphan(&dir);
        Self {
            dir: Some(dir),
            name: format!("{}-{started}-{session}", std::process::id()),
            offer,
            ..Self::default()
        }
    }

    /// This session's copy, if autosave has a folder.
    #[must_use]
    pub fn copy_path(&self) -> Option<PathBuf> {
        self.path(Self::COPY)
    }

    fn path(&self, extension: &str) -> Option<PathBuf> {
        Some(self.dir.as_ref()?.join(&self.name).with_extension(extension))
    }

    /// Called every frame: writes a copy when `interval` seconds of unsaved
    /// changes have passed since the last one (and the document changed since),
    /// removes it once there is nothing unsaved. `interval` `None`: autosave is
    /// off. Returns the seconds until a copy is due, to wake up for it.
    pub fn tick(
        &mut self,
        doc: &Document,
        history: &History,
        file: &DocumentFile,
        interval: Option<f64>,
        now: f64,
    ) -> Option<f64> {
        if self.writer.as_ref().is_some_and(JoinHandle::is_finished) {
            self.wait();
        }
        let (true, Some(interval)) = (file.is_dirty(history), interval) else {
            self.since = None;
            if self.written.is_some() {
                self.remove_copy();
            }
            return None;
        };
        let since = *self.since.get_or_insert(now);
        let due = since + interval - now;
        if due > 0.0 || self.writer.is_some() {
            return Some(due.max(1.0));
        }
        self.since = Some(now);
        let state = history.state_id();
        if self.written != Some(state) || self.copy_path().is_some_and(|copy| !copy.exists()) {
            self.write(doc, file.path(), state);
        }
        Some(interval)
    }

    /// Starts writing the copy in the background: to a temporary name, then
    /// renamed, so a crash while writing keeps the previous copy.
    fn write(&mut self, doc: &Document, original: Option<&Path>, state: u64) {
        let (Some(dir), Some(copy), Some(note)) = (self.dir.clone(), self.path(Self::COPY), self.path(Self::ORIGINAL))
        else {
            return;
        };
        if let Err(err) = fs::create_dir_all(&dir) {
            log::warn!("Autosave: no recovery folder {}: {err}", dir.display());
            return;
        }
        self.hold_lock();
        let doc = doc.clone();
        let original = original.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        self.written = Some(state);
        self.writer = Some(std::thread::spawn(move || {
            let part = copy.with_extension("part");
            grid4d_io::save(&doc, &part)?;
            fs::rename(&part, &copy)?;
            fs::write(&note, original)?;
            Ok(())
        }));
    }

    /// Locks this session's lock file (once), so other sessions leave its copy alone.
    fn hold_lock(&mut self) {
        let Some(path) = self.path(Self::LOCK).filter(|_| self.lock.is_none()) else {
            return;
        };
        match File::create(&path).and_then(|file| file.try_lock().map(|()| file).map_err(Into::into)) {
            Ok(file) => self.lock = Some(file),
            Err(err) => log::warn!("Autosave: could not lock {}: {err}", path.display()),
        }
    }

    /// Waits for a copy being written.
    pub fn wait(&mut self) {
        if let Some(writer) = self.writer.take() {
            match writer.join() {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    log::warn!("Autosave failed: {err}");
                    self.written = None;
                }
                Err(_) => self.written = None,
            }
        }
    }

    fn remove_copy(&mut self) {
        self.wait();
        self.written = None;
        for extension in [Self::COPY, Self::ORIGINAL] {
            if let Some(path) = self.path(extension) {
                let _ = fs::remove_file(path);
            }
        }
    }

    /// A normal exit: the copy is no longer needed.
    pub fn finish(&mut self) {
        self.remove_copy();
        if let (Some(lock), Some(path)) = (self.lock.take(), self.path(Self::LOCK)) {
            drop(lock);
            let _ = fs::remove_file(path);
        }
    }

    /// Answers the offer: the copy to open (`Recover`), and every crashed
    /// session's copies removed — the recovered one becomes this session's
    /// own copy, so it is still there should Grid4D stop again before the
    /// next autosave.
    pub fn resolve(&mut self, choice: RecoveryChoice) -> Option<Recovery> {
        let offer = self.offer.take()?;
        let dir = self.dir.clone()?;
        let crashed = orphans(&dir);
        let mut recovered = None;
        if choice == RecoveryChoice::Recover
            && let Some(own) = self.copy_path()
            && fs::rename(&offer.copy, &own).is_ok()
        {
            if let Some(original) = &offer.original {
                let _ = fs::write(
                    own.with_extension(Self::ORIGINAL),
                    original.to_string_lossy().as_bytes(),
                );
            }
            self.written = Some(Self::UNKNOWN);
            self.hold_lock();
            recovered = Some(Recovery { copy: own, ..offer });
        }
        for stem in crashed.iter().filter(|&stem| *stem != self.name) {
            for extension in [Self::COPY, Self::ORIGINAL, Self::LOCK] {
                let _ = fs::remove_file(dir.join(stem).with_extension(extension));
            }
        }
        recovered
    }
}

/// The names (without extension) of the copies in `dir` whose session is not
/// running.
fn orphans(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == Autosave::COPY))
        .filter(|path| {
            // A running session holds its lock; a missing lock file is no one's.
            File::open(path.with_extension(Autosave::LOCK)).map_or(true, |lock| lock.try_lock().is_ok())
        })
        .filter_map(|path| Some(path.file_stem()?.to_string_lossy().into_owned()))
        .collect()
}

/// The most recently written orphaned copy in `dir`.
fn newest_orphan(dir: &Path) -> Option<Recovery> {
    orphans(dir)
        .into_iter()
        .filter_map(|stem| {
            let copy = dir.join(&stem).with_extension(Autosave::COPY);
            let written = fs::metadata(&copy).and_then(|m| m.modified()).ok()?;
            let original = fs::read_to_string(copy.with_extension(Autosave::ORIGINAL))
                .ok()
                .filter(|text| !text.trim().is_empty())
                .map(|text| PathBuf::from(text.trim()));
            Some(Recovery {
                copy,
                original,
                written,
            })
        })
        .max_by_key(|recovery| recovery.written)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::time::Duration;

    use super::*;
    use crate::NativeDialogs;

    /// An empty folder of its own for a test.
    pub(crate) fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("grid4d-autosave-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_copy_is_written_only_while_there_are_unsaved_changes() {
        let dir = folder("dirty");
        let mut autosave = Autosave::new(dir.clone());
        assert_eq!(autosave.offer, None);
        let doc = Document::with_demo_scene();
        let history = History::default();
        let mut file = DocumentFile::with_dialogs(Box::new(NativeDialogs));
        let copy = autosave.copy_path().unwrap();

        // Clean: nothing to keep safe, however long it stays so.
        assert_eq!(autosave.tick(&doc, &history, &file, Some(60.0), 0.0), None);
        assert_eq!(autosave.tick(&doc, &history, &file, Some(60.0), 500.0), None);
        assert!(!copy.exists());

        // Unsaved changes: the copy is due a minute later.
        file.mark_unsaved();
        assert_eq!(autosave.tick(&doc, &history, &file, Some(60.0), 1000.0), Some(60.0));
        assert_eq!(autosave.tick(&doc, &history, &file, Some(60.0), 1030.0), Some(30.0));
        assert!(!copy.exists());
        autosave.tick(&doc, &history, &file, Some(60.0), 1060.0);
        autosave.wait();
        assert!(copy.exists(), "written after the interval");
        assert_eq!(grid4d_io::load(&copy).unwrap().name, doc.name);
        assert!(!copy.with_extension("part").exists());

        // The same state is not written again; a removed copy is.
        let written = fs::metadata(&copy).unwrap().modified().unwrap();
        std::thread::sleep(Duration::from_millis(20));
        autosave.tick(&doc, &history, &file, Some(60.0), 1120.0);
        autosave.wait();
        assert_eq!(fs::metadata(&copy).unwrap().modified().unwrap(), written);

        // Autosave off, or the changes saved: the copy goes.
        assert_eq!(autosave.tick(&doc, &history, &file, None, 1130.0), None);
        assert!(!copy.exists());
        autosave.tick(&doc, &history, &file, Some(60.0), 1140.0);
        autosave.tick(&doc, &history, &file, Some(60.0), 1200.0);
        autosave.wait();
        assert!(copy.exists());
        file.mark_saved(&history);
        autosave.tick(&doc, &history, &file, Some(60.0), 1210.0);
        assert!(!copy.exists() && !copy.with_extension("txt").exists());

        // This session's lock keeps its copy from being offered to another.
        file.mark_unsaved();
        autosave.tick(&doc, &history, &file, Some(60.0), 2000.0);
        autosave.tick(&doc, &history, &file, Some(60.0), 2060.0);
        autosave.wait();
        assert_eq!(Autosave::new(dir.clone()).offer, None, "the session is running");
        autosave.finish();
        assert!(!copy.exists() && !copy.with_extension("lock").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_crashed_sessions_newest_copy_is_offered_and_resolving_clears_them_all() {
        let dir = folder("crash");
        let older = Document::new("Older");
        let newer = Document::new("Newer");
        grid4d_io::save(&older, &dir.join("1-1.g4d")).unwrap();
        grid4d_io::save(&newer, &dir.join("2-2.g4d")).unwrap();
        fs::write(dir.join("2-2.txt"), "C:/work/shot.g4d").unwrap();
        // A lock file nobody holds: that session ended.
        File::create(dir.join("2-2.lock")).unwrap();
        let old_time = SystemTime::now() - Duration::from_secs(3600);
        File::options()
            .write(true)
            .open(dir.join("1-1.g4d"))
            .unwrap()
            .set_modified(old_time)
            .unwrap();

        let mut autosave = Autosave::new(dir.clone());
        let offer = autosave.offer.clone().unwrap();
        assert_eq!(offer.copy, dir.join("2-2.g4d"));
        assert_eq!(offer.original, Some(PathBuf::from("C:/work/shot.g4d")));

        let recovered = autosave.resolve(RecoveryChoice::Recover).unwrap();
        assert_eq!(recovered.copy, autosave.copy_path().unwrap(), "now this session's copy");
        assert_eq!(grid4d_io::load(&recovered.copy).unwrap().name, "Newer");
        assert_eq!(recovered.original, offer.original);
        let mut left: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .collect();
        left.sort();
        assert_eq!(
            left,
            vec![
                recovered.copy.clone(),
                recovered.copy.with_extension("lock"),
                recovered.copy.with_extension("txt"),
            ],
            "the other copies are gone"
        );
        assert_eq!(Autosave::new(dir.clone()).offer, None, "this session holds its copy");

        // Stopped again before writing: the recovered copy is offered next time.
        drop(autosave);
        let mut again = Autosave::new(dir.clone());
        assert_eq!(again.offer.as_ref().map(|o| &o.copy), Some(&recovered.copy));
        assert_eq!(again.resolve(RecoveryChoice::Discard), None);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0, "discarded");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn sessions_made_at_once_have_names_of_their_own() {
        let dir = folder("names");
        let names: std::collections::HashSet<String> = (0..50).map(|_| Autosave::new(dir.clone()).name).collect();
        assert_eq!(names.len(), 50, "even within one millisecond");
        let _ = fs::remove_dir_all(dir);
    }
}
