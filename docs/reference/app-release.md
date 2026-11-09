# The application as released

> Our own specification (not drawn from Maxon's documentation): what a user
> of a released Grid4D sees about the program itself — its version, the
> licences of the software it is built from, and (later steps) what happens
> after a crash and how fast it draws.
>
> Covers implementation-plan steps **14.1** (About), **14.3** (crash reports
> and robustness), **14.4** (the frame-time readout) and **14.5** (the
> interface pass).

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
  Phase 13, 1.0.0-rc.1 for the first release candidate (the packages of
  14.6), 1.0.0 for the first stable release, 1.1.0 for Phase 15 (materials,
  UVs and textures; `.g4d` version 4), 1.2.0 for Phase 16 (render quality;
  `.g4d` version 5).

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

## Behaviour (14.4)

### The frame-time readout

- **View ▸ Frame Time** (in any view's View menu; off when Grid4D starts,
  not saved) shows, in the top-right corner of the active view, how long
  frames take:
  - first line: the time between frames, the rate and the slowest frame of
    about the last two seconds, "16.7 ms · 60 fps · worst 18.2 ms" (a
    single slow frame — a hitch — shows there even when the average hides
    it);
  - second line: the slowest part of the viewport's own work in that frame
    and its time, "slowest: Evaluation 2.1 ms" — one of **Evaluation**
    (animation, generators, deformers, clones), **Extraction** (what to
    draw, batched) and **Drawing** (the commands sent to the graphics card).
- The values are averaged over the last frames so they can be read while
  the view moves. A frame is only drawn when something changes (orbiting,
  playing, editing); with nothing moving the readout keeps its last
  values, and pauses longer than a quarter of a second do not count.
- The rate cannot go above the display's refresh rate (frames wait for the
  screen); a time between frames at the refresh interval means the budget
  is met. The parts are processor time; the graphics card's own time is not
  measured.
- Choosing it again hides it. It is drawn in every theme with the view's
  own (dark) colours, like the HUD.

### The performance budget

What Grid4D promises on a mid-range computer (the reference: an RTX 3050
Laptop GPU on Windows 11), release build. Measured results, and anything
accepted as an exception, are recorded in `docs/performance.md`.

| Scenario | Budget |
|----------|--------|
| 10 000 primitive objects, orbiting or playing | ≥ 60 fps; no memory allocated per frame by evaluation and extraction once warm |
| One mesh of 1 000 000 triangles | ≥ 60 fps orbiting; Points / Edges / Polygons mode ≥ 30 fps |
| Select All on 10 000 objects | no frame noticeably slower than its neighbours |
| Opening a 50 MB `.g4d` | under 2 s |

`cargo run --release --example stress -- <folder>` writes the scenes these
are measured on (`stress-10k.g4d`, `stress-1m.g4d`, `stress-50mb.g4d`).

## Behaviour (14.5)

- **One list of shortcuts.** Help ▸ Keyboard Shortcuts & About and the
  manual's chapter on shortcuts show the same list, in the same order: the
  keys (as written on the keyboard, not translated) and what they do
  (translated). A shortcut added to Grid4D is added to that list once.
- **Every field and list row has a name** for screen readers: the
  Coordinates fields are named by their labels (P . X, S . X, R . H …), the
  timeline's frame-rate menu is "Frame Rate", and each Object Manager row is
  the object's name, with whether it is selected.
- **Text uses only the built-in font.** Every character the interface shows
  is in the typeface Grid4D carries (Ubuntu Light, with egui's icons), so no
  system font is needed: menu paths read "Render › Render to Picture
  Viewer", and the remove button of a list is "×".
- **Every text in the interface is translated**, the Coordinate Manager's
  column titles (Position, Scale / Size, Rotation) included.
- Dialogs are wide enough for their texts in both languages (Optimize).

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
| The readout: `FrameTimes` (averaged parts, the slowest), View ▸ Frame Time | `grid4d-ui::panels::viewport::frame_time` |
| Benchmarks (`cargo bench`): evaluation, extraction, triangulation, subdivision, generators, `.g4d` save / load, the path tracer | `crates/*/benches/` |
| Zero allocations in a steady frame (a counting allocator) | `crates/render/tests/steady_frame.rs` |
| The stress scenes | `examples/stress.rs` (the binary's package) |
| Measurements | `docs/performance.md` |
| The shortcut list (`shortcuts::TABLE`), shown by About; a test keeps the manual's §23 equal to it | `grid4d-ui::shortcuts`, `panels::about` |
| The whole interface as one value (`Shell`: document, editor, panels; one frame in `ui`), used by the binary and by tests | `grid4d-ui::shell` |
| Interface tests (selection with Shift / Ctrl, drag to re-parent, rename, a typed value, a gizmo drag, a deformer handle drag, a menu command — each with undo) | `crates/ui/tests/interface.rs` (`egui_kittest`) |
| The walkthrough: the interface rendered in English and Greek, Dark and Light (menus, panes, windows) for review | `crates/ui/tests/walkthrough.rs` (ignored; run by hand) |

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

## Decisions (14.4)

7. **The readout times the viewport's own work** (evaluation, extraction,
   drawing commands) with a clock read per part — nothing allocates while
   it is off, and the parts a user can influence (scene size, generators,
   the display mode) are the ones named.
8. **Benchmarks are built in CI, run by hand**: timings on shared runners
   vary too much to compare; the reference machine's numbers live in
   `docs/performance.md`.
9. **The allocation test counts per thread** with `allocation-counter`
   (its own global allocator; the workspace forbids `unsafe`, so Grid4D
   cannot write one), linked into that one test binary only.

## Decisions (14.5)

10. **The walkthrough is pictures, not a window.** Posted mouse input does
    not reach the window from a test driver, so the interface is rendered
    headless with `egui_kittest` and wgpu (a software adapter will do) and
    the pictures are read; clicks and drags are tested the same way, through
    egui's own input.
11. **Keys stay in English in the shortcut list**: they name the keys on
    the keyboard (Ctrl, Shift, F8); the actions are translated.
12. **No system fonts**: characters outside the built-in font are replaced
    rather than a font added (› for ▸, × for ✕).

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
- [x] View ▸ Frame Time shows the time between frames, the worst recent
      frame and the slowest part.
- [x] Every budget measured on the reference machine and met, or accepted
      with a reason, in `docs/performance.md`.
- [x] Help ▸ About and the manual list the same shortcuts (a test).
- [x] Interface tests drive selection, re-parenting, renaming, typing, the
      gizmo, a deformer handle and a menu, each with undo.
- [x] The interface read in English and Greek, Dark and Light; what read
      wrong fixed.
