//! The panic hook in a process of its own (a test binary), since it is
//! installed for the whole process: a panic on any thread writes a report
//! with the build, the adapter, the message, a backtrace and the last log
//! lines, and the next start points to it.

use std::fs;

use grid4d_ui::BuildInfo;
use grid4d_ui::crash::{self, CrashReports};

/// Writes nothing anywhere; only what passes it is kept for the report.
struct Quiet;

impl log::Log for Quiet {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, _: &log::Record<'_>) {}

    fn flush(&self) {}
}

#[test]
fn a_panic_on_any_thread_writes_a_report_pointed_to_on_the_next_start() {
    let dir = std::env::temp_dir().join(format!("grid4d-crash-hook-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    crash::install_log(Box::new(Quiet), log::LevelFilter::Debug);
    crash::install(
        dir.clone(),
        BuildInfo {
            version: "9.9.9",
            commit: "f00dfac",
            date: "2026-11-01",
            licences: "",
        },
    );
    crash::set_adapter("Test Adapter · Vulkan");
    log::info!("before the crash");
    log::debug!("filtered out");

    let worker = std::thread::Builder::new()
        .name("render worker".to_owned())
        .spawn(|| {
            let items: Vec<u32> = Vec::new();
            let index = items.len() + 3;
            items[index]
        })
        .unwrap();
    assert!(worker.join().is_err(), "the thread panicked");

    let reports: Vec<_> = fs::read_dir(&dir).unwrap().filter_map(Result::ok).collect();
    assert_eq!(reports.len(), 1, "one report");
    let path = reports[0].path();
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        name.starts_with("crash-") && name.ends_with(&format!("-{}.txt", std::process::id())),
        "{name}"
    );
    let text = fs::read_to_string(&path).unwrap();
    for expected in [
        "Version:  9.9.9 (f00dfac, 2026-11-01)",
        "Graphics: Test Adapter · Vulkan",
        "Thread:   render worker",
        "crash_report.rs",
        "index out of bounds: the len is 0 but the index is 3",
        "Backtrace:",
        "INFO  crash_report] before the crash",
    ] {
        assert!(text.contains(expected), "{expected:?} in\n{text}");
    }
    assert!(!text.contains("filtered out"), "only what the logger lets through");

    assert_eq!(CrashReports::new(&dir).new_report, Some(path));
    assert_eq!(CrashReports::new(&dir).new_report, None);
    let _ = fs::remove_dir_all(dir);
}
