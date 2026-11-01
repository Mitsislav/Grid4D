//! Crash reports, kept on this computer only (spec
//! `docs/reference/app-release.md`): a panic hook writes a plain-text report
//! into the data folder's `crashes` folder, and the next start points to the
//! newest one not shown before. Nothing is sent anywhere.
//!
//! The binary installs the logger ([`install_log`], which keeps the last
//! lines for a report) and the hook ([`install`]) before anything else;
//! [`set_adapter`] adds the graphics adapter once it is known.

use std::backtrace::Backtrace;
use std::collections::VecDeque;
use std::fmt::Write as _;
use std::fs;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Instant, SystemTime};

use crate::BuildInfo;

/// Log lines a report keeps.
const LOG_LINES: usize = 200;
/// Reports kept in the folder; older ones go at start-up.
const KEEP: usize = 20;
const PREFIX: &str = "crash-";
const EXTENSION: &str = "txt";
/// Holds the name of the newest report pointed to so far.
const SHOWN: &str = "shown";

/// What a report says about the program, besides the panic.
struct Context {
    build: Option<BuildInfo>,
    adapter: String,
    dir: Option<PathBuf>,
}

static CONTEXT: Mutex<Context> = Mutex::new(Context {
    build: None,
    adapter: String::new(),
    dir: None,
});
static LOG: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Installs `inner` as the program's logger, keeping the last lines it
/// writes for a crash report. Once per process; later calls do nothing.
pub fn install_log(inner: Box<dyn log::Log>, max: log::LevelFilter) {
    if log::set_boxed_logger(Box::new(KeepingLog { inner })).is_ok() {
        log::set_max_level(max);
    }
}

/// Installs the panic hook: every panic, on any thread, writes a report into
/// `dir` (made when first needed), then the previous hook runs as before.
pub fn install(dir: PathBuf, build: BuildInfo) {
    {
        let mut context = lock(&CONTEXT);
        context.build = Some(build);
        context.dir = Some(dir);
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        on_panic(info);
        previous(info);
    }));
}

/// The graphics adapter, for reports written from now on.
pub fn set_adapter(adapter: &str) {
    adapter.clone_into(&mut lock(&CONTEXT).adapter);
}

/// Writes the report of the panic `info`. Never panics itself and never
/// waits for a lock (the panicking thread may hold one).
fn on_panic(info: &PanicHookInfo<'_>) {
    let Ok(context) = CONTEXT.try_lock() else { return };
    let Some(dir) = context.dir.clone() else { return };
    let thread = std::thread::current();
    let report = Report {
        build: context.build.unwrap_or_default(),
        adapter: context.adapter.clone(),
        thread: thread.name().unwrap_or("unnamed").to_owned(),
        message: info.payload_as_str().unwrap_or("(no message)").to_owned(),
        location: info.location().map(ToString::to_string).unwrap_or_default(),
        backtrace: Backtrace::force_capture().to_string(),
        log: LOG
            .try_lock()
            .map(|lines| lines.iter().cloned().collect())
            .unwrap_or_default(),
        time: SystemTime::now(),
    };
    drop(context);
    match report.write(&dir, std::process::id()) {
        Ok(path) => eprintln!("Grid4D: a crash report was written to {}", path.display()),
        Err(err) => eprintln!(
            "Grid4D: the crash report could not be written into {}: {err}",
            dir.display()
        ),
    }
}

/// A crash report's contents.
#[derive(Clone, Debug)]
pub struct Report {
    pub build: BuildInfo,
    pub adapter: String,
    pub thread: String,
    pub message: String,
    /// Where in the source: `file:line:column`.
    pub location: String,
    pub backtrace: String,
    /// The last lines logged, oldest first.
    pub log: Vec<String>,
    pub time: SystemTime,
}

impl Report {
    /// The report as written: a few labelled lines, then the backtrace and
    /// the log.
    #[must_use]
    pub fn text(&self) -> String {
        let build = &self.build;
        let mut text = String::from("Grid4D crash report\n\n");
        let _ = write!(text, "Version:  {}", build.version);
        if !build.commit.is_empty() {
            let _ = write!(text, " ({}, {})", build.commit, build.date);
        }
        let [year, month, day, hour, minute, second] = utc(self.time);
        let _ = writeln!(
            text,
            "\nTime:     {year}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02} UTC"
        );
        let _ = writeln!(
            text,
            "System:   {} {} ({})",
            std::env::consts::OS,
            std::env::consts::ARCH,
            std::env::consts::FAMILY
        );
        let adapter = if self.adapter.is_empty() {
            "unknown"
        } else {
            &self.adapter
        };
        let _ = writeln!(text, "Graphics: {adapter}");
        let _ = writeln!(text, "Thread:   {}", self.thread);
        let _ = writeln!(text, "At:       {}", self.location);
        let _ = writeln!(text, "\nMessage:\n{}", self.message);
        let _ = writeln!(text, "\nBacktrace:\n{}", self.backtrace.trim_end());
        let _ = writeln!(text, "\nLast log lines:");
        for line in &self.log {
            let _ = writeln!(text, "{line}");
        }
        text
    }

    /// The report's file name: `crash-<UTC time>-<process>.txt`, which sorts
    /// in the order reports were written.
    #[must_use]
    pub fn file_name(&self, process: u32) -> String {
        let [year, month, day, hour, minute, second] = utc(self.time);
        format!("{PREFIX}{year}-{month:02}-{day:02}T{hour:02}-{minute:02}-{second:02}-{process}.{EXTENSION}")
    }

    /// Writes the report into `dir` (made if missing); returns its path.
    ///
    /// # Errors
    /// When the folder cannot be made or the file written.
    pub fn write(&self, dir: &Path, process: u32) -> std::io::Result<PathBuf> {
        fs::create_dir_all(dir)?;
        let path = dir.join(self.file_name(process));
        fs::write(&path, self.text())?;
        Ok(path)
    }
}

/// The crash reports found at start-up. Lives in `EditorState::crash`.
#[derive(Debug, Default)]
pub struct CrashReports {
    /// The newest report not pointed to before, until the user has seen it.
    pub new_report: Option<PathBuf>,
}

impl CrashReports {
    /// Looks into `dir` (the folder [`install`] writes to): removes all but
    /// the newest reports, and takes the newest one not pointed to before,
    /// which counts as pointed to from now on.
    #[must_use]
    pub fn new(dir: &Path) -> Self {
        let mut names: Vec<String> = fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| name.starts_with(PREFIX) && name.ends_with(EXTENSION))
            .collect();
        names.sort_unstable();
        let old = names.len().saturating_sub(KEEP);
        for name in names.drain(..old) {
            let _ = fs::remove_file(dir.join(name));
        }
        let shown = fs::read_to_string(dir.join(SHOWN)).unwrap_or_default();
        let new_report = names
            .last()
            .filter(|&newest| newest.as_str() > shown.trim())
            .map(|newest| {
                if let Err(err) = fs::write(dir.join(SHOWN), newest) {
                    log::warn!("Crash reports: could not note {newest} as shown: {err}");
                }
                dir.join(newest)
            });
        Self { new_report }
    }
}

/// A logger that passes everything to `inner` and keeps the last lines it
/// writes.
struct KeepingLog {
    inner: Box<dyn log::Log>,
}

impl log::Log for KeepingLog {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        self.inner.enabled(metadata)
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.inner.enabled(record.metadata()) {
            return;
        }
        self.inner.log(record);
        static START: OnceLock<Instant> = OnceLock::new();
        let seconds = START.get_or_init(Instant::now).elapsed().as_secs_f64();
        let line = format!(
            "[{seconds:9.3}s {:5} {}] {}",
            record.level(),
            record.target(),
            record.args()
        );
        let mut lines = lock(&LOG);
        if lines.len() == LOG_LINES {
            lines.pop_front();
        }
        lines.push_back(line);
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

/// `time` in UTC: year, month, day, hour, minute, second.
fn utc(time: SystemTime) -> [u64; 6] {
    let seconds = time.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (days, of_day) = (seconds / 86_400, seconds % 86_400);
    // The proleptic Gregorian calendar from a day count (H. Hinnant's
    // `civil_from_days`), on eras of 400 years starting 0000-03-01.
    let z = days + 719_468;
    let era = z / 146_097;
    let of_era = z % 146_097;
    let year_of_era = (of_era - of_era / 1460 + of_era / 36_524 - of_era / 146_096) / 365;
    let day_of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = era * 400 + year_of_era + u64::from(month <= 2);
    [year, month, day, of_day / 3600, of_day % 3600 / 60, of_day % 60]
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn at(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
    }

    #[test]
    fn utc_dates() {
        assert_eq!(utc(at(0)), [1970, 1, 1, 0, 0, 0]);
        assert_eq!(utc(at(951_782_400)), [2000, 2, 29, 0, 0, 0], "a leap day");
        assert_eq!(utc(at(1_798_718_602)), [2026, 12, 31, 12, 3, 22]);
        assert_eq!(utc(at(4_102_444_800)), [2100, 1, 1, 0, 0, 0]);
    }

    fn report() -> Report {
        Report {
            build: BuildInfo {
                version: "1.2.3",
                commit: "abc1234",
                date: "2026-11-01",
                licences: "",
            },
            adapter: "Test GPU · Vulkan".to_owned(),
            thread: "main".to_owned(),
            message: "index out of bounds".to_owned(),
            location: "crates/ui/src/x.rs:12:5".to_owned(),
            backtrace: "   0: grid4d::main\n".to_owned(),
            log: vec!["[    0.100s INFO  grid4d] Renderer: Test GPU".to_owned()],
            time: at(1_798_718_602),
        }
    }

    #[test]
    fn a_report_says_what_where_and_on_which_build() {
        let text = report().text();
        for expected in [
            "Version:  1.2.3 (abc1234, 2026-11-01)",
            "Time:     2026-12-31 12:03:22 UTC",
            std::env::consts::OS,
            "Graphics: Test GPU · Vulkan",
            "Thread:   main",
            "At:       crates/ui/src/x.rs:12:5",
            "Message:\nindex out of bounds",
            "Backtrace:\n   0: grid4d::main",
            "Last log lines:\n[    0.100s INFO  grid4d] Renderer: Test GPU",
        ] {
            assert!(text.contains(expected), "{expected:?} in\n{text}");
        }
        assert_eq!(report().file_name(42), "crash-2026-12-31T12-03-22-42.txt");
    }

    #[test]
    fn the_newest_report_is_pointed_to_once_and_old_ones_go() {
        let dir = crate::autosave::tests::folder("crashes");
        assert_eq!(CrashReports::new(&dir.join("none yet")).new_report, None);
        let mut written = Vec::new();
        for k in 0..25 {
            let report = Report {
                time: at(1_798_718_602 + k * 60),
                ..report()
            };
            written.push(report.write(&dir, 7).unwrap());
        }
        fs::write(dir.join("notes.txt"), "not a report").unwrap();

        let reports = CrashReports::new(&dir);
        assert_eq!(reports.new_report.as_ref(), written.last());
        let left = fs::read_dir(&dir).unwrap().count();
        assert_eq!(left, KEEP + 2, "the newest 20, the notes and the shown mark");
        assert!(!written[4].exists() && written[5].exists());
        assert_eq!(CrashReports::new(&dir).new_report, None, "pointed to once");

        let next = Report {
            time: at(1_900_000_000),
            ..report()
        }
        .write(&dir, 8)
        .unwrap();
        assert_eq!(CrashReports::new(&dir).new_report, Some(next));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_report_into_a_folder_that_cannot_be_made_is_an_error() {
        let dir = crate::autosave::tests::folder("crash-blocked");
        let file = dir.join("a file");
        fs::write(&file, "").unwrap();
        assert!(report().write(&file.join("crashes"), 1).is_err());
        let _ = fs::remove_dir_all(dir);
    }
}
