# Grid4D architecture

How Grid4D is put together, the rules every change follows, and where to add
things. For what is planned next, see [`IMPLEMENTATION_PLAN.md`](IMPLEMENTATION_PLAN.md);
for how to use the application, the [manual](manual.md).

## The crates

Grid4D is a Cargo workspace. Each layer is a crate, and dependencies point one
way only — from the interface down to geometry — so the compiler enforces the
layering. The main line:

```
grid4d-geom ◀── grid4d-scene ◀── grid4d-eval ◀── grid4d-render ◀── grid4d-ui ◀── grid4d
 (geometry)       (document)      (evaluation)      (viewport)      (interface)   (binary)
```

| Crate | What it does | Uses (Grid4D crates) |
|-------|--------------|----------------------|
| `grid4d-geom` | Geometry: polygon meshes (n-gons), topology, normals, triangulation, BVH, primitives, splines, the maths of deformers, Cloner layouts, surfaces from splines, mesh operations (extrude, bevel, cuts, …), UVs (projections, checks, islands, unwrapping by LSCM, relax, pack) | — |
| `grid4d-text` | Glyph outlines of the built-in font, for Text splines | geom |
| `grid4d-scene` | The document — the single source of truth: the scene graph, object kinds and parameters, tags, animation tracks, materials, textures, layers, takes, render and simulation settings, and `SceneEdit` (every undoable change). No GPU, no interface, no file formats | geom |
| `grid4d-plugin` | The plugin API: parameter descriptions, generator / deformer / command traits, the registry | geom, scene |
| `grid4d-eval` | What is shown at a frame: world matrices, drawables, pick shapes, the Material tags reaching each object (material layers), generated and deformed meshes, clones, lights (and which linked lights skip each object: a 64-bit mask), the surround (the topmost visible Sky or Environment; `sky`: the daylight model, equirectangular mapping, spherical-harmonic ambient light) — cached, and skipped when nothing changed | geom, text, scene, plugin |
| `grid4d-render` | The wgpu viewport: per-frame extraction, mesh cache, shading modes, textures (pictures handed over by the interface, a bind group per textured material), shadows, SSAO, a Sky's or Environment's background pass and ambient light, picking, snapping. Reads the evaluated scene, never the document (the materials' texture slots come with the frame) | geom, scene, eval |
| `grid4d-raytrace` | The final renderer: a deterministic CPU path tracer over the evaluated scene, with the document's materials (Material tags as layers), the texture pictures the interface hands over and glass; lights with a surface (area lights, glowing surfaces) sampled from one table by power, and the surround (Sky, Environment) by importance over its brightness, with multiple importance sampling | geom, scene, eval |
| `grid4d-sim` | Rigid-body simulation, cached per frame; the only crate that sees `rapier3d` | geom, scene, eval |
| `grid4d-io` | `.g4d` files (a versioned zip: JSON, binary meshes, embedded pictures), glTF and OBJ import / export, PNG and OpenEXR out, texture pictures in (decoded with mip-maps) | geom, scene |
| `grid4d-script` | Rhai scripts working on a copy of the document | geom, scene, plugin |
| `grid4d-ui` | The egui interface: panels, theme, icons, shortcuts, commands, undo history, translations, autosave, crash reports, texture pictures loaded and material previews rendered in the background, the UV Editor | all of the above |
| `grid4d` | The binary: the window, start-up (the logger and the panic hook first), settings kept between sessions, the list of built-in plugins — the interface itself is `grid4d_ui::shell::Shell` | ui, scene, plugin, plugins |
| `plugins/*` | The example plugins: Gear, Wave, Scatter | plugin, geom, scene |

## How a frame works

1. **Input** — panels draw with egui and, instead of changing the document
   structurally, push `EditorCommand`s (create, delete, select, reparent, run a
   tool …). Plain value edits (a field being dragged) may write the document
   directly, and are recorded as one undo step when the interaction ends.
2. **Commands** — once per frame, `commands::apply_pending` applies the
   queued commands. Every document change becomes `SceneEdit`s applied through
   `Document::apply`, which returns the exact inverse; `History` keeps the
   inverses as undo steps.
3. **Evaluation** — `EvaluatedScene::update` re-evaluates only when the
   document's revision or the frame changed: animation, generators, deformers,
   clones, the simulation's poses.
4. **Rendering** — `FrameData::extract` turns the evaluated scene into GPU
   instances; `ViewportRenderer::render` draws each view into its own texture,
   which egui shows.

## Rules

**One source of truth.** The document (`grid4d-scene`) holds the data; the
editor's own state — selection, tool, modes, undo history — lives in
`EditorState`. Nothing else keeps copies that could drift.

**Every change undoes.** A document change is a `SceneEdit` (add a variant and
a case in the random undo / redo test for a new kind of change), recorded
through `commit` as one step. A panel never changes the document without
either an `EditorCommand` or a live edit followed by `CommitLive`. A second
random test (`crates/ui/src/random_commands.rs`) drives long sequences of
`EditorCommand`s, undos and redos through `commands::apply_pending` and
checks that a history state always shows the same document — add a case
there for a new command.

**Read without touching.** Code that only reads uses `Scene::get`, never
`get_mut`: every `get_mut` bumps the revision and makes evaluation run again.
A test draws every panel on idle frames and fails if the revision moves.

**Objects by id.** Objects are referred to by `NodeId` (a generational key),
never by index or reference. Deleting is a soft delete (`detach`), so undo
brings back the same ids.

**The hot path does not allocate.** Everything that runs every frame for the
viewport — evaluation, extraction, the renderer, picking, gizmo maths —
reuses its buffers (`clear()` keeps capacity), never calls `format!`,
`collect()` or `Box::new`, and grows GPU buffers geometrically. Tests guard
the buffers (`extract_reuses_capacity`), and `crates/render/tests/steady_frame.rs`
counts allocations over whole frames of 10 000 objects (playing, orbiting,
selecting): there must be none. The budget (frame rates, opening times) and
its measurements are in [`performance.md`](performance.md); `criterion`
benches live in each crate's `benches/` (`cargo bench -p <crate>`), and
`cargo run --release --example stress` writes the scenes it is measured on.

**Safe and explicit Rust.** No `unsafe` (forbidden workspace-wide), no
`unwrap()` / `expect()` outside tests, conversions with `try_from` unless
lossless, closed sets as enums matched exhaustively (no `_ =>` that would hide
a new variant). `cargo clippy -- -D warnings` must pass.

**Shaders** are WGSL files validated in `cargo test`; every `#[repr(C)]` struct
shared with a shader has a size test.

**Files.** A released `.g4d` version never changes: new data means a new
version and a migration. The current version's module (`io/src/v5.rs`) reads
and writes; an older archive version is upgraded to it as JSON first, one
version at a time (`io/src/v3.rs` to version 4, `io/src/v4.rs` to version 5),
so its meaning stays fixed while the current one grows. Loading never trusts sizes a file claims and rejects
numbers that are not finite. Files are read through `grid4d_io::read_file`
(the memory reserved first, so a file too large is an error, not an abort)
and written to a temporary file that replaces the old one only when complete.

**Failures are told, locally.** A panic writes a crash report into the data
folder (`ui/src/crash.rs`; nothing is sent anywhere) and the next start
points to it; start-up without graphics or without a writable data folder
says so (`ui/src/startup.rs`). Errors the user can act on are alerts
(`EditorState::file.alert`), not log lines.

**Every text the interface shows is translated**: `i18n::tr("English text")`,
with the Greek in `crates/ui/src/i18n/el.rs`; a test lists what is missing.
Document data (names) is never translated. Texts use only characters of the
built-in font (Ubuntu Light and egui's icons), and widgets have accessible
names (a field is `labelled_by` its label).

**The interface is tested headless.** `crates/ui/tests/interface.rs` drives
the whole `Shell` with `egui_kittest` (clicks, drags, keys, typing — through
egui's input, as a user would); `crates/ui/tests/walkthrough.rs` (run by
hand) renders it in both languages and themes for review. Shortcuts live in
one table, `shortcuts::TABLE`: Help ▸ About shows it and a test keeps the
manual's list equal to it.

**Releases** are built by `.github/workflows/release.yml` on a `v*` tag (or
as a dry run by hand): packages for Windows (`.zip`, WiX `.msi`), macOS
(universal `.dmg`) and Linux (`.tar.gz`, AppImage) from the files in
`assets/` (icons, `Info.plist`, `.desktop`, MIME type, WiX source), a draft
GitHub Release with the version's `CHANGELOG.md` section, and itch.io when a
`BUTLER_API_KEY` secret exists. The tag must equal the workspace version.

**Inspired by, never copied.** The workflow follows Cinema 4D's; no Maxon
icons, artwork, trademarks or file formats, and behaviour specs in
`docs/reference/` are written in our own words from public documentation.

## Conventions

- Units: centimetres. World space: right-handed, Y up.
- Rotations: H, P, B Euler angles in radians (heading about Y, pitch about X,
  bank about Z, in that order), shown in degrees.
- Colours in the document are linear RGB.
- Duplicate names follow Cinema 4D: `Cube`, `Cube.1`, `Cube.2`.
- Shortcuts follow Cinema 4D's where one exists.

## Where to add things

**A panel** — `crates/ui/src/panels/<name>.rs` with a `show` function (or a
`<Name>Panel` struct), exported in `panels/mod.rs`, then a `Pane` in
`ui/src/workspace.rs` (title, place in every preset). Panels never call each
other and never touch files.

**A kind of object** — `ObjectKind` and `ObjectKindTag` in
`scene/src/node.rs`; its geometry in `geom` if it is a new shape; a `MeshKey`
in `eval`; the file format's `KindDto` in `io`; an icon in `ui/src/icons.rs`;
its parameters in `ui/src/panels/attributes.rs`. Exhaustive matches point at
every other place.

**A command** — a variant of `EditorCommand`, handled in `commands::apply`;
if it changes the document, through `commit`, with an undo / redo test.

**A plugin** — see [`plugins.md`](plugins.md). **A script** — see
[`scripting.md`](scripting.md).

**A feature users see** — a behaviour spec in `docs/reference/` first, then
its chapter in the [manual](manual.md) (with a screenshot when it helps) and
its row in [`progress.md`](progress.md).
