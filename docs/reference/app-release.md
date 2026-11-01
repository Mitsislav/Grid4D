# The application as released

> Our own specification (not drawn from Maxon's documentation): what a user
> of a released Grid4D sees about the program itself — its version, the
> licences of the software it is built from, and (later steps) what happens
> after a crash and how fast it draws.
>
> Covers implementation-plan steps **14.1** (About), **14.3** (crash reports
> and robustness) and 14.4 (the frame-time readout).

## Purpose

A user reporting a problem can say exactly which build they run; the
licences of everything Grid4D contains are available inside the program, as
those licences require. When something goes wrong — a crash, no usable
graphics card, a data folder that cannot be written, a full disk, a file too
large for memory — Grid4D says so in words and loses no work it could keep.

## Behaviour (14.1)

- **Help ▸ Keyboard Shortcuts & About** shows, under the name, the version
  and the build: "Version 0.13.0 (6189df3, 2026-11-01)" — the version, the
  first characters of the source commit it was built from and that commit's
  date. A build made outside a git checkout shows the version alone.
- The window has two pages, **Keyboard & mouse** (the shortcut list, as
  before) and **Licences**: what Grid4D's own licence is (MIT or Apache-2.0,
  at the user's option), then the third-party software it contains — each
  licence once, with the crates that use it and the licence's full text.
  The text scrolls (lines are not wrapped; long ones scroll sideways); it
  is the same as `THIRD_PARTY_LICENSES.txt` shipped beside the program, whose
  first lines say Grid4D's own licence.
- The version numbers follow the plan's releases: 0.13.0 for the work of
  Phase 13, 1.0.0 for the first stable release.

## Behaviour (14.3)

### Crash reports

- When Grid4D runs into an internal error (a Rust panic, on any thread), it
  writes a **crash report**, a plain text file, into the `crashes` folder of
  its data folder (on Windows `%APPDATA%\grid4d\data\crashes`, on Linux
  `~/.local/share/grid4d/crashes`, on macOS
  `~/Library/Application Support/grid4d/crashes`), named after the time
  (UTC) and the process: `crash-2026-11-01T14-03-22-1234.txt`.
- The report holds: the version, commit and date (as in About), the system
  (operating system and processor architecture), the graphics adapter, the
  thread, the error's message and where in the source it happened, a
  backtrace, and the last lines Grid4D logged (up to 200). No document
  content, no file other than what the log mentions.
- **Nothing is sent anywhere.** The report stays on the computer; the user
  may attach it to a bug report.
- Only the 20 newest reports are kept; older ones are removed at start-up.
- **On the next start**, the newest report not shown before is pointed to,
  once:
  - with a recovery copy waiting, in **Recover Unsaved Work?** under the
    message: "A crash report was written: <file name>", with a **Show
    Report** button;
  - without one, in its own window, **Grid4D Stopped Unexpectedly**, with
    the same line, **Show Report** and **OK**.

  **Show Report** opens the system's file manager at the report (Explorer
  with it selected, Finder revealing it, the folder on Linux).
- An internal error in a background job (a render, an autosave) may leave
  Grid4D running; it is reported all the same, and pointed to on the next
  start.

### Start-up and data

- **No usable graphics adapter.** If no adapter can be used (no driver, an
  unsupported card, a remote session without graphics), Grid4D does not
  vanish: it shows a system message box, **Grid4D Cannot Start**, saying the
  graphics could not be started, the reason given by the system, and that
  Grid4D needs a graphics card with Vulkan, Direct3D 12 or Metal support,
  where updating the graphics driver often helps; then it exits. A software
  adapter (such as Windows' WARP) is used when it is the only one: slow, but
  Grid4D works.
- **No usable data folder.** If Grid4D has no data folder, or cannot create
  or write it (permissions, a read-only disk), it starts anyway and says so
  once, in **Data Folder Unavailable**: settings, recovery copies, scripts
  and crash reports cannot be kept in this session.
- **Autosave failing.** The first time in a session that a recovery copy
  cannot be written (a full disk, the folder taken away), Grid4D says so in
  **Autosave Failed** with the reason: the user knows the work is not being
  kept safe and can save it themselves. Later failures are only logged.
- **A full disk while saving.** Saving writes a temporary file beside the
  document and only then replaces it, so when the disk is full (or anything
  else stops the write) the old file is kept as it was, the temporary file
  is removed, and **Could Not Save Document** gives the reason; the document keeps
  its unsaved changes.
- **A file too large for memory.** Opening, merging or importing a file
  first reserves the memory it needs; when that is not available the file
  is refused with "not enough memory to read this file (<size>)" instead of
  Grid4D stopping.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| The commit and its date (`GRID4D_COMMIT`, `GRID4D_COMMIT_DATE`, empty without git) | `build.rs` (the binary) |
| `BuildInfo` (version, commit, date, licences text), `EditorState::build` | `grid4d-ui`; filled by `src/app.rs` |
| The About window's pages | `grid4d-ui::panels::about` |
| The licence list, generated by `cargo about` from `about.toml` and `about.hbs`; checked by CI to match `Cargo.lock` | `assets/THIRD_PARTY_LICENSES.txt` |
| The licence policy (`cargo deny`: allowed licences, advisories, duplicate versions, sources) | `deny.toml`, CI |
| The panic hook, the report's text and file, the last log lines (`crash::install`, `crash::install_log`, `CrashReports`), `EditorState::crash` | `grid4d-ui::crash`; installed by `src/main.rs` |
| The pointer on the next start (in Recover Unsaved Work?, or its own window); Show Report (`EditorCommand::ShowCrashReport` → `file::reveal`) | `grid4d-ui::panels::recovery` |
| The data folder check and the start-up failure message (`startup::check_data_folder`, `startup::cannot_start`) | `grid4d-ui::startup`; called by `src/main.rs` and `src/app.rs` |
| Autosave's first failure as an alert | `grid4d-ui::autosave`, `file::autosave` |
| Reading files with the memory reserved first (`read_file`) | `grid4d-io` |

## Decisions (2026-11-01)

1. **The commit's date, not the build's**, so two builds of the same commit
   are identical.
2. **Plain text** for the licence list: readable in the program, in any
   editor and in every package (the plan's `.html` became `.txt`).

## Decisions (14.3)

3. **Every panic is reported**, not only the one that ends the program: a
   failed render or autosave is a bug too, and a report costs nothing.
4. **The report is pointed to once**, on the next start; it stays in the
   folder (the newest 20) for anyone who wants it later.
5. **The time in a report's name is UTC**, so names sort in the order the
   reports were written whatever the time zone.
6. **No GPU means no editor**: a CPU-only fallback (egui's software
   rendering) is not part of eframe's wgpu path, and the viewport needs
   wgpu. A clear message instead of a silent exit.

## Acceptance criteria

- [x] About shows the version, commit and date; the Licences page lists every
      third-party licence with its full text.
- [x] CI fails when the licence list is out of date or a dependency's
      licence is not allowed.
- [x] A panic writes a report with the version, system, adapter, message,
      backtrace and log; the next start points to it once.
- [ ] Show Report opens the file manager at the report (not yet seen on
      screen: the test machine's posted clicks did not reach the dialog).
- [x] Without a usable adapter Grid4D shows why and exits; without a data
      folder it starts and says what is not kept.
- [x] A save that cannot be written keeps the old file; a file too large
      for memory is refused with a message.
