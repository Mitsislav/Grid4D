# Grid4D — Implementation Plan

> Status of this document: **v1, 2026-10-18.** Written after a full read of the
> v0.1 codebase (≈ 6.1 k lines, 27 tests). It turns the short roadmap into
> ordered, PR-sized work with design decisions, acceptance criteria and the
> files each step touches. Update it when a phase lands.
>
> **2026-10-19: the project was renamed from OpenC4D to Grid4D** — crates
> `oc4d-*` → `grid4d-*`, binary `grid4d`, documents `.oc4d` → `.g4d` (old files
> still open), original app icon in `assets/icon/`. Older entries below use the
> new names.

All rules in [`ARCHITECTURE.md`](ARCHITECTURE.md) apply to every step below — dependency direction,
the `EditorCommand` choke point, the allocation-free hot path, `NodeId`-only
references, no `unsafe`, no `unwrap`, exhaustive `match`, and
**inspired by, never copied** (no Maxon assets, icons, file formats or trademarks).

---

## Contents

1. [Where we are (v0.1 audit)](#1-where-we-are-v01-audit)
2. [Target architecture](#2-target-architecture)
3. [Release train](#3-release-train)
4. [Phase 0 — Foundations & quick fixes](#phase-0--foundations--quick-fixes-v011)
5. [Phase 1 — Undo / Redo](#phase-1--undo--redo-v02)
6. [Phase 2 — Document persistence (`.g4d`)](#phase-2--document-persistence-g4d-v02)
7. [Phase 3 — Geometry core, evaluation, offscreen viewport](#phase-3--geometry-core-evaluation-offscreen-viewport-v03)
8. [Phase 4 — Polygon modelling & Subdivision Surface](#phase-4--polygon-modelling--subdivision-surface-v04)
9. [Phase 5 — Interchange (glTF, OBJ)](#phase-5--interchange-gltf-obj-v05)
10. [Phase 6 — Animation, tags, materials, managers](#phase-6--animation-tags-materials-managers-v06)
11. [Phase 7 — Viewport: cameras, lights, 4-view, shading](#phase-7--viewport-cameras-lights-4-view-shading-v07)
12. [Phase 8 — Offline renderer](#phase-8--offline-renderer-v08)
13. [Phase 9 — Splines & spline generators](#phase-9--splines--spline-generators-v09)
14. [Phase 10 — Deformers & MoGraph](#phase-10--deformers--mograph-v010)
15. [Phase 11 — Simulation](#phase-11--simulation-v011)
16. [Phase 12 — Workspace](#phase-12--workspace-v012)
    · [Phase 13 — Extensibility](#phase-13--extensibility-v013)
    · [Phase 14 — Stabilisation](#phase-14--stabilisation-v10)
17. [After 1.0 — the road to a complete suite](#after-10--the-road-to-a-complete-suite)
    · [Phase 15 — Materials, UVs and textures](#phase-15--materials-uvs-and-textures-v11)
    · Phases 16 – 24 (outlines): render quality, modelling 2, fields and motion graphics 2,
    sculpting and painting, characters and animation 2, simulation 2, nodes, GPU rendering, pipeline
18. [Cross-cutting: testing, performance, CI, docs](#cross-cutting-testing-performance-ci-docs)
19. [Risks & open decisions](#risks--open-decisions)

Size legend for steps: **S** ≈ one small PR (< 300 lines), **M** ≈ one focused PR
(300–1000 lines), **L** ≈ several PRs.

---

## 1. Where we are (v0.1 audit)

### What works

| Area | State | Key files |
|------|-------|-----------|
| Scene graph | Slotmap arena, generational `NodeId`, cycle-safe `insert/insert_child/remove/reparent(keep_world)`, allocation-free `walk` | `crates/scene/src/scene.rs` |
| Object types | `Null, Cube, Sphere, Cylinder, Plane, Camera, Light` with C4D defaults | `crates/scene/src/node.rs` |
| Transforms | HPB Euler (radians) as source of truth, quaternion round-trip | `crates/scene/src/transform.rs` |
| Time | fps, range, fractional playback, loop, step | `crates/scene/src/time.rs` |
| Renderer | wgpu, 4 instanced unit meshes, MSAA, depth, analytic grid, Fresnel selection rim, geometric buffer growth | `crates/render/src/{gpu,frame,mesh}.rs`, `shaders/*.wgsl` |
| Picking | CPU ray vs. local AABB / exact sphere | `crates/render/src/picking.rs` |
| UI | Standard layout, menu bar, toolbar, mode palette, Object Manager (tree, rename, D&D reparent, traffic lights, search), Attribute Manager (Basic/Coord./Object + project), timeline, status bar, About | `crates/ui/src/panels/*` |
| Interaction | Orbit/pan/dolly, H/O framing, Move/Scale/Rotate gizmo with axis locks and World/Object | `crates/ui/src/panels/viewport/*` |
| Commands | `EditorCommand` queue applied once per frame after layout | `crates/ui/src/commands.rs` |

### What is visible in the UI but not implemented

Undo/Redo, Open/Save/Import/Export, Points/Edges/Polygons modes, snapping
(`EditorState::snapping` has no behaviour), Takes, Layers, Tags, Materials,
User Data, scene cameras, shading modes, Filter, 4-view, Record/keyframes,
Render View / Picture Viewer / Render Settings, Simulate, Spline/Generator menus.
Camera `fov_y_deg` and Light `color/intensity` are editable but unused by the viewport.

### Issues found while reading the code (fix in Phase 0)

| # | Issue | Where | Why it matters |
|---|-------|-------|----------------|
| A1 | `Selection::contains` is a linear `Vec` scan and is called **per node per frame** by `FrameData::extract` and by the bracket overlay | `ui/src/editor.rs:90`, `ui/src/panels/viewport/mod.rs:157`, `overlay.rs:28` | O(nodes × selected) on the hot path: "Select All" on 10 k objects ≈ 10⁸ comparisons/frame |
| A2 | `GizmoGeometry::new` is built twice per frame, and the 3 × 64-point rotation rings are projected even for Move/Scale | `viewport/mod.rs:140,183`, `gizmo.rs:63` | Wasted work on the per-frame path |
| A3 | Menu "Previous/Next Frame" neither stops playback nor closes the menu (shortcut `F`/`G` and transport buttons do) | `ui/src/panels/menu_bar.rs:137-142` | Inconsistent behaviour |
| A4 | `_ =>` arms hide new `ObjectKindTag` variants | `ui/src/commands.rs:101,150` | Violates the exhaustive-match rule; new object types silently get defaults |
| A5 | No test covers `commands::apply` | `ui/src/commands.rs` | `ViewportPanel::new(None, None, 1)` works headless, so it is testable today |
| A6 | Not a git repository; no LICENSE, no CI, no CONTRIBUTING | repo root | Blocks open-source collaboration |
| A7 | No Rust toolchain on the current dev machine, no `target/` | — | The v0.1 build is unverified here; `Cargo.lock` is committed so it should reproduce |

Known limitations to keep in mind (not bugs): the viewport draws directly into
egui's render pass (shares its MSAA/depth — blocks 4-view, overlays with their own
depth, ID-buffer picking); `reparent(keep_world)` cannot represent shear under
non-uniform parent scale; the timeline treats the document range and the preview
range as one.

---

## 2. Target architecture

The one-way dependency rule stays. New crates are added only where a real layer
appears:

```
                grid4d-geom   (PolyMesh, primitives, triangulation, topology, splines — glam only)
                     ▲
                grid4d-scene  (Document, hierarchy, tags, materials, animation data, SceneEdit/History)
                 ▲         ▲
         grid4d-eval      grid4d-io   (eval: generators, deformers, animation → EvaluatedScene)
          ▲     ▲      ▲        ▲     (io: .g4d, glTF, OBJ — serde lives here, not in scene)
grid4d-render  grid4d-raytrace  │     (raytrace: Phase 8)
          ▲     ▲  grid4d-sim   │     (sim: Phase 11 — rapier3d, the cache)
          ▲     ▲      ▲        │
          └───── grid4d-ui ─────┘
                     ▲
                grid4d (bin)
```

| Crate | Introduced | May depend on | Must never depend on |
|-------|-----------|---------------|----------------------|
| `grid4d-geom` | Phase 3 | `glam` | scene, wgpu, egui |
| `grid4d-scene` | exists | `grid4d-geom`, `glam`, `slotmap` | wgpu, egui, serde* |
| `grid4d-eval` | Phase 3 | scene, geom | wgpu, egui |
| `grid4d-io` | Phase 2 | scene, geom, `serde`, `serde_json`, `gltf`, `tobj` | wgpu, egui |
| `grid4d-render` | exists | eval, scene, geom, `wgpu` | egui |
| `grid4d-raytrace` | Phase 8 | eval, scene, geom, `rayon` | wgpu (CPU first), egui |
| `grid4d-text` | Phase 9 | geom, `skrifa` (from 14.3; `ttf-parser` before), `epaint_default_fonts` (font bytes only) | scene, wgpu, egui |
| `grid4d-sim` | Phase 11 | eval, scene, geom, `rapier3d` (its types never leave the crate) | wgpu, egui |
| `grid4d-ui` | exists | all of the above, `egui`, `rfd`, `egui_tiles` (Phase 12) | — |

\* Keeping serde out of `grid4d-scene` means the file format is a separate,
versioned DTO layer in `grid4d-io`; in-memory types can change without breaking files.

Two architectural moves carry most of the later phases:

1. **Reversible edits (Phase 1).** Every document mutation becomes a `SceneEdit`
   that returns its inverse. `EditorCommand` stays the UI-facing vocabulary and is
   translated into edits.
2. **Authoring data vs. evaluated data (Phase 3).** Today the renderer reads the
   `Scene` directly. With generators, deformers and animation, the renderer and
   picking read an `EvaluatedScene` (world matrices, visibility, geometry handles)
   produced by `grid4d-eval` with per-node revision-based caching.

---

## 3. Release train

| Version | Theme | Phases | "Done" means |
|---------|-------|--------|--------------|
| v0.1.1 | Foundations | 0 | git + CI green on Win/Linux/macOS, A1–A5 fixed |
| **v0.2** | **Safe editing** | 1, 2 | Every edit undoable; save/load `.g4d`; dirty flag + "save changes?" |
| v0.3 | Real geometry | 3 | Primitives with segments, Make Editable (`C`), offscreen viewport, mesh cache |
| v0.4 | Modelling | 4 | Points/Edges/Polygons modes, core mesh tools, Subdivision Surface |
| v0.5 | Interchange | 5 | glTF 2.0 + OBJ import/export round-trips |
| v0.6 | Animation & look | 6 | Keyframes, F-curves, tags, materials, Coordinate & Material managers |
| v0.7 | Viewport | 7 | Scene cameras, 4-view, shading modes, real lights, snapping |
| v0.8 | Rendering | 8 | Render View + Picture Viewer, CPU path tracer, PNG/EXR output |
| v0.9 | Splines | 9 | Spline objects and primitives, point editing and the Pen, Extrude / Lathe / Sweep / Loft, Text |
| v0.10 | Deformers & MoGraph | 10 | Animated parameters; Bend / Twist / Taper / Bulge / FFD; Current State to Object; Cloner (Linear, Radial, Grid, Object) with Plain and Random effectors |
| v0.11 | Simulation | 11 | Rigid and collider bodies (rapier3d) on objects and clones, cached per frame and saved when baked; renders and export show the simulation |
| v0.12 | Workspace | 12 | Docking layouts with presets (egui_tiles), Preferences (units, undo depth, autosave, themes), Layers, Takes, the interface in Greek |
| v0.13 | Extensibility | 13 | Plugins built in through `grid4d-plugin` (generators, deformers, commands; missing plugins kept), Rhai scripts as one undo step, the Script Manager and its library ✅ |
| v1.0 | Stabilisation | 14 | `.g4d` frozen and specified, fuzzed and property-tested files, crash reports, the performance budget measured and met, licences checked, installable packages for Windows / macOS / Linux from a tagged release, complete documentation |

After 1.0: v1.1 – v2.0 (Phases 15 – 24), in
[After 1.0](#after-10--the-road-to-a-complete-suite).

Phases 1 and 2 are independent and can run in parallel. Phase 5 needs Phase 3;
Phase 6 animation needs Phase 1 (edits) and Phase 3 (evaluation).

---

## Phase 0 — Foundations & quick fixes (v0.1.1)

**Goal:** a reproducible, collaborative baseline before larger work.

> **Status (2026-10-18): done locally**, CI not yet run (no remote). Rust 1.99.1;
> fmt, clippy `-D warnings` and 38 tests pass; the app starts (Vulkan). The first
> build on the newer clippy flagged `chunks_exact_to_as_chunks` in a mesh test —
> fixed. Remaining: push to GitHub so the CI workflow (incl. the 1.95 MSRV job) runs.

| Step | Size | Work |
|------|------|------|
| 0.1 | S | Install toolchain (`rustup`, stable ≥ 1.95, MSVC build tools on Windows); run `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`; fix anything the first build reveals. |
| 0.2 | S | `git init`, first commit of v0.1, add `LICENSE-MIT` + `LICENSE-APACHE` (README already proposes the Rust default), `CONTRIBUTING.md` (points to the architecture rules). |
| 0.3 | S | CI (GitHub Actions): matrix Windows / Ubuntu / macOS; jobs `fmt --check`, `clippy -D warnings`, `test`; cache `~/.cargo` + `target`; Linux job installs the X11/Wayland dev packages listed in the README. |
| 0.4 | S | **A1:** give `Selection` O(1) membership — keep the ordered `Vec<NodeId>` (active = last) plus a `slotmap::SecondaryMap<NodeId, ()>`; keep the API identical. Add a test that `contains` agrees with the Vec after add/toggle/retain. |
| 0.5 | S | **A2:** build `GizmoGeometry` once per frame and reuse it for interaction and painting; compute `rings` only for `Tool::Rotate` (`Option` per tool). |
| 0.6 | S | **A3/A4:** menu frame stepping calls `time.stop()` and `ui.close()`; replace the two `_ =>` arms with explicit variants. |
| 0.7 | S | **A5:** unit tests for `commands::apply` (create → unique names, delete subtree clears selection, reparent keeps world, select modes), using `ViewportPanel::new(None, None, 1)`. |

**Acceptance:** CI green on all three OSes; test count > 27; no behaviour change
except A3.

---

## Phase 1 — Undo / Redo (v0.2)

**Goal:** every document change — structural or value edit, including gizmo drags
— is undoable with `Ctrl+Z` / `Ctrl+Y` (and `Ctrl+Shift+Z`), with C4D-like granularity
(one drag = one undo step).

### Design

**1. `SceneEdit` — reversible, data-level operations** (`crates/scene/src/edit.rs`)

```rust
pub enum SceneEdit {
    /// Re-attach a previously detached subtree (also used for "create").
    Attach { root: NodeId, parent: Option<NodeId>, index: usize },
    /// Detach a subtree but keep its nodes alive in the arena (soft delete).
    Detach { root: NodeId },
    Move { id: NodeId, parent: Option<NodeId>, index: usize, transform: Transform },
    SetData { id: NodeId, data: NodeData },   // name, kind, transform, colour, flags
    SetTime(TimeSettings),                    // fps, range, loop
    SetDocumentName(String),
}

impl Document {
    /// Applies `edit` and returns the edit that undoes it.
    pub fn apply(&mut self, edit: SceneEdit) -> Result<SceneEdit, SceneError>;
}
```

**2. Stable ids across undo (the key design point).** A `SlotMap` never re-issues
the same key, so "delete, then undo by re-inserting" would produce a *new*
`NodeId` and break every later history entry and the selection. Therefore deletion
is a **soft delete**: `Detach` unlinks the subtree from `roots`/`children` but keeps
its nodes in the arena, marked detached. `walk`, `roots`, `len` and `iter` only see
attached nodes, so render, picking and panels are unaffected. Detached nodes are
**purged** when the history entry that owns them is dropped (redo stack cleared,
history limit reached, new document). New `Scene` API:
`detach_subtree`, `attach_subtree(root, parent, index)`, `purge_detached(root)`,
`index_in_parent(id)`, `insert_at(parent, index, node)`, `reparent_at(…, index)`.

**3. `NodeData` snapshot.** All non-hierarchy fields of `Node` move into a
`NodeData` struct (cheap to clone today; mesh data later lives behind `Arc` so
clones stay cheap). One `SetData` variant covers every Attribute-Manager and
Object-Manager value edit.

**4. History** (`crates/ui/src/history.rs` — editor state, not document)

```rust
pub struct UndoGroup { label: &'static str, edits: Vec<SceneEdit>, selection_before: Selection, selection_after: Selection }
pub struct History  { undo: Vec<UndoGroup>, redo: Vec<UndoGroup>, revision: u64, limit: usize }
```

* `EditorCommand` → `commands::apply` translates to edits, applies them via
  `Document::apply`, and pushes the inverses as one `UndoGroup`.
* Selection is **restored** by undo/redo (stored in the group) but selection
  changes alone do not create entries in v0.2 (decision recorded below).
* `History::state_id()` (id of the newest undo group, `0` before any) identifies the
  document state → drives the dirty flag (Phase 2): `state_id() != saved_state_id`.
  A plain revision counter would call a document dirty after edit + undo.

**5. Interactive value edits.** Plain value edits may keep writing the scene
directly for live feedback (an architecture rule), but commit an undo step **on
interaction end**:

* Gizmo: snapshot `NodeData` on drag start, write live while dragging, on drag end
  push `EditorCommand::CommitLive { inverse: SetData { id, before }, label }` (the
  "after" is the current state; no re-apply; nothing recorded if unchanged).
* Attribute Manager, project settings, timeline: a `LiveEdit<K, T>` tracker keeps
  the value from before the interaction (cloned only when the value changed), and
  reports the end of an interaction — pointer released and no text field focused —
  so a drag, a typed value or a click is exactly one `CommitLive` step. Changes made
  elsewhere between frames (gizmo, undo) become the new baseline.
* Object Manager and menus: discrete edits (rename, traffic lights, enable, Loop
  Playback) push `EditorCommand::Edit { edit, label }`.

### Steps

| Step | Size | Work | Files |
|------|------|------|-------|
| 1.1 ✅ | M | `NodeData` split; soft-delete (`detach/attach/purge`), ordered insert/reparent; tests for invariants (no dangling links, detached nodes invisible to `walk`, purge frees) — done 2026-10-18, incl. a seeded random-edit-sequence invariant test | `scene/{node,scene}.rs` |
| 1.2 ✅ | M | `SceneEdit` + `Document::apply` returning inverse; property test: random edit sequences, apply + inverse ⇒ document equals original — done 2026-10-18: variants `Insert/Detach/Attach/Move/SetData/SetTime/SetName`, atomic `apply_all`, `TimeSettings`, `Scene::transform_keeping_world`; seeded test checks every intermediate undo and redo state | `scene/edit.rs`, `scene/time.rs` |
| 1.3 ✅ | M | `History` + `UndoGroup`; `EditorCommand::{Undo, Redo, CommitData}`; translate existing commands; `NewDocument` clears history and purges — done 2026-10-18: create/delete/reparent are one step each, detached nodes are purged once no step can restore them (redo discarded, limit, clear), `History::state_id` ready for the dirty flag | `ui/{history,commands,editor}.rs` |
| 1.4 ✅ | M | Gizmo commit-on-release; Attribute Manager `track_edit`; Object Manager rename/visibility via commands — done 2026-10-18: generic `EditorCommand::{Edit, CommitLive}` (replacing `CommitData`), `LiveEdit` tracker (interaction = pointer held or text focus), also covers project settings, the timeline's range/fps/loop and the Animate ▸ Loop toggle; headless egui test drags the gizmo | `ui/{live_edit,commands}.rs`, `ui/panels/{viewport/gizmo,attributes,outliner,timeline,menu_bar}.rs` |
| 1.5 ✅ | S | Toolbar Undo/Redo enabled state + tooltip with label ("Undo Move"); Edit menu entries; shortcuts `Ctrl+Z`, `Ctrl+Y`, `Ctrl+Shift+Z`; history limit preference (default 100) — done 2026-10-18; shortcuts are ignored while a mouse button is held (a drag commits its own step). The limit is `History::DEFAULT_LIMIT` = 100; its UI waits for the preferences window (Phase 10+) | `ui/panels/{toolbar,menu_bar,about}.rs`, `ui/shortcuts.rs` |

**Acceptance:** create → move (gizmo) → rename → reparent → delete, then undo ×5
returns the exact original document and selection; redo ×5 reproduces it;
`NodeId`s are identical across the round trip; no allocation added to
`FrameData::extract` (existing `extract_reuses_capacity` still passes).

---

## Phase 2 — Document persistence (`.g4d`) (v0.2)

**Goal:** open/save/save-as, recent files, dirty tracking and safe quit.

### Design

* New crate **`grid4d-io`**. The file format is a set of versioned **DTO** structs
  with `serde` derives, mapped to/from `Document`. Hierarchy is stored as a nested
  tree (not slotmap keys) — ids are rebuilt on load.
* Container: **JSON** (`serde_json`) for v1: diffable, debuggable, trivially
  inspectable by contributors. Top level:
  `{ "format": "oc4d", "version": 1, "document": { … } }`. When polygon meshes
  arrive (Phase 3) large arrays move to a binary sidecar inside a zip container
  (`document.json` + `geometry/*.bin`) as format version 2, with a migration test.
* Every format bump ships a `migrate_vN_to_vN+1` function and a fixture file in
  `crates/io/tests/fixtures/`.
* Errors: `IoError` enum (`Io`, `Parse { line, column }`, `UnsupportedVersion`,
  `Invalid(String)`) — no panics on malformed files (fuzz the parser in CI later).
* UI: native dialogs via `rfd`; `EditorCommand::{Open(PathBuf), Save, SaveAs(PathBuf)}`;
  file work happens in `commands::apply`, never inside a panel.
* Dirty flag = `history.revision != saved_revision`; window title
  `Grid4D — name.g4d *`; on close request (`ViewportCommand::CancelClose`) show a
  "Save changes?" modal; same on New/Open.
* eframe persistence (`persistence` feature): recent files, window size, manager
  splitter sizes.

### Steps

| Step | Size | Work |
|------|------|------|
| 2.1 ✅ | M | `grid4d-io` crate: DTOs, `save(&Document, path)`, `load(path) -> Result<Document, IoError>`; round-trip test on the demo scene; fixtures — done 2026-10-18: flat pre-order object list with parent indices (no recursion limit, no cycles), HPB radians (exact round trip), atomic save via temp file + rename, non-finite numbers rejected, golden fixture `demo_v1.oc4d` |
| 2.2 ✅ | M | File menu Open/Save/Save As + `Ctrl+O/S/Shift+S`; dirty flag; title; "Save changes?" modal on New/Open/Quit — done 2026-10-18: `ui/file.rs` (+ `Ctrl+N`), `FileDialogs` seam with rfd, dirty = `History::state_id` ≠ saved, close button intercepted via `CancelClose`, error alert panel. Open: the title shows the file name; the document's own `name` is not renamed on Save As (revisit) |
| 2.3 ✅ | S | Recent files (File ▸ Recent), persisted window/splitter state — done 2026-10-18: `RecentFiles` (10, newest first, vanished files dropped), opening a recent file asks "Save changes?" too; eframe `persistence` on the binary restores window geometry and panel sizes |
| 2.4 ✅ | S | Drag-and-drop a `.g4d` file onto the window opens it — done 2026-10-18: first `.g4d` of a drop opens via `EditorCommand::OpenFile` (with the "Save changes?" prompt); other files get an alert; `drop_overlay` panel says what a drop will do while hovering. **Phase 2 (save/load) is complete**; glTF/OBJ follow in Phase 5 |

**Acceptance:** save → quit → open reproduces the scene exactly (names, order,
transforms, flags, kinds, time settings); malformed files produce an error dialog,
not a crash; unsaved changes are never lost silently.

---

## Phase 3 — Geometry core, evaluation, offscreen viewport (v0.3)

**Goal:** objects carry real geometry; primitives get segment parameters; an
object can be converted to an editable polygon object; the renderer draws
arbitrary meshes efficiently; the viewport renders into its own target.

### 3A. `grid4d-geom` — mesh data

* `PolyMesh`: `points: Vec<Vec3>`, faces as **n-gons** in face-vertex form
  (`face_starts: Vec<u32>`, `face_points: Vec<u32>`) — modelling tools (extrude,
  knife, bevel) create n-gons, so support them from day one.
* Attributes: per-corner UV (`Option<Vec<Vec2>>`), per-face smoothing via a Phong
  angle (normals computed, not stored).
* `Topology` (half-edge / edge table) **built on demand** and cached by mesh
  revision for operations that need adjacency; plain arrays stay the storage format.
* `triangulate(&PolyMesh, &mut TriBuffer)` (fan for convex, ear-clipping for
  concave n-gons) — reuses the caller's buffer.
* Primitive generators move here from `render/src/mesh.rs`, now with C4D-style
  parameters: Cube (size, segments X/Y/Z, fillet later), Sphere (radius, segments,
  type), Cylinder (radius, height, rotation/height/cap segments), Plane (width,
  height, segments), plus Cone, Torus, Disc, Capsule (the menu already lists them).
* Component selections stored as bitsets on the mesh (`points`, `edges`, `polys`).

### 3B. Scene & evaluation

* `ObjectKind::Polygon { mesh: Arc<PolyMesh> }`. Edits use `Arc::make_mut`
  (copy-on-write); an undo step for a mesh operation stores the *previous* `Arc` —
  cheap and simple for v0.3, optimise to deltas only if profiling demands it.
* "Make Editable" (`C`): `EditorCommand::MakeEditable` replaces a primitive kind with
  `Polygon` from its generator (one undo step).
* `Node::revision: u64`, bumped by every edit to that node (in `Document::apply`
  and the direct-write paths).
* **`grid4d-eval`**: `Evaluator::evaluate(&Document, &mut EvaluatedScene)` produces,
  per visible node, world matrix, visibility, selection-independent draw data and a
  `GeometryHandle` (primitive params hash, or mesh `Arc` pointer + revision).
  Caches are keyed by revision, so steady state allocates nothing. Render and
  picking read `EvaluatedScene` instead of walking `Scene`.

### 3C. Renderer upgrade

* Instanced batches are keyed by `GeometryHandle` instead of the closed
  `MeshKind` enum: identical primitives (same params) still share one GPU buffer
  and draw instanced; each editable mesh gets its own buffer.
* `GpuMeshCache`: `HashMap<GeometryHandle, GpuMesh>` with last-used frame; upload
  only on new handle; evict unused entries every N frames; buffers grow
  geometrically (existing rule).
* **Offscreen viewport target:** render into a per-viewport colour + depth (+ MSAA
  resolve) texture registered with `egui_wgpu::Renderer::register_native_texture`,
  then shown as an egui image. Unlocks: overlays with their own depth (points/edges
  in Phase 4), wireframe passes, ID-buffer picking if ever needed, multiple viewports
  with independent settings (Phase 7), post effects. `gpu_view.rs` is the only
  egui-wgpu adapter. Keep the size test pattern for any new uniform/instance struct.
* Picking: per-object BVH over triangles (`grid4d-geom`), built lazily per geometry
  revision; object pick = ray vs. instance AABB, then BVH for exact hit.

### Steps

| Step | Size | Work |
|------|------|------|
| 3.1 ✅ | M | `grid4d-geom` crate: `PolyMesh`, triangulation, normals, tests (Euler characteristic, winding — port `winding_matches_normals`) — done 2026-10-18: CSR face-vertex storage with validation, Newell face vectors, `Topology` (edges, edge/point → faces, closed, manifold & orientation, Euler), `Smoothing` (flat / smooth / Phong angle 80°), `Triangulator` (shorter-diagonal quads, ear clipping with fan fallback, per-corner vertices, reused buffers) |
| 3.2 ✅ | M | Parametric primitives (+ Cone, Torus, Disc, Capsule); Attribute Manager segment fields; render consumes generated meshes — done 2026-10-18: `grid4d_geom::primitives` (lathe-based round shapes, welded cube lattice), segment fields on every primitive, `.g4d` **format v2** with v1 migration, and the renderer's part of 3.4 (`MeshKey`-keyed batches + GPU mesh cache with eviction); `grid4d file.g4d` opens a file at start-up |
| 3.3 ✅ | L | `grid4d-eval` + `EvaluatedScene`, node revisions; render/picking migrate; `extract_reuses_capacity`-style test for evaluation — done 2026-10-18: process-wide unique scene/node revisions (bumped by `get_mut` and structural edits), `EvaluatedScene::update` skips unchanged scenes, `MeshKey`/`Drawable`/`PickShape` live in eval, render/picking/brackets read it; the Attribute Manager edits a copy so idle frames never bump the revision (guarded by a test over every panel) |
| 3.4 ✅ | S | ~~`GeometryHandle`-keyed batches + `GpuMeshCache`~~ — done early in 3.2 for primitives (`MeshKey`); finished in 3.6: `MeshKey::Polygon(MeshId)`, the batch carries the `MeshHandle` the renderer uploads once per id |
| 3.5 ✅ | M | Offscreen viewport target (colour/depth/MSAA, resize handling) — done 2026-10-19: `grid4d_render::ViewportTarget` (`Rgba8Unorm` gamma colour because egui samples textures as gamma, 4× MSAA when supported, `Depth32Float`, transparent clear over the UI's gradient), `ViewportRenderer::render` submits its own pass, `ui/viewport/gpu_view.rs` registers and re-points the egui texture on resize; the paint callback, `Arc<Mutex<FrameData>>` and the window's depth/MSAA are gone. Checked on screen, maximized and resized |
| 3.6 ✅ | M | `ObjectKind::Polygon`, Make Editable (`C`) per [`reference/modeling-modes.md`](reference/modeling-modes.md) §2 (incl. Cylinder/Cone caps with their own point rings), BVH picking, `.g4d` **v3** (zip + binary geometry) with migration (v2 was taken by segment counts); v3 may also change the `"format"` tag from `"oc4d"` to `"grid4d"` — done 2026-10-19: `scene::MeshHandle` (shared immutable mesh, unique `MeshId`, cached bounds, lazy BVH), `EditorCommand::MakeEditable` (`C`, Mesh ▸ Conversion, top of the left palette; one undo step, same `NodeId`, size baked in), separate cylinder/cone caps in the generator, `geom::Bvh` (median split, boundary-inclusive ray tests) for exact picking, **format v3** = zip with `document.json` (tag `"grid4d"`) + little-endian `meshes/<n>.bin`, shared meshes stored once, v1/v2 JSON read-only; golden `demo_v3.g4d`. Measured (release, ignored test `a_million_triangles_upload_and_pick_quickly`): 1 M triangles tessellate in ~0.1 s, the first pick builds the BVH in ~0.35 s, then < 1 µs per pick. Open for Phase 4: Optimize (the spec's cylinder criterion is checked by welding in tests), and where component selections go in the file (an optional field in v3, omitted when empty, or v4) |

**Acceptance:** 10 000 cubes and a 1 M-triangle mesh render at interactive rates
(see performance budget); editing a primitive's segments updates instantly;
Make Editable + undo round-trips; steady-state frames allocate nothing in
eval/extract/prepare/paint.

---

## Phase 4 — Polygon modelling & Subdivision Surface (v0.4)

**Goal:** the C4D component workflow on polygon objects.

**Behaviour specs** (parameters, edge cases, acceptance tests — read before
implementing): [`reference/modeling-modes.md`](reference/modeling-modes.md) (3.6,
4.1–4.3) and [`reference/generator-subdivision-surface.md`](reference/generator-subdivision-surface.md) (4.7).

### Design

* `EditMode::{Points, Edges, Polygons}` become functional for the active
  `Polygon` object. Component display drawn by the offscreen viewport with depth
  test (points as screen-space quads, edges as lines, selected faces tinted).
* Selection tools: Live Selection (brush radius, "only visible elements" via
  depth/BVH occlusion), Rectangle Selection, Loop and Ring selection, Grow/Shrink,
  Select Connected, Invert. Component selection changes are undoable.
* Gizmo operates on the selected components (pivot = selection centre, axes =
  object/world/normal).
* Mesh operations live in `grid4d-geom::ops` as pure functions
  `fn extrude(mesh: &PolyMesh, sel: &PolySelection, params) -> PolyMesh`, each with
  topology-invariant tests; UI wraps them as interactive tools with a parameter
  panel in the Attribute Manager ("tool mode", like C4D).
* Generator framework in `grid4d-eval`: a generator object takes its children as
  input and hides them in the editor (C4D behaviour). First generator:
  **Subdivision Surface** (Catmull-Clark, editor/render levels), using the Phase 3
  caches.

### Tool order (by modelling value)

| Step | Size | Work |
|------|------|------|
| 4.1 ✅ | M | Component modes, display, picking of points/edges/polys — done 2026-10-19 per [`reference/modeling-modes.md`](reference/modeling-modes.md) §3–§5: `geom::BitSet` + `ComponentSet` (edges by corner), `selection::convert` (tolerant/strict), `Topology::edge_corners`; `scene::Components` (copy-on-write selected + hidden sets next to the `MeshHandle`, saved in `.g4d` v3 as optional index ranges — v3 grows additively, `demo_v3_initial.g4d` keeps loading); `render::pick_component` (nearest visible point/edge in screen space with BVH occlusion, polygon by ray) and the overlay (`overlay.wgsl`: point squares, pixel-width edges, polygon tint, pulled towards the eye; hover in its own tiny buffers); UI: active object only, click/Shift/Ctrl/empty-space selection as undo steps, Shift/Ctrl+click on a mode converts, Select All/Deselect All on components, gizmo hidden and Delete inert in component modes until 4.3/4.5 (the gizmo since 4.3). Checked on screen. Hover picking projects every point per pointer move — fine for typical meshes, revisit (screen grid) for million-point meshes |
| 4.2 ✅ | M | Live / Rectangle selection, Loop / Ring, Grow / Shrink / Invert / Connected — done 2026-10-19 per [`reference/modeling-modes.md`](reference/modeling-modes.md) §5: pure algorithms in `geom::selection` (grow/shrink with edge- or point-adjacency, connected, invert, hidden closure) and `geom::loops` (edge loop with Stop at Selections/Non-Quads/Poles; edge ring through quads and, unless stopped, around triangle fans; polygon strips); `render::components_in_rect` (tolerant/visible-only) and `paint_brush` + `BrushCoverage` (≥ 75 % of an edge's/polygon's samples unless tolerant); viewport `select_tools.rs`: Live strokes as one `CommitLive` step with Esc cancel and middle-drag radius, Rectangle frames (objects in Model mode), Loop/Ring with hover preview, double-click loop/connected; Select menu with Invert, Connected, Grow/Shrink (+ Grow Using Edges Only), Hide Selected/Unselected, Unhide All; Attribute Manager Tool mode with each tool's options. Checked on screen (loop, ring strip, rectangle on an editable sphere). **Not bound:** the tools' shortcuts (secondary sources agree on U~L/U~B for Loop/Ring, conflict on the rest; Maxon's pages list none) — verify, then bind with a chord state machine. Our readings of the reference: with Stop at Poles off, a loop continues as straight as possible through poles surrounded by quads; Shrink drops members with a neighbour outside the selection (a fully selected plane does not shrink) |
| 4.3 ✅ | M | Component transform via gizmo + undo — done 2026-10-19 per [`reference/modeling-modes.md`](reference/modeling-modes.md) §6: `geom::selection::points_of` (points of a point/edge/polygon selection); `scene::MeshHandle::with_points` (same faces, new `MeshId`, the `Topology` shared, the BVH rebuilt on demand); the gizmo appears in component modes at the bounding-box centre of the selected points (none when nothing is selected) with object axes, or world axes with the World toggle (Scale stays in object space as for objects); a drag keeps a `PointEdit` (original mesh, affected points, world matrix, pivot) and rewrites the moved points from the original each frame — Move translates, Scale and Rotate act around the pivot — leaving the selection untouched, one `CommitLive` step on release ("Move Points", "Scale Polygons", …); *Tweak Mode* (Move tool option in the Attribute Manager, off by default): with nothing selected, dragging a component moves just it in the view plane, selection unchanged, Esc restores; the renderer drops a polygon object's GPU mesh and batch as soon as a frame does not draw them, so the meshes of a drag do not pile up. Checked on screen (rectangle-selected points, gizmo at their centre); the drag itself by headless egui tests. Each drag frame re-tessellates the whole mesh — fine for typical meshes, revisit (update only the moved vertices) for very large ones. Normal axes and numeric entry wait for the Coordinate Manager |
| 4.4 ✅ | M | Extrude, Inner Extrude (interactive drag + numeric) — done 2026-10-19 per [`reference/modeling-tools.md`](reference/modeling-tools.md) (new spec): `geom::ops` — `pieces` (Preserve Groups + Maximum Angle), `extrude_polygons` (mitred directions so every face moves the full Offset, walls with Subdivision / N-gons, Caps; a point only the piece uses moves itself, outline points get one copy per outline pass, so the result stays manifold), `extrude_edges` (border edges only, Rotation, chains share points, the new outer edges become the selection) and `inset` (mitred inner outline, rings, N-gons); existing point and face indices are kept, so selections survive. `PolyMesh::with_faces_where`. UI: `Tool::Extrude` (D) and `Tool::Inset` (I, Polygons mode), the reference's shortcuts (Maxon Quick Start); Mesh menu entries; options in the Attribute Manager's Tool mode with Apply (`EditorCommand::ApplyMeshTool`); viewport drag left/right at the object's depth, rebuilt from the press each frame, one `CommitLive`, Esc cancels, a click still selects; nothing selected = every visible component, hidden polygons untouched. Checked on screen (Apply: extrude → inset → extrude). **Later:** Offset Variation, point-mode extrude (Bevel), live re-adjusting of the last operation. Also from testing the app on screen: hidden polygons are now left out of the shaded display and object picking (a cached shown mesh in `grid4d-eval`), and a camera created at the view no longer fills it with its marker |
| 4.5 ✅ | M | Loop/Path Cut, Knife (line), Connect points/edges, Dissolve, Delete — done 2026-10-20 per [`reference/modeling-tools.md`](reference/modeling-tools.md) (spec extended; edge Delete settled: the edges go with their polygons): `geom::ops` split into `extrude.rs`, `rebuild.rs` (`delete`, `melt` — merged regions must have one outline, border edges cannot melt —, `dissolve` = melt + removing points left between two edges) and `cut.rs` (`connect` — chosen corners joined around each face —, `loop_cut` across an edge ring of quads with the reference side carried along, end faces gain the points, `line_cut` — the plane through the eye and the line, limited to its wedge unless Infinite, visible-only test from the caller, crossings paired along the line); they return a `Rebuilt` mesh with point and face origins and the new edges, and `Rebuilt::carry` moves selections and hidden flags across. UI: Delete (Del/⌫) in component modes, Mesh ▸ Connect / Melt / Dissolve, `Tool::LoopCut` (preview under the cursor, click cuts at the cursor, Number of Cuts evenly spaced, Select Cuts) and `Tool::LineCut` (drag, Esc cancels; Visible Only, Infinite Cut, Connect Cut Edges, Select Cuts); shortcuts from Maxon's list: `0`, `Ctrl+⌫` Dissolve, `Alt+⌫` Melt and a chord state (`U~L/B/W/Y/K/I/Z`, `K~L/K`, `M~M/N/T/W/L/K`) shown in the status bar. Checked on screen: K then L, a loop cut by click with its edges selected, Dissolve back, undo, Delete of a corner point. **Later:** Path mode and Edge Distance for Loop/Path Cut, multi-point lines and Split/Remove for Line Cut, Dissolve with Shift (Phong angle), UV interpolation beyond linear |
| 4.6 ✅ | M | Bevel (edges/points, offset + subdivision), Bridge, Weld, Optimize — done 2026-10-20 per [`reference/modeling-tools.md`](reference/modeling-tools.md) (spec extended): `geom::ops::bevel` (bevelled edges between two faces; every endpoint and bevelled point replaced by points sliding the offset along the other edge, or into the face where both of a corner's edges are bevelled; strips of Subdivision + 1 quads along a quadratic profile pulled by Depth; Limit clamps the offset; the hole at each replaced point filled from the edges no face has from the other side), `bridge_polygons` (pairing in opposite order with the shortest distance, Spin Steps, Subdivision, Delete Original Polygons; reversed band when the originals stay) and `bridge_edges` (a quad, or a triangle for edges sharing a point), `weld` and `optimize` (grid hash within Tolerance, unused points; collapsed faces removed). UI: `Tool::Bevel` (M~S, drag or Apply like Extrude; Edges and Points mode), `Tool::Bridge` (B / M~B, drag between two polygons or border edges, Apply on the two selected), `Tool::Weld` (click a selected point, or elsewhere for the centre), the Optimize dialog (U~O, Mesh menu); new polygons/edges selected via `modeling::Fresh`. Checked on screen: M~S and the Bevel options, the Optimize dialog and documents built with the operations (rounded bevel, bridged hole, weld). **Later:** Bevel in Polygons mode, Solid/Proportional/Radial modes and shaping; flat n-gon corner patches could become rounded grids; Bridge point by point and chaining; Weld with Shift/Ctrl clicks |
| 4.7 ✅ | L | Generator framework + Subdivision Surface generator — done 2026-10-20 per [`reference/generator-subdivision-surface.md`](reference/generator-subdivision-surface.md): `geom::subdivision` (`subdivide` with `SubdivisionOptions { levels, linear_pre, boundary }`: Catmull-Clark on n-gons — face points, edge points, the interior / border / corner rules, non-manifold edges treated as borders —, bilinear Linear Pre-Subdivision, linear UVs, levels clamped to `MAX_LEVEL` 6); `ObjectKind::SubdivisionSurface { viewport_levels, render_levels, linear_pre, boundary }` (`ObjectKindTag::GENERATORS`, `is_generator`), `.g4d` v3 grows additively (`KindDto::SubdivisionSurface`, levels clamped on load); `grid4d-eval::generator` — the first child is the cage, merged with its visible descendants in the generator's space (mirrored inputs re-wound), the result cached per generator and rebuilt only when the inputs (mesh ids / primitive parameters + relative matrices) or options change; cage nodes stay evaluated (component modes, framing) without drawable or pick shape; a disabled generator acts as a group; `generator_mesh` for Make Editable. UI: original icon, toolbar button and Create ▸ Generator (Alt-click = `WrapInGenerator`: the generator takes the active object's place, the object keeps its transform, one undo step), Attribute Manager parameters, `Q` = `ToggleParentGenerator` (the selected generator or the nearest one above; verified in Maxon's list), Make Editable bakes enabled generators at the viewport level and detaches the cage. Checked on screen: the smoothed cube, editing the cage in Points mode, Q, the menu. **Later:** edge weights/creases, OpenSubdiv types, UV options, the renderer level (Phase 6), a benchmark, buffer reuse between levels |
| 4.8 ✅ | S | Snapping (grid, point, edge, polygon centre) — done 2026-10-20 per [`reference/modeling-snapping.md`](reference/modeling-snapping.md) (new spec): `grid4d-render::snap` — `find_snap(&EvaluatedScene, &ScreenView, &mut SnapCache, &SnapQuery)` looks around the cursor for vertices, edges (closest point), edge mid points, polygon centres, the surface (ray cast), object axes and the floor grid's points and lines at the current spacing; point-like targets beat line-like ones, which beat surfaces; primitives' unit meshes cached per `MeshKey`; the moved subtree, the moved points with their edges/polygons and the generators above them are skipped; `snapped_position` puts the element on the target (3D) or on the view ray through it (2D), along the dragged axis when constrained (rays in f64). UI: `SnapSettings` (enabled, Auto/2D/3D, radius, targets) replaces `EditorState::snapping`; gizmo Move drags and Tweak keep the unsnapped offset and snap it each frame (`viewport/snapping.rs`), the target is highlighted with its name (`overlay::snap_target`); Shift+S (verified in Maxon's list), the palette button (click toggles, right-click menu), Tools ▸ Snap and the Move tool's Snap group. Checked on screen with real-cursor drags (grid point and edge mid point snaps). **Later:** Quantize (Shift+Q), per-tool settings, workplane, guides, intersections, perpendicular, snapping in other tools |

Shortcuts follow Cinema 4D where one exists (e.g. `C` Make Editable, `D` Extrude,
`I` Inset — both bound since 4.4 —, `K~L` Loop/Path Cut, `M~S` Bevel, `U~L` Loop Selection) —
verify each against Maxon's public documentation before binding; two-key chords use
the chord state in `shortcuts.rs` (since 4.5).

**Acceptance:** model a simple object (box → extrude → loop cuts → bevel → SubD)
entirely with tools, undo each step, save and reload it. ✅ 2026-10-20 — the headless
test `phase_4_acceptance_model_undo_redo_save_and_reload` (viewport tests) does exactly
this with the real tools (Apply, a loop cut click, Alt-create), undoing and redoing
every step and reloading the saved archive.

---

## Phase 5 — Interchange (glTF, OBJ) (v0.5)

| Step | Size | Work |
|------|------|------|
| 5.1 ✅ | M | glTF 2.0 import (`gltf` crate): hierarchy, TRS → HPB, meshes → `Polygon`, cameras, `KHR_lights_punctual`; unit conversion (glTF metres → cm ×100) and Y-up already match — done 2026-10-20 per [`reference/interchange-gltf.md`](reference/interchange-gltf.md) (new spec): `grid4d-io::import` (`Objects { nodes }` in pre-order with parent indices, `from_document` / `into_document`) and `gltf_import` (the `gltf` crate without its image/import features; `.glb` chunk, base64 data URIs and relative files read by us; triangles, strips and fans merged per mesh with identical positions welded, `TEXCOORD_0` per corner, base colour → display colour, shared meshes, cameras, lights, empty nodes → Nulls, matrices decomposed, cycles cut, non-finite data rejected). UI: File ▸ Open accepts glTF (a new untitled document named after the file), File ▸ Merge… (Ctrl+Shift+O, verified in Maxon's list) and dropped glTF files add the objects as one undo step (`file::merge`), `.g4d` files merge too; dialog filters for both formats. Checked on screen with a generated scene (open, merge of the `.glb`). **Later:** an import dialog (scale, toggles), materials/textures, animation, skins, Draco |
| 5.2 ✅ | M | glTF 2.0 export (`.gltf`/`.glb`): primitives exported via their generated meshes; generators exported evaluated — done 2026-10-20 per [`reference/interchange-gltf.md`](reference/interchange-gltf.md) (export section): `grid4d-io::gltf_export` (`export_gltf` by extension, `glb_bytes`, `gltf_document`; JSON via `serde_json` without empty arrays; meshes triangulated by `grid4d_geom::Triangulator` with the viewport's smoothing for normals, corners sharing position/normal/UV merged into one vertex, u16 or u32 indices, one glTF mesh per (mesh, colour), materials from display colours, perspective cameras, `KHR_lights_punctual` point lights, cm → m, writes through temporary files). `Imported` renamed `Objects` (both directions). UI: `file::exportable` bakes primitives (`editable_mesh`) and switched-on generators (`generator_mesh`, cage left out; switched-off ones → Nulls); File ▸ Export… (`EditorCommand::Export`, `FileDialogs::pick_export` with `.glb`/`.gltf` filters; the document is not touched). Checked in the app: exports validated by an independent script, reopened and merged. Also fixed in the importer: `-0.0` and `0.0` weld. **Later:** export dialog (scale, visible/selected only), materials/textures, animation |
| 5.3 ✅ | S | OBJ import/export (`tobj` for import; writer by hand) — done 2026-10-20 per [`reference/interchange-obj.md`](reference/interchange-obj.md) (new spec): `grid4d-io::obj` — `import_obj` (`tobj` without default features, faces kept with their corners, separate indices; objects per `o`/`g` at the origin, 1 unit = 1 cm, per-corner UVs with V flipped, a constant stand-in UV map dropped, `Kd` from the `.mtl` read by relative path only, repeated corners dropped, bad indices / non-finite numbers rejected) and `export_obj` / `obj_text` (one `o` per polygon object in world space, mirrored objects re-wound, `vt` and `vn` written once per value, normals from the viewport's smoothing, `.mtl` with one material per colour). `import` / `export` dispatch by extension (`IMPORT_EXTENSIONS`, `EXPORT_EXTENSIONS`); the UI's dialogs, drop handling and File ▸ Export take `.obj`. Exported generators now use their cage's colour (as the viewport). Checked in the app: export validated by an independent script, reopened, merged; the table scene's world positions match to 3·10⁻⁶ cm. **Later:** import/export options (scale, axis flips, splitting, flip faces) |
| 5.4 | S | Later in Phase 6: materials (PBR metallic-roughness) and animation tracks in glTF |

FBX needs a proprietary SDK → out of scope. USD is attractive but its Rust
ecosystem is immature → revisit after v1.0.

**Acceptance:** glTF sample models (Khronos sample assets: Box, Duck, …) import
correctly; export → re-import round-trips hierarchy, transforms and geometry.
Round trips ✅ 2026-10-20 (tests and in-app checks for glTF and OBJ); the Khronos
sample models are not in the repository yet (a decision on test assets is pending),
generated scenes cover the same features.

---

## Phase 6 — Animation, tags, materials, managers (v0.6)

### 6A. Tags

`Node::tags: Vec<Tag>` with `enum TagKind { Material(MaterialId), Phong { angle }, Display, Protection, Target(NodeId), … }`.
Object Manager shows tag icons on the right of each row; the Attribute Manager
`Mode ▸ Tags` becomes functional. Tags are edited through `SceneEdit` (undoable).

### 6B. Animation data (in `grid4d-scene`)

* `ParamId` — a typed, closed address of an animatable parameter
  (`ParamId::Position(Axis)`, `Rotation(Axis)`, `Scale(Axis)`, `Kind(KindParam)`,
  `Visibility`, …) — no string paths.
* `Track { node: NodeId, param: ParamId, curve: Curve }`,
  `Curve { keys: Vec<Key> }`, `Key { frame: f64, value: f32, interp: {Step, Linear, Bezier}, tangents }`.
* Evaluation in `grid4d-eval` writes animated values into the `EvaluatedScene` — the
  authoring data is not overwritten by playback. Attribute Manager shows the
  evaluated value and a keyframe indicator (filled = key on this frame, outlined =
  animated).
* Separate **document range** and **preview range** in `TimeState`.

### 6C. UI

| Step | Size | Work |
|------|------|------|
| 6.1 ✅ | M | Tags data + UI — done 2026-10-20 per [`reference/tag-basics.md`](reference/tag-basics.md) (new spec): `grid4d-scene::Tag` (`Phong { angle_limit, angle }`, `Protection { position, scale, rotation }`, `Target { target, up, pitch }`) in `NodeData::tags` (edits are `SetData`: undo and save for free), readers via the `Tags` trait (`smoothing_angle`, `protection`, `aim`, `tag_index`); `.g4d` v3 `TagDto` (additive, object references by index, resolved after load); Phong → `grid4d-eval::Shading` in `Drawable`, the renderer's batches and GPU meshes keyed by (`MeshKey`, `Shading`), exports use the angle; Target evaluated (`apply_aims`: world matrix turned so −Z faces the target, a correction passed to the subtree); Protection hides the gizmo of protected channels and greys the Coordinates fields. UI: tag icons in the Object Manager (click selects → `EditorState::tag_selection`, valid while its object is active), Tags menu and *Add Tag* context menu (`EditorCommand::AddTag`), Delete / *Delete Tag* (`EditorCommand::DeleteTag`), the tag view in the Attribute Manager (object links as lists). Checked in the app. **Later:** the Material tag (6.6), Protection limits, Phong edge breaks, copying tags, the gizmo showing a Target-turned orientation |
| 6.2 ✅ | M | Tracks/curves/keys + evaluation + tests (interpolation, tangent continuity) — done 2026-10-20 per [`reference/animation-keys.md`](reference/animation-keys.md) (new spec): `grid4d-scene::anim` — `Param` (`Position`/`Rotation`/`Scale` × `Axis`), `Key { frame, value, interpolation, tangents }` (Spline / Linear / Step; Auto = clamped Catmull-Rom slopes with ⅓ handles, or Manual handles), `Curve` (sorted, one key per frame, `set_key`, `remove_key`, `value_at` with Bézier segments solved by bisection, constant before/after), `Track`, the `Tracks` reader (`animate`); tracks live in `NodeData::tracks` (one per parameter), so edits are `SetData`. `grid4d-eval`: `EvaluatedScene::set_frame` (re-evaluates only if animated), world matrices recomputed in pre-order with animated transforms before Target tags; the viewport passes the current whole frame. `.g4d` v3 `TrackDto` (additive; defaults left out; non-finite keys refused). Checked in the app (bouncing ball, turning parent). Deviation: tracks per object rather than a document-level list (see the spec). **Later:** preview range (with 6.3's timeline work), before/after modes (repeat, oscillate), animating object parameters (`Kind`), generator cages following animation |
| 6.3 ✅ | M | Record Active Objects (`F9`), Autokeying, key indicators in the Attribute Manager, keys drawn on the timeline (select, move, delete) — done 2026-10-20 per [`reference/animation-recording.md`](reference/animation-recording.md) (new spec): the UI shows and edits values at the current frame (`NodeData::transform_at`, `Scene::world_matrix_at` for the gizmo); `NodeData::set_transform_at` writes a key (Autokeying, Ctrl+F9, red view frame) or a temporary value (`NodeData::unkeyed { frame, params }`, in the base transform, never saved, dropped by `commands::apply_pending` when the frame changes and after undo/redo); `EditorCommand::RecordKeys { at }` / `DeleteKeys` / `MoveKeys { by }` built in `ui/src/keys.rs` (one `commit` each); record channels (Animate ▸ Record) and the key selection are editor state; key buttons in the Coord. tab (`KeyState`: grey / red ring / red dot / yellow; click toggles a key, Shift+Ctrl+click deletes the track); the timeline's key strip (select, Shift, band select, drag to move, Delete after a click in it, context menu, Ctrl+click records at a frame); Ctrl+F / Ctrl+G previous / next key (verified in Maxon's shortcut list); the gizmo keeps rotation angles continuous. Checked in the app. **Later:** the preview range, copy/paste and scaling of keys, per-channel key rows, Frame Scene/Selected at the animated positions |
| 6.4 ✅ | L | New panel `fcurve.rs` — F-Curve editor (pan/zoom, tangents, interpolation menu); dock as a tab beside the timeline — done 2026-10-20 per [`reference/animation-fcurves.md`](reference/animation-fcurves.md) (new spec): Timeline | F-Curve tabs (`EditorState::bottom`, `widgets::bottom_tabs`; Tab over the panel, taken out of the raw input before egui's focus navigation by `shortcuts::take_tab`), the F-Curve panel resizable; track list (filter: `EditorState::curves.tracks`), graph with grid, value gutter, frame ruler (scrubs) and the current frame; keys selected by click, Ctrl, box or curve click (`CurveSelection`), moved in whole frames / free values (Shift constrains), written live and recorded with the new `EditorCommand::CommitLiveAll`; tangent handles on selected keys (manual, mirrored slope; Shift breaks); Ctrl+click adds a key on the curve; header fields for one key; context menu → `EditorCommand::EditCurveKeys { op }` (Interpolation, Auto / Zero Angle / Zero Length via `Curve::apply_tangent_preset`, Delete); wheel zoom around the cursor (Ctrl time, Shift values), middle / Alt drag pans, H / S frame. Rotations shown in degrees. Checked in the app. **Later:** Ease presets, Ctrl/Alt handle constraints, mirroring, quantize, normalised curves, copy/paste |
| 6.5 ✅ | M | New panel `coordinates.rs` — Coordinate Manager (P/S/R, Object/World/Size modes, Apply) — done 2026-10-20 per [`reference/manager-coordinate.md`](reference/manager-coordinate.md) (new spec): a tab beside the Attribute Manager (`EditorState::managers`, `widgets::manager_area_tabs`; Shift+F5 / Shift+F7 and the Window menu, verified in Maxon's list); Position / Scale·Size·Size+ / Rotation in Object or World space at the current frame, written through `set_transform_at` (Autokeying, temporary values); only the columns that changed are applied, so world values that do not round-trip change nothing; sizes are reached through the scale; component modes edit the selection's centre, bounding-box size and a relative rotation (`MeshHandle::with_points`); live writes with one `CommitLive` per interaction, Apply as one `Edit`. Checked in the app. **Later:** frozen coordinates, primitives' size parameters, several objects at once |
| 6.6 ✅ | M | Materials: `Material { name, base_color, roughness, metallic, emission, … }` in the document; new panel `materials.rs` (Material Manager, thumbnails rendered later); Material tag; viewport uses base colour (then PBR in Phase 7) — done 2026-10-21 per [`reference/manager-material.md`](reference/manager-material.md) (new spec): `grid4d-scene::Material` (colour, roughness, metallic, emission) in an ordered `Materials` table with stable `MaterialId`s, kept in the `Scene` (revision) and edited whole through `SceneEdit::SetMaterials`; `Tag::Material`, `Tags::material`, `Scene::effective_material` (inheritance); evaluation colours objects by their own or inherited material (markers excepted; generators: their own, else the cage's); `.g4d` v3 `materials` + `TagDto::Material` (additive); glTF import/export and OBJ import/export carry materials (`Objects::materials`, `effective_materials`), and `Objects::sources` lets Merge remap Target references too (a bug since 6.1: merged Target tags kept ids of the other document); the Material Manager is a bottom tab (Shift+F2; New, Delete, Apply; double-click / Ctrl+N; drag onto the Object Manager or the viewport → `EditorCommand::AssignMaterial`), the Attribute Manager shows the selected material (live edits, one step each) and the Material tag's link, the Object Manager shows material tags in their colour. Checked in the app. **Later:** several Material tags (layering), projection, textures, rendered previews, PBR viewport (Phase 7) |

**Acceptance:** keyframe a bouncing ball (position + scale), edit curves, play
back at document fps, save/reload, export to glTF with animation.
Status 2026-10-21: all but the last part — glTF export does not write animation
yet (planned, with glTF animation import).

---

## Phase 7 — Viewport: cameras, lights, 4-view, shading (v0.7)

| Step | Size | Work |
|------|------|------|
| 7.1 ✅ | M | Scene cameras: Cameras ▸ use scene camera, look-through, camera `fov` used; camera frustum drawn in the viewport — done 2026-10-21 per [`reference/object-camera.md`](reference/object-camera.md) (new spec): `OrbitCamera::roll` (bank), `from_world` / `eye_matrix`; `EditorState::view_camera` (Cameras ▸ Default Camera / Use Camera ▸ … / Selected Object as Camera, double-click a camera's icon in the Object Manager); the viewport follows the linked object at the current frame (evaluated world: animation, Target tags) with its field of view, and writes navigation (incl. H / O / Reset View) back through `set_transform_at` (temporary values, Autokeying), one `CommitLive` ("Move Camera") per gesture; Protection holds it; leaving restores the editor camera; the view's title names the object; wire frustums for the other cameras (`overlay::camera_frustums`); new cameras clone the view with bank and field of view. Checked in the app. **Later:** saving the linked camera with the document (per view, with 7.2), render resolution aspect, focus distance / DOF handles |
| 7.2 ✅ | M | 4-view layout (Perspective, Top, Front, Right) with orthographic cameras; Panel menu arrangements; each view its own offscreen target and settings (from Phase 3C) — done 2026-10-21 per [`reference/viewport-layouts.md`](reference/viewport-layouts.md) (new spec): `Projection::Parallel` on `OrbitCamera` (orthographic matrix, eye far back, parallel rays; `half_height` = the zoom), `ViewKind` presets (Perspective, Top, Bottom, Front, Back, Left, Right) in each view's Cameras menu; `ViewportPanel` holds four `ViewState`s and an `Arrangement` (Single, 2 Stacked, 2 Side-by-Side, 4 Views; Panel menu, F1–F4 alone / F5 all, middle click toggles alone); only the active view (thin frame) takes input, a press or scroll elsewhere activates that view in the same frame; one renderer, one offscreen target per view (`GpuView`); the grid lies in the view's plane (`GridPlane`, shader flags in `light_dir.w`), grid snapping uses it; component picking and snapping use parallel rays (`ScreenView::parallel`); overlays pulled a fixed distance in parallel views. Checked in the app. **Later:** three-view and other four-view arrangements, the Parallel/axonometric projections, saving views with the document, camera links in every view |
| 7.3 ✅ | M | Display modes: Gouraud, Gouraud + Lines, Quick Shading, Wireframe, Box; per-view — done 2026-10-21 per [`reference/viewport-display.md`](reference/viewport-display.md) (new spec): `grid4d-render::DisplayMode { shading: ShadingMode, style: DisplayStyle }` in `FrameData::display`, one per view (`ViewState`); shading modes Gouraud, Quick (default light only, no highlight), Constant (unlit), each with "(Lines)", Hidden Line (depth-only surfaces) and Lines (no surfaces); styles Wireframe (polygon edges, n-gons as outlines) and Box (bounding box per mesh key, shaded or as 12 edges); the renderer caches per (`MeshKey`, `Shading`, boxed) a surface and a line list, each built the first time a frame draws it; a depth-only and a line pipeline (`vs_line` / `fs_line`: pulled towards the viewer, dark over surfaces, the object's colour alone, orange when selected); polygon meshes now linger 8 renders (views with different styles draw different meshes). Display menu per view, N~A … N~G, N~H, N~K (verified in Maxon's list; `EditorCommand::SetShading` / `SetDisplayStyle` on the active view). Checked in the app. **Later:** Isoparms, Skeleton, active-object display settings, backface culling, X-Ray, textures, wide lines, picking by the box in Box mode |
| 7.4 ✅ | M | Light objects affect the viewport (storage buffer of N lights; "default light" when the scene has none); PBR-lite shading from materials — done 2026-10-21 per [`reference/object-light.md`](reference/object-light.md) (new spec): `grid4d-scene::light` (`LightType` Omni / Spot / Infinite, `LightDetails`: Use Inner, inner and outer opening angles, Falloff None / Linear / Inverse Square, Radius/Decay) in `ObjectKind::Light`; `.g4d` v3 grows additively (`light_type`, `details` omitted at their defaults, angles and radius clamped on load); glTF `KHR_lights_punctual` spot / directional both ways; `EvaluatedScene::lights` (world position, −Z direction after animation and Target tags, colour × intensity) and `Drawable::surface` (roughness, metallic, emission from the material); `FrameData` lights (`LightRaw`, 64 bytes) → a storage buffer that grows geometrically, `InstanceRaw` 160 bytes with the surface; `mesh.wgsl` sums the lights (cone smoothstep, falloff) with a normalised Blinn-Phong highlight and Schlick Fresnel, metals without diffuse, a soft ambient and studio reflection, emission added; Gouraud uses the scene's lights or the default light, Quick always the default light. Attribute Manager Type / Details, Spot cones and Infinite arrows in the view. Bug fixed on the way: colour swatches wrote their colour back every frame (0.9 → 0.90000004), so showing a light or material marked the document modified. Checked in the app. **Later:** Square Spot, Area, Parallel, IES, shadows (7.6), visible light, per-light switches |
| 7.5 ✅ | S | Filter menu (hide object types, grid, HUD per view) — done 2026-10-21 per [`reference/viewport-filter.md`](reference/viewport-filter.md) (new spec): `grid4d-render::ViewFilter` (bits per `FilterItem`: Polygon, Generator = the primitives, Subdivision Surface, Camera, Light, World Grid, World Axis, HUD; `ALL`, `NONE`, `only`), `FilterItem::of(ObjectKindTag)`, `EvaluatedNode::kind`; one filter per view replaces the grid / triad / HUD toggles (moved from Options to Filter); hidden types are skipped by `FrameData::extract`, `pick`, rectangle selection, `find_snap` (`SnapQuery::shown`), selection brackets, camera pyramids and light cones; lights still light; a selected hidden object keeps its gizmo. Filter menu: All, None, toggles, Ctrl+click keeps one. Checked in the app. **Later:** entries for future object kinds, axes / handles / guides / SDS cage / n-gon lines / horizon, Default, Geometry Only, presets, saving the filter |
| 7.6 ✅ | M | Optional: shadow map for the key light, SSAO — behind a per-view toggle — done 2026-10-21 per [`reference/viewport-effects.md`](reference/viewport-effects.md) (new spec): the light's **Shadow** (`LightShadow`: None, Shadow Maps (Soft), Raytraced (Hard), Area; `.g4d` v3 additive, omitted when None); `ViewEffects { shadows, ssao }` per view (Options menu) in `FrameData::effects`; the key light = the first light with a shadow (`FrameData::shadow_light`), its shadow map (`effects.rs`: `shadow_view` fits it to the drawn objects' world bounds — orthographic for Infinite, perspective through the cone for Spot, six 95° faces for Omni; a 2048² depth array, depth-biased pass in `shadow.wgsl` with markers clipped) sampled in `mesh.wgsl` with a 5 × 5 comparison filter (wider for soft) and a normal offset of 1.5 texels; SSAO (`ssao.wgsl`): a normal/depth pre-pass, 16 hemisphere samples with a per-pixel rotation, a 4 × 4 blur, textures per view target made on first use; it darkens the soft light fully and the direct light by 60 %. `ViewportRenderer::render` takes `&mut ViewportTarget`. Checked in the app. **Later:** several shadow-casting lights, shadow map size / attenuation / transparency, SSAO settings, the default light's shadow |

Hot-path rules apply to every new per-frame buffer (light buffer, per-view
uniforms): grow geometrically, size tests next to each `#[repr(C)]` struct.

Phase 7 is complete (2026-10-21).

---

## Phase 8 — Offline renderer (v0.8)

* New crate **`grid4d-raytrace`**: CPU path tracer (deterministic and testable
  first; GPU compute later). Uses `grid4d-eval`'s evaluated scene, `grid4d-geom` BVH,
  `rayon` for tiles.
* Materials from Phase 6 (diffuse/GGX), area/point/spot lights, camera DOF later.
* **Render View** (`Ctrl+R`): progressive render into the active viewport.
* **Picture Viewer** (`Shift+R`): new panel/window `picture_viewer.rs` with
  history, zoom, save PNG/EXR (`image`, `exr` crates).
* **Render Settings** (`Ctrl+B`): resolution, samples, output path, frame range →
  stored in the document (`RenderSettings`, persisted in `.g4d`).
* Rendering runs on a worker thread; the UI polls a channel each frame (no blocking
  in `ui()`); cancellable.

**Acceptance:** render the demo scene to PNG; reference-image tests with a
tolerance (small resolution, fixed seed) in CI.

| Step | Size | Work |
|------|------|------|
| 8.1 ✅ | L | The path tracer core — done 2026-10-21 per [`reference/render-engine.md`](reference/render-engine.md) (new spec): `grid4d-eval::Purpose` (`EvaluatedScene::for_render`: render visibility, Subdivision Renderer levels); new crate `grid4d-raytrace`: `RenderScene` (every visible surface as world-space triangles — primitives and polygon objects triangulated with their Phong smoothing — in one `grid4d-geom` BVH, barycentric shading normals, materials, lights), `RenderCamera` (a world placement, perspective or parallel; `framing`), `RenderOptions` (size, samples, bounces, seed), `render` → `Image` (linear, premultiplied; `to_rgba8` sRGB with straight alpha); paths with direct light from every light at each hit (types, cones, falloff; shadow rays per the light's Shadow: none, hard, soft over a 5 cm ball / half a degree), a diffuse + GGX material (metallic, roughness ≥ 0.05), emission, two bounces by default (lobe chosen by Fresnel/diffuse weight, light after a bounce capped against fireflies), the automatic light without lights, a transparent background; rows in parallel with `rayon`, one seeded random stream per pixel sample (identical with one thread or many); `examples/render.rs` renders a `.g4d` or the demo scene to PNG (`png`), a reference-image test (`tests/reference.rs`, `GRID4D_UPDATE_REFERENCE=1` to rewrite). Checked by rendering the demo and the test scenes. **Later:** environment/sky, textures, transparency, depth of field, motion blur, a faster BVH (SAH), tiles for progressive display |
| 8.2 ✅ | M | Render Settings (`Ctrl+B`) — done 2026-10-21 per [`reference/render-settings.md`](reference/render-settings.md) (new spec): `grid4d-scene::RenderSettings` in `Document::render` (size, Lock Ratio, `FrameRange` Current/All/Manual + step, samples, bounces, Auto Light, Save + File + Alpha Channel; `validated`, `frames`, `frame_count`, `output_path` relative to the document's folder with `.png` added), `SceneEdit::SetRender`; `.g4d` v3 `render` entry (additive, omitted while default, clamped on load); `RenderOptions::from(&RenderSettings)` and `RenderOptions::auto_light`, `Image::to_rgb8`; the example renders with the document's settings; `panels/render_settings.rs`: a non-modal window (Output, Save, Sampling, Options pages; presets; edits through `LiveEdit` + `CommitLive`), opened from the Render menu, the toolbar and Ctrl+B; File ▸ … through `FileDialogs::pick_render_output` (`EditorCommand::BrowseRenderOutput`). **Later:** several render settings per document, Preview Range, a render frame rate, film/pixel aspect, DPI |
| 8.3 ✅ | M | Picture Viewer (`Shift+R`) — done 2026-10-28 per [`reference/render-picture-viewer.md`](reference/render-picture-viewer.md) (new spec): `grid4d-raytrace::Progressive` (samples added in passes, the same picture as `render`) and `RenderControl` (cancel, progress); `grid4d-io::save_png` (`png` becomes a normal dependency of `grid4d-io`); `ui/src/rendering.rs`: `Renders` in `EditorState` — a worker thread per render over a copy of the scene at the current frame through the active view's camera (`render_camera`), passes of 1, 1, 2, 4 … 32 samples sent as 8-bit pictures, `poll` once a frame in `commands::apply_pending`, the history (512 MB), the save target decided at the start; `file::save_finished_render` / `save_render_as` (folder made, alert on failure); `panels/picture_viewer.rs`: a resizable window (Render, Stop, Save As…, Fit / 100 % / − / +, wheel zoom about the pointer, drag to pan, checkerboard behind alpha, history list, progress bar and status line); `EditorCommand::{RenderToPictureViewer, StopRender, SaveRenderAs}`, Shift+R, Render menu, toolbar, Window ▸ Picture Viewer. **Later:** a render queue, filters, channels, A/B comparison |
| 8.4 ✅ | M | Render View (`Ctrl+R`) — done 2026-10-28 per [`reference/render-view.md`](reference/render-view.md) (new spec): `rendering.rs` gets a second job beside the Picture Viewer's — `Renders::{start_view, clear_view, view_render}`, `ViewRender`, `ViewKey` (view, camera, size in pixels, scene revision, render settings, frame) — rendering the active view at its size with the settings' quality, never saved or kept; the viewport draws it over the active view (a texture, overlays on top, a status line) and clears it as soon as its `view_key` differs; `EditorCommand::RenderView`, Ctrl+R, Esc, Render menu, toolbar. **Later:** Render Region (Alt+R), re-rendering while editing (interactive) |
| 8.5 ✅ | S | Frame ranges and OpenEXR — done 2026-10-29 per [`reference/render-animation.md`](reference/render-animation.md) (new spec): `ImageFormat` (PNG / OpenEXR) and `FrameNaming` (`Name0000`, `Name.0000`, `Name_0000`) in `RenderSettings`, `output_file(document, frame)` (the format's extension replaces a typed `.png`/`.exr`), `is_animation`; `.g4d` v3 `format`/`naming` (additive); `grid4d-io::save_exr` (single-part scanline, HALF, ZIP blocks of 16 lines; `flate2` with the zlib-rs backend `zip` already uses, `half`; checked against the `exr` crate, dev only); `rendering.rs`: one worker renders every frame of the range in order (`Eye`: a linked camera object followed per frame), sends each finished frame in linear light (`FinishedFrame`), `file::save_rendered_frame` writes it at once (PNG or EXR, numbered for an animation); the Picture Viewer's single History entry counts frames; the Save page's Format and Name. **Phase 8 is complete.** **Later:** movie formats, multi-pass, 32-bit EXR, EXR from Save As, a RAM player |

---

## Phase 9 — Splines & spline generators (v0.9)

**Goal:** curves as first-class objects — drawn, edited point by point and drawn
with a pen — and the classic generators that turn them into surfaces: Extrude,
Lathe, Sweep, Loft.

**Behaviour specs** (written per step before implementing, as before):
`reference/object-spline.md` (9.1–9.3), `reference/generator-extrude.md`,
`generator-lathe.md`, `generator-sweep.md`, `generator-loft.md` (9.4–9.7),
`object-text.md` (9.8).

### Design

* **Spline data** in `grid4d-geom::spline` (glam only, like meshes): a `Spline`
  holds control points, two tangents per point (used by Bezier), a type
  (`Linear`, `Cubic`, `Akima`, `BSpline`, `Bezier`), and one or more
  **segments**, each open or closed (a rectangle with a hole, the letters of a
  text). `Interpolation` (`None`, `Natural(n)`, `Uniform(n)`, `Adaptive(angle)`
  — the default, 5° —, `Subdivided(max_length)`) samples it into a `Polyline`
  (points per segment, which samples are control points, closed flags), with
  tests on hand-built curves like `shapes.rs`.
* **Spline primitives** produce real control splines (a Circle is four Bezier
  points with tangents), so Make Editable keeps them editable: Circle,
  Rectangle (optional rounding), Arc, n-Side, Star, Helix, Flower; each with
  its plane (XY default, ZY, XZ), Reverse and the intermediate points.
  `ObjectKind::Spline { spline: SplineHandle, components }` is the editable
  spline — a shared, immutable handle with a unique id like `MeshHandle`, the
  selected points next to it like polygon objects.
* **Evaluation and drawing:** `grid4d-eval` samples each spline once per
  (source, interpolation) and caches the polyline; the evaluated node carries a
  `CurveKey` the way meshes carry a `MeshKey`. `grid4d-render` builds a line
  list once per key and draws it with the existing instanced line pipeline
  (display colour; selected and active colours like objects), never in the
  path tracer (splines are guides, not surfaces). Picking measures the screen
  distance to the polyline; framing and snapping use its points. A new
  `FilterItem::Spline`.
* **Editing** (Points mode on an editable spline): the same selection tools,
  gizmo and `CommitLive` steps as polygon points, through a shared
  "editable points" seam (`MeshHandle::with_points` ↔ `SplineHandle::with_points`);
  Bezier tangents are drawn as handles and dragged (Shift breaks them).
  Spline commands (Close/Open, Reverse Sequence, Set First Point, Hard/Soft
  Interpolation, Delete points) are pure functions returning a new spline.
* **Spline generators** extend `generator.rs`: inputs may be splines (sampled
  in the generator's space) as well as meshes. Extrude and Lathe take their
  first child spline; Sweep takes a profile (first child) and a path (second);
  Loft takes all child splines in order. Caps need polygons with holes, so
  `grid4d-geom::triangulate` gains outline-with-holes triangulation (hole
  bridging + ear clipping) with even-odd nesting of segments. Results are
  cached like the Subdivision Surface's and baked by Make Editable.
* **File format:** `.g4d` v3 grows additively — spline kinds in `KindDto`, an
  editable spline's arrays inline (points, tangents, segments).
* **Not in Phase 9:** splines in glTF/OBJ, the path tracer drawing curves
  (hair/sketch), spline masks/booleans, the Spline Smooth and Spline Pen's
  sketch modes, Formula and Profile primitives.

### Steps

| Step | Size | Work |
|------|------|------|
| 9.1 ✅ | L | Spline data, primitives and drawing — done 2026-10-29 per [`reference/object-spline.md`](reference/object-spline.md) (new spec): `geom::spline` — `Spline` (points, Bezier tangents, segments open/closed), `SplineType` (Linear, Cubic, Akima, B-Spline, Bezier — every type turned into cubic Bezier spans), `Interpolation` (None, Natural, Uniform, Adaptive — default 5° —, Subdivided) → `Polyline`, `SplinePrimitive` (`SplineShape` Circle/Ellipse/Ring, Rectangle with rounding, Arc/Sector/Segment/Ring, n-Side, Star, Helix with biases, Flower; `SplinePlane` XY/ZY/XZ; Reverse) giving real control splines; `ObjectKind::SplinePrimitive` (one variant, `ObjectKindTag` per shape, `SPLINES`); `grid4d-eval` samples each spline once per change (`CurveHandle`/`CurveId`, `MeshKey::Curve`, `PickShape::Curve`); `grid4d-render` draws curves with the instanced line pipeline in every display mode in their own colour (selected: the selection colour), `pick_curve` by screen distance, `FilterItem::Spline`; the path tracer skips them; `.g4d` v3 `spline` kind (additive; plane, Reverse, intermediate points omitted while default; counts clamped on load); glTF/OBJ keep the node only; UI: icons, Create ▸ Spline, a toolbar spline button with its list, Attribute Manager parameters, a click near the line selects. **Moved to 9.2:** the editable `Spline` kind (only Make Editable creates it) |
| 9.2 ✅ | M | Editable splines — done 2026-10-29 per [`reference/object-spline.md`](reference/object-spline.md) (extended): `scene::SplineHandle` (shared, unique `SplineId`, `with_points`, `edited`) and `ObjectKind::Spline { spline, interpolation, components }` (tag `Spline`); Make Editable bakes a primitive's control spline (plane and Reverse in the points); `geom::spline` — `smooth_tangents` (what turning a spline into Bezier gives points without handles), `without_points` (index map for hidden flags); eval caches editable splines' curves by (`SplineId`, interpolation) and exposes `EvaluatedScene::spline`; `.g4d` v3 `editable_spline` (arrays inline, zero tangents omitted, point selection/hidden ranges; counts checked on load); UI: `panels/viewport/spline_tools.rs` — points and Bezier handles drawn over the view, click / Rectangle / Live Selection on points through `SelectComponents`, handle drags (mirrored, Shift breaks, Esc restores, one `CommitLive`), the gizmo's `PointEdit` generalised to splines (tangents turned and scaled with their points); Select All / Deselect All / Invert / Delete Points on the spline target; Attribute Manager Type, Close Spline, intermediate points. The polygon tools are untouched. **Later:** hiding points, Grow/Shrink, Tweak and snapping on spline points, numeric point entry |
| 9.3 ✅ | M | Spline commands and the Pen — done 2026-10-29 per [`reference/object-spline.md`](reference/object-spline.md) (extended): `geom::spline` — `hard`, `soft` (`smooth_tangents_where`), `reverse_segments` and `set_first_points` (pure, returning old → new point maps), `segment_ranges`; Mesh ▸ Spline (`EditorCommand::SplineCommand(SplineOp)`: Hard / Soft Interpolation, Reverse Sequence, Set First Point on the active editable spline, the selected and hidden points remapped, one undo step, none if nothing changed); Close/Open stays the Object tab's Close Spline (9.2); `Tool::Pen` (Create ▸ Spline ▸ Pen, `PenSettings::kind`, Bezier by default) in `panels/viewport/pen_tool.rs`: points on the floor (perspective) or the view's plane through the origin (parallel views), a drag pulls the outgoing tangent, a click on the first point of the open segment (≥ 3 points) closes it, Esc / Enter / another tool finish (`EditorState::pen_finished`, cleared when another object becomes active), preview line to the cursor; `PenPoint` adds to the active editable spline (a new segment after a closed one) or inserts a new Spline object; each point one undo step. **Later:** moving points while drawing, the Pen snapping to objects |
| 9.4 ✅ | M | Extrude generator — done 2026-10-29 per [`reference/generator-extrude.md`](reference/generator-extrude.md) (new spec): `geom::surface` — `Extrusion` (`ExtrudeDirection` Auto / X / Y / Z / Custom / Absolute, offset, movement, subdivision 1–4000, Reverse Normals, start / end caps, `CapType` N-Gons / Triangles) and `extrude(&Polyline, &Extrusion)`: closed runs nested even-odd (outlines counter-clockwise, holes clockwise around the movement, so the solid faces outwards whichever way it was drawn), shell rings welded to the caps; `geom::triangulate_outlines` — holes bridged to the outline (Eberly) and ear-clipped; `ObjectKind::Extrude { extrusion, hierarchical }` in `GENERATORS`; `grid4d-eval::generator` generalised (`Options::{Subdivision, Extrude}`, spline inputs: primitive parameters or editable spline id + interpolation, in the generator's space; first child only, or every spline below it when hierarchical); splines inside a generator stay drawn and picked; `.g4d` v3 `extrude` kind (additive, defaults omitted, subdivision clamped); UI: icon, Create ▸ Generator, toolbar (Alt wraps the selected spline), Attribute Manager rows, filter *Generator*. **Later:** bevels, Delaunay / grid caps, UVs, cap selections |
| 9.5 ✅ | S | Lathe generator — done 2026-10-29 per [`reference/generator-lathe.md`](reference/generator-lathe.md) (new spec): `geom::surface::Lathe` (angle, subdivision 3–4000, movement, scaling, Reverse Normals, caps, caps type) and `lathe(&Polyline, &Lathe)`: the profile measured as (distance from Y, height), nested and ordered like the Extrude's (shared `profile` / `caps` / `assemble`); open profiles face away from the area between them and the axis; a full turn wraps its last ring onto the first and welds points on the axis (collapsed faces become triangles); open turns get caps on closed profiles; `ObjectKind::Lathe(Lathe)` in `GENERATORS`; `generator::Options::Lathe` (first child spline only); `.g4d` v3 `lathe` kind (additive, defaults omitted, subdivision clamped, scaling ≥ 0); UI: icon, Create ▸ Generator, toolbar, Attribute Manager rows (caps rows shared with the Extrude). **Later:** Isoparm display, bevels, UVs |
| 9.6 ✅ | M | Sweep generator — done 2026-10-29 per [`reference/generator-sweep.md`](reference/generator-sweep.md) (new spec): `geom::surface::Sweep` (end scale and rotation, start / end growth, Parallel Movement, Constant Cross Section, Banking, Reverse Normals, caps) and `sweep(&profile, &path, &Sweep)`: each path segment cut to the growth range by length, stations with the path's direction (averaged at points), corner stretch 1 / cos(half turn) across the corner (at most 5×), frames unturned / level / parallel-transported from the mean plane (the leftover twist of a closed path spread evenly); profile nesting and caps shared with the Extrude; `ObjectKind::Sweep(Sweep)` in `GENERATORS`, `ObjectKind::generator_inputs` (2); `generator::Options::Sweep` (profile in its own space, path in the generator's); Alt + create wraps the first two selected objects (profile, path); Make Editable removes both inputs; `.g4d` v3 `sweep` kind (additive, defaults omitted, values clamped); UI: icon, toolbar, Attribute Manager rows. **Later:** rail spline, Scale / Rotation graphs, Keep Segments, UVs |
| 9.7 ✅ | M | Loft generator — done 2026-10-29 per [`reference/generator-loft.md`](reference/generator-loft.md) (new spec): `geom::surface::Loft` (Subdivision U / V 2–4000, Organic Form, Per Segment, Loop, Linear Interpolation, Reverse Normals, caps) and `loft(&[Polyline], &Loft)`: each section's first segment resampled by length from its first point (closed: U − 1 points), closed sections turned counter-clockwise around the skin's direction, rings placed per section pair by distance or evenly, points interpolated by Catmull-Rom (through every section), uniform B-spline with mirrored ends (Organic: through the first and last only) or linearly; a single closed section is a cap; `ObjectKind::Loft(Loft)` in `GENERATORS`, `generator_inputs` = every child (now exhaustive); `generator::Options::Loft` (every spline child in order, in the generator's space); Alt + create wraps every selected object in selection order; Make Editable removes all sections; `.g4d` v3 `loft` kind (additive, defaults omitted, counts clamped); UI: icon, toolbar, Attribute Manager rows. **Later:** Adapt UV / UVs, sections with holes |
| 9.8 ✅ | M | Text spline — done 2026-10-29 per [`reference/object-text.md`](reference/object-text.md) (new spec): `geom::spline::Text` (text as `Arc<str>`, height = em size, `TextAlign`, horizontal / vertical spacing, plane, reverse, intermediate points) and `TextAlign` — plain data; new crate `grid4d-text` (`ttf-parser` 0.25, already in the tree through egui): `text_spline(&Text)` lays lines out from the origin and turns every glyph contour into a closed Bezier segment (quadratic curves raised to cubic), in the built-in **Ubuntu Light** (egui's interface font, `epaint_default_fonts`, Ubuntu Font Licence — no new font file or licence; decision in the spec); `ObjectKind::Text(Text)` in `SPLINES`; eval caches its curve (`CurveSource::Text`) and takes spline bounds from the cached curve; every generator accepts it; Make Editable → editable spline; `.g4d` v3 `text` kind (additive, defaults omitted, at most 10 000 characters, height ≥ 0); UI: icon, Create ▸ Spline ▸ Text, toolbar list, Attribute Manager (multi-line field, Align, Height, spacings, plane rows shared with the primitives). Framing (H / O) now uses the evaluated bounds of objects whose extent the document cannot know (texts, generator results). **Later:** font choice, kerning, Separate Letters |

**Acceptance:** a Circle, Rectangle or Text spline extruded into a capped solid,
a profile lathed into a vase and swept along a helix, two circles lofted into a
tube; splines edited point by point and drawn with the Pen; everything undoable
and saved, and the generators' results rendered by the path tracer.

Phase 9 is complete (2026-10-29): every part of the acceptance is met and
tested (a review after 9.7 also fixed export, cage hiding and animated inputs
for the spline generators).

---

## Phase 10 — Deformers & MoGraph (v0.10)

**Goal:** shapes that move and multiply. Deformers bend, twist, taper and bulge
any object (primitives, polygon objects, generator results) without touching its
points; the Cloner repeats objects along lines, circles, grids and over other
objects, and effectors vary the clones. Object parameters (a Bend's strength, a
cube's size, a Cloner's count) become animatable, so all of it can move.

**Behaviour specs** (written per step before implementing, as before, from the
public manual — clean-room): `reference/animation-parameters.md` (10.1),
`deformer-basics.md` and `deformer-bend.md` (10.2), `deformer-twist-taper-bulge.md`
(10.3), `deformer-ffd.md` (10.4), `modeling-current-state.md` (10.5),
`mograph-cloner.md` (10.6–10.7), `mograph-effectors.md` (10.8).

### Design

* **Animated parameters (10.1).** `Param` grows a variant for an object's own
  numbers: `Param::Object(ObjectParam)`, where `ObjectParam` names one numeric
  field of an `ObjectKind` (a closed enum per kind, e.g. `CubeSize(Axis)`,
  `BendStrength`, `ClonerCount`), with `ObjectKind::param(p) -> Option<f32>` and
  `set_param(p, f32)` (integers rounded and clamped; enums and booleans are not
  animatable). `grid4d-eval` evaluates a node's kind at the frame
  (`NodeData::kind_at(frame)`, like `transform_at`) before drawing, sampling
  curves or building generators — the existing caches compare parameters, so an
  animated parameter rebuilds only what depends on it. The Attribute Manager
  gets key buttons beside animatable rows (the same states as the Coordinates
  fields: none, keyed, key here, changed), Autokeying records parameter edits,
  and the F-Curve editor lists the new tracks by their labels. Temporary values
  (`NodeData::unkeyed`) cover parameters as they cover transforms.
* **Deformers (10.2–10.4)** are objects (`ObjectKind::Bend { .. }`, `Twist`,
  `Taper`, `Bulge`, `Ffd`), each with a **box** in its own space (`size`, and a
  `DeformerMode`: *Limited* — the effect stops at the box but carries the rest
  along, *Within Box* — only inside the box, *Unlimited*). Their maths is pure
  and in `grid4d-geom::deform`: `fn bend(point, &Bend) -> Vec3` and friends, in
  the deformer's space, so they are unit-tested like the primitives.
  **Who is deformed:** a deformer deforms its **parent** and everything below
  the parent (the reference behaviour: group objects under a Null with the
  deformer to bend them together). Several deformers apply in Object Manager
  order, the deepest first. A generator's inputs are built with the deformers
  *inside* the input (a Bend under the cage bends the cage before smoothing);
  deformers beside the cage, under the generator, deform the generator's
  result. Splines are not deformed in Phase 10.
  **Evaluation:** `grid4d-eval` gathers, for each object with geometry, its
  deformer chain (ids, parameters, matrices into each deformer's space) — an
  unchanged chain means an unchanged result, cached per node like generator
  results (`Deformed { inputs, mesh }`), so idle frames and unrelated edits do
  nothing. A deformed primitive is drawn from its own mesh
  (`MeshKey::Polygon` of the result) instead of the shared unit mesh. Picking,
  snapping, framing, the path tracer and export all see the deformed mesh,
  since they already read evaluated meshes.
  **Display:** a deformer draws its box as lines in its display colour
  (overlay, not rendered), is picked near its lines, and has *Fit to Parent*
  (size and placement from the parent's bounds). Interactive handles (drag the
  strength) come with 10.3.
* **FFD (10.4)** is a lattice of `grid` points (default 3 × 3 × 3) spanning its
  box: the deformer's points are edited in Points mode like a spline's
  (selection, gizmo, one `CommitLive` per drag — the `PointEdit` seam grows a
  third case), and every deformed point moves by the trivariate Bernstein
  blend of the lattice's offsets.
* **Current State to Object (10.5)** bakes what the viewport shows — deformers,
  generators, clones, animation at the current frame — into a new polygon
  object (or a Null holding one per part), leaving the original untouched; one
  undo step. Make Editable keeps its meaning (the object's own geometry).
* **Cloner (10.6–10.7)** is a generator (`ObjectKind::Cloner { mode, .. }`)
  whose children are the objects to repeat. Modes: **Linear** (count, offset,
  per-step or end-point P / R / S), **Radial** (count, radius, plane, start /
  end angle, align), **Grid Array** (count × 3, per-step or end-point size;
  cube form), then **Object** (on another object's points or polygon centres,
  aligned to normals) in 10.7. **Clones** chooses which child each clone uses:
  Iterate, Random (seed). Clones are **instances**, not nodes:
  `EvaluatedScene::instances()` lists `EvaluatedInstance { owner, source,
  kind, world, bounds, drawable, pick }`, built after animation and Target
  tags so they follow the cloner. Consumers already batch by `MeshKey`, so a
  thousand cubes stay one instanced draw; picking a clone selects its Cloner;
  the path tracer and export bake instances like other surfaces. A child with
  its own children (a group) is cloned whole. Nested cloners clone clones.
  The instance list is rebuilt only when the evaluation is; it reuses its
  buffer (hot-path rule) and a test guards it.
* **Effectors (10.8):** **Plain** and **Random** objects, linked in a Cloner's
  *Effectors* list (`Vec<NodeId>`; creating an effector with a Cloner selected
  adds it there). Each has a **Strength**, Position / Rotation / Scale
  offsets (Random: within ± those values, from a seed per clone), a colour mode
  (off, or the effector's colour blended by strength) and a simple **falloff**
  of its own — Infinite, Sphere, Box or Linear, with size and soft edge, in the
  effector's space — drawn as lines like a deformer's box. Applied in list
  order to the clones' local matrices and colours. (Fields, the reference's
  modular falloffs, are later.)
* **File format:** `.g4d` v3 grows additively — deformer, cloner and effector
  kinds; parameter tracks as `Param` DTOs (`object` variant with the
  parameter's stable name). No version bump while v3 is unreleased.
* **Not in Phase 10:** deforming splines, the Wrap / Spline / Displacer /
  Smoothing deformers, Fields, MoGraph selections, the Shader / Step / Delay /
  Formula effectors, Fracture, the Matrix object, Tracer, Text MoGraph,
  render instances and multi-instances as separate modes (clones are always
  instances), clone colours per material.

### Steps

| Step | Size | Work |
|------|------|------|
| 10.1 ✅ | L | Animated parameters — done 2026-10-29 per [`reference/animation-parameters.md`](reference/animation-parameters.md) (new spec): `scene::param` — `ObjectParam` (39 real-valued parameters named by meaning, `label`, `index`, `ParamUnit` for degrees / percent), `ObjectKind::{params, param, set_param}` (a parameter exists exactly while its row is shown; values clamped); `Param::Object`, `ParamSet` as `u128`, `NodeData::{kind_at, set_kind_at, base_value}` and `value_at` / `set_value_at` / `key_state` / `record` / `remove_keys` for any parameter; `grid4d-eval` evaluates kinds at the frame (borrowed when nothing is animated) for drawables, curves, generators and their inputs, lights; Attribute Manager key buttons after each animatable field (`KeyRows`, found by label), Autokeying and temporary values through `set_kind_at`; the F-Curve editor lists parameter tracks after the transform ones (degrees, percent, own colours); Make Editable and export bake the current frame, Make Editable drops tracks the new object lacks; `.g4d` v3 parameter names (`"radius"`, `"size_y"`, …). Counts are not animatable yet (decision in the spec). **Later:** counts and choices, Record of parameters |
| 10.2 ✅ | L | Deformer framework and **Bend** — done 2026-10-29 per [`reference/deformer-basics.md`](reference/deformer-basics.md) and [`deformer-bend.md`](reference/deformer-bend.md) (new specs): `grid4d-geom::deform` — `DeformerMode` (Limited, Within Box, Unlimited), `BendOrigin` (Bottom, Center C / S, Top), `Bend::apply` (a column bent on a circle; Keep Y-Axis Length or the chord kept; strength in degrees, a stated difference from the reference's percentage), `box_lines` (the box's edges bent); `ObjectKind::Bend`, `ObjectKindTag::DEFORMERS`, `is_deformer`, animatable Size / Strength / Angle (`ObjectParam::Strength`); `grid4d-eval::deform` — a stack of active deformers while walking (pushed by their parent, deepest first, siblings in order), links into each deformer's space at the frame, the `Deformed` cache per node (mesh id or primitive parameters + links), generator inputs carrying the deformers inside them; deformed primitives drawn from their own mesh; the box as a curve (`CurveSource::Bend`), picked like a spline, `FilterItem::Deformer`; `EditorCommand::{CreateChild, FitToParent}` (Shift + create under the selection, fitted); Create ▸ Deformer, toolbar group, icon, Attribute Manager rows; `.g4d` v3 `bend` kind. **Later:** the strength handle (10.3), deformed splines, component modes on the deformed shape, export of deformed shapes (10.5) |
| 10.3 ✅ | M | **Twist, Taper, Bulge** and the strength handle — done 2026-10-29 per [`reference/deformer-twist-taper-bulge.md`](reference/deformer-twist-taper-bulge.md) (new spec): `grid4d-geom::deform` — `Twist` (angle by height), `Taper` and `Bulge` (strength, `Shape`: curvature and fillet; formulas stated in the spec), a shared `along` (height through the box by Mode); every deformer's `handle`, `handle_plane` and `with_handle` (the Bend by a search along its arc, the Twist winding on, Taper / Bulge by distance); `ObjectKind::{Twist, Taper, Bulge}` in `DEFORMERS`, `deformer_size_mut`, animatable `ObjectParam::{Amount, Curvature}`; eval's `Deformer` and `CurveSource::Deformer` cover all four; `viewport/deformer_tools.rs` — the orange handle drawn on the selected deformer and dragged before the gizmo (it may sit on an arrow), through `set_kind_at` (Autokeying), Shift steps 5° / 5 %, one `CommitLive`, Esc; `.g4d` v3 kinds; icons, toolbar, Attribute Manager rows (shared box / strength / shape / Fit rows). **Later:** size and angle handles |
| 10.4 ✅ | M | **FFD** — done 2026-10-29 per [`reference/deformer-ffd.md`](reference/deformer-ffd.md) (new spec): `grid4d-geom::deform::Ffd` (size, grid 2–20 per axis, mode; `grid_points`, `apply(lattice, point)` with Bernstein weights per axis, Limited clamping to the box, `lattice_lines`), `scale_lattice`; `ObjectKind::Ffd { ffd, lattice: SplineHandle, components }` — the lattice kept as a linear spline's points, so `ObjectKind::{points, points_mut}` lets Points mode, selections, the gizmo's `PointEdit` and Tweak work on it unchanged (no third case needed); eval's `Deformer` became `Clone` (it carries the lattice) and the lattice is registered with the editable points; Delete leaves lattice points; grid changes / Reset Lattice regrid, Size and Fit to Parent scale it; `.g4d` v3 `ffd` kind (lattice omitted while regular, counts checked, selections as ranges). **Later:** animating lattice points |
| 10.5 ✅ | S | **Current State to Object** — done 2026-10-29 per [`reference/modeling-current-state.md`](reference/modeling-current-state.md) (new spec): `EditorCommand::CurrentStateToObject` (Mesh ▸ Conversion; no shortcut — the reference has none by default); `ui::current_state::bake` evaluates the document at the current frame (an `EvaluatedScene`, viewport purpose) and copies each topmost selected object's visible subtree as plain objects — deformed meshes (`EvaluatedScene::deformed`, new), generator results (the cage's Phong and material taken over), primitives as polygons, splines and Texts as editable splines, a disabled generator as a Null; deformers and consumed inputs left out; transforms and parameters frozen, no tracks; each copy inserted after its original as one undo step, the copies selected. Export (glTF, OBJ) now writes deformed shapes. Clones join in 10.6 |
| 10.6 ✅ | L | **Cloner** framework — done 2026-10-29 per [`reference/mograph-cloner.md`](reference/mograph-cloner.md) (new spec): `grid4d-geom::cloner` — `Cloner` (layout, `CloneOrder` Iterate / Random with seed, Reset Coordinates), `Layout::{Linear, Radial, Grid}` (`StepMode` Per Step / End Point; per-step scale multiplies), `count` (≤ 10 000), `matrix(index)`, `source(index, children)` (SplitMix hash); `ObjectKind::Cloner` in `GENERATORS` (Alt + create, `Q`, filter *Generator*), every child an input, animatable Radius / Start / End Angle (Radial) and Size (Grid); `grid4d-eval` — the Cloner's children become `Source`s (drawable, pick, bounds) and are hidden, `EvaluatedScene::instances()` (`EvaluatedInstance { owner, source, kind, world, bounds, drawable, pick }`) built after animation and Target tags in reused buffers, the Cloner's bounds are its clones', `PickShape::Clones` (rectangle selection, no ray hit); a disabled Cloner is a group; `grid4d-render` draws instances in the objects' batches (1 000 cubes = one draw), selected with the Cloner, and picks them (→ the Cloner); the path tracer bakes them; `.g4d` v3 `cloner` kind (layout by mode, counts clamped); `ui::current_state::clone_objects` for Make Editable (a Null of polygon objects, children removed), Current State to Object and glTF / OBJ export; icon, toolbar, Attribute Manager rows. **Later:** groups and nested cloners (10.7) |
| 10.7 ✅ | M | Cloner **Object** mode, groups, nested cloners — done 2026-10-30 per [`reference/mograph-cloner.md`](reference/mograph-cloner.md) (extended): `grid4d-geom::cloner` — `Layout::Object(Surface)` (`Distribution` Vertex / Polygon Center, Align Clone: Y along the area-weighted normal), `Cloner::placements(surface, normals, out)` filling reused buffers; `ObjectKind::Cloner { cloner, object: Option<NodeId> }` (`ObjectKind::cloner`); `grid4d-eval` — every object inside an enabled Cloner is a `Source` (`top` for its children), so a child group is cloned whole; Cloners built deepest first (`Cloning` scratch), an inner Cloner's instances carried into the outer one's and dropped from the list; the object's mesh from the evaluation, or, hidden, its own polygons or a cached primitive mesh; `EvaluatedInstance::index` (clone number, names `Cube.3` when baked); `.g4d` v3: `object` layout, the link by index resolved after every object exists; Merge remaps it; Attribute Manager Object / Distribution / Align Clone rows (an object list like the Target tag's) |
| 10.8 ✅ | L | **Effectors** — done 2026-10-30 per [`reference/mograph-effectors.md`](reference/mograph-effectors.md) (new spec): `grid4d-geom::effector` — `Effector` (Strength, Position / Scale (Uniform) / Rotation each switched, colour, seed, `Falloff`: Infinite, Sphere, Box, Linear along Z; size, soft edge, smoothstep), `offset(amount, &Randoms)` in the clone's own space, `Randoms::of(seed, clone)` (SplitMix, −1 … 1 and a random colour), `lines` (a marker cross and the falloff's outline); `ObjectKind::{PlainEffector, RandomEffector}` (`ObjectKindTag::EFFECTORS`), animatable Strength and falloff Size; `ObjectKind::Cloner { effectors: Vec<NodeId>, .. }`; `grid4d-eval` applies the enabled effectors in list order to each clone's placement (then the child's own placement) and blends colours as `c × keep + tint` across the clone's parts and nested clones, in a reused buffer (hot-path test); effectors drawn and picked as curves (`CurveSource::Effector`), `FilterItem::Effector`; `.g4d` v3 `plain_effector` / `random_effector` and the Cloner's `effectors` by index (resolved after every object, remapped by Merge); `create_effector` adds a new effector to the selected Cloners in one undo step; Create ▸ Effector, toolbar, Attribute Manager rows and the Cloner's Effectors list (Add… / ✕). **Later:** Fields, other effectors, Remap / Min-Max, Random modes |

**Acceptance:** a cylinder bent into an arch with an animated Bend, a twisted
and tapered column, an FFD-shaped sphere; a Subdivision Surface over a deformed
cage; a ring of cubes from a Radial Cloner and a grid of spheres pushed by a
Plain effector's falloff and scattered by a Random one, with colours; all
animated by keys on parameters, undoable, saved, picked, exported and rendered
by the path tracer; the viewport stays interactive with 1 000 clones.

Phase 10 is complete (2026-10-30): every part of the acceptance is met and
tested — 1 000 clones are one instanced draw with no per-frame allocation.

---

## Phase 11 — Simulation (v0.11)

**Goal:** things fall, collide, bounce and come to rest. A *Rigid Body* tag
makes an object (or every clone of a Cloner) a solid body under gravity; a
*Collider Body* tag makes it something others hit. The simulation runs from
the start frame and is **cached per frame**, so scrubbing, playback, the Render
View, final renders and export all show the same, repeatable motion — and any
edit that changes the outcome simply makes the cache stale.

**Behaviour specs** (written per step before implementing, from the public
manual — clean-room): `reference/simulation-rigid-body.md` (11.1–11.2:
the tags, shapes, mass, bounce and friction, project settings),
`simulation-cache.md` (11.3, 11.5: start state, cache, bake, playback,
renders), `simulation-clones.md` (11.4).

### Design

* **Engine: `rapier3d` 0.36** (Dimforge, Apache-2.0, pure Rust, MSRV 1.86),
  behind a new crate **`grid4d-sim`** — the only crate that sees it. Rapier
  0.36 does its maths in **glam 0.33** (through `glamx`) while we are on
  glam 0.34, so two glam versions coexist; `grid4d-sim` converts at its edge
  through plain arrays (`[f32; 3]`, `[f32; 4]`) and nothing else ever touches
  a rapier type. When rapier follows glam 0.34 the conversions shrink to
  nothing (decision D10). Built without the `parallel` feature: one thread
  per simulation, so a run is deterministic on a machine (tests compare with
  a tolerance across platforms).
* **Units:** the scene is in centimetres, rapier is tuned for metres (contact
  tolerances, sleeping thresholds), so `grid4d-sim` simulates **in metres**
  and scales by 0.01 at its edge; gravity is set in the UI in cm/s² (default
  981 down). Masses in kilograms from a density in g/cm³ (default 1, water).
* **Scene data (11.1).** A body is a tag, like C4D's:
  `Tag::Dynamics(Body)` with `Body { motion: Motion, shape: CollisionShape,
  mass: Mass, bounce, friction, linear_damping, angular_damping,
  initial_velocity: (linear cm/s, angular °/s), children: Inherit }`.
  `Motion`: **Dynamic** (falls and collides), **Kinematic** (follows its own
  animation and pushes others), **Static** (never moves). The Tags menu offers
  it twice — *Rigid Body* (Dynamic) and *Collider Body* (Static) — as presets
  of the same tag (one per object). `CollisionShape`: **Automatic** (convex
  hull for moving bodies, the mesh itself for static ones), **Box**,
  **Sphere**, **Convex Hull**, **Static Mesh** (static / kinematic only).
  `Inherit`: *None* (only this object) or *Compound* (this object and its
  visible children as one body). Project-wide **`SimulationSettings`** in
  `Document::simulation` (enabled, gravity, steps per frame 1–100 default 5,
  solver iterations, time scale), changed through `SceneEdit::SetSimulation`
  (validated, like `SetRender`), shown in the Attribute Manager's Project
  settings. `.g4d` v3 grows additively (a `dynamics` tag, a `simulation`
  block omitted while default).
* **The simulation core (11.2)** is plain data, `Send`, no UI:
  `Simulation::new(&EvaluatedScene /* at the start frame */, &Scene, &settings)`
  gathers every body — its shape from the **evaluated** mesh (so a primitive,
  a polygon object, a generator's result, a deformed shape all work), scaled
  by its world matrix's scale (rapier bodies have none) — and builds the
  rapier world; `step_to(frame)` advances it frame by frame (`steps` sub-steps
  of `1 / fps × time scale`), kinematic bodies moved to their animated pose
  each sub-step (`set_next_kinematic_position`), and records each body's
  world pose per frame into a **`SimCache`** (frames × bodies of position +
  rotation; 28 bytes per body per frame — 1 000 bodies × 300 frames ≈ 8 MB).
  Bodies are identified by `BodyKey::{Node(NodeId), Clone(NodeId, u32)}`.
* **Evaluation stays simulation-agnostic (11.3).** `grid4d-eval` gets a
  general hook: **`EvaluatedScene::set_overrides(&Overrides)`** — world
  matrices for some nodes (and some clones, by Cloner and clone number) at the
  current frame, applied after animation (children follow their simulated
  parent, as with animation). `grid4d-sim` turns a cached frame into
  `Overrides`; the dependency arrow stays eval ◀ sim ◀ ui.
* **The cache in the editor (11.3).** `EditorState::simulation` (a
  `ui::simulation::Sim`, like `Renders`) owns the `Simulation` and its cache.
  A cheap **inputs key** — for every body node: its tag, its pose and shape
  key (mesh id or primitive parameters) at the start frame, its animation
  when kinematic; plus the settings and the frame range — is rebuilt when the
  scene revision changes and compared: selecting, renaming or moving an
  unrelated object keeps the cache; anything else drops it. Before drawing
  frame *f*, the cache is filled up to *f* within a time budget per UI frame
  (a few ms; the rest next frame, with a repaint request), so the first
  playthrough simulates "live" and every later visit is read from the cache;
  scrubbing back is instant. The timeline draws the cached range as a bar.
  **Simulate ▸ Bake** fills the whole range now (with progress, cancellable);
  **Clear Cache** empties it. Frames before the start frame show the start
  state.
* **Everything else reads the cache.** The viewport (through overrides),
  Current State to Object and export at the current frame (their evaluations
  get the same overrides), the Render View and final renders (the job gets a
  snapshot of the cache plus its own `Simulation` to extend it — the run is
  deterministic, so it simulates the same motion), picking and snapping.
* **Clones as bodies (11.4).** A Rigid Body tag on a Cloner makes **every
  clone** a body (each clone's shape from its child, a cloned group as a
  compound), keyed `BodyKey::Clone(cloner, n)`; the clones' placements at the
  start frame (layout and effectors) are their start state; afterwards the
  cache moves them. Collider Body on a Cloner: static clones (a field of
  posts).
* **Bodies in the file and performance (11.5).** A baked cache can be saved
  with the document (`.g4d` v3 `simulation.bin`: frames × bodies, little-endian,
  checked sizes, dropped on load if the inputs key no longer matches) so a
  reopened file plays at once; a memory budget (Project settings, default
  256 MB) stops caching beyond it with a warning; a benchmark keeps 1 000
  falling cubes at interactive speed.
* **Not in Phase 11:** soft bodies, cloth, ropes, particles, fluids, forces
  (wind, attractor), connectors / joints and motors, collision triggers
  (*On Collision*), Follow Position / Rotation springs, fracturing. The
  Simulate menu keeps them listed as planned.

### Steps

| Step | Size | Work |
|------|------|------|
| 11.1 ✅ | M | **Scene data and the physics crate** — done 2026-10-30 per [`reference/simulation-rigid-body.md`](reference/simulation-rigid-body.md) (new spec): `scene::dynamics` — `Body` (enabled, `Motion` Dynamic / Collider — an animated collider is kinematic, so two motions cover C4D's three, `Inherit` None / Compound, `CollisionShape` Automatic / Box / Sphere / Convex Hull / Static Mesh, bounce, friction, `MassMode` Global / Custom Density / Custom Mass, rotational mass, damping, initial velocity and spin; `rigid`, `collider`, `validated`) and `SimulationSettings` (enabled, gravity cm/s², steps per frame, solver iterations, time scale; `validated`); `Tag::Dynamics`, `TagKind::Dynamics`; `Document::simulation`, `SceneEdit::SetSimulation` (in the random undo/redo test); `EditorCommand::AddTag` now carries the tag, so the Tags menus offer Rigid Body and Collider Body as presets of one kind; Attribute Manager tag rows and a Simulation section in the Project settings (one `CommitLive` step); `.g4d` v3 `dynamics` tag and `simulation` block (defaults omitted, non-finite numbers refused); new crate `grid4d-sim` — `rapier3d` 0.36 (its own `PhysicsWorld`), `convert.rs` (arrays across the glam versions, cm ⇄ m), `World` (bodies with ball / box shapes from a `Body`, `step_frame` in sub-steps, poses); tests: ½ g t² free fall, a ball resting on a collider, identical runs |
| 11.2 ✅ | L | **Simulation core** — done 2026-10-30 per [`reference/simulation-rigid-body.md`](reference/simulation-rigid-body.md): `grid4d-sim::Simulation::new(doc, evaluated start frame)` — every enabled Dynamics tag on a visible object becomes a body at its evaluated pose (rotation and translation; the scale baked into the shapes and kept for the cache), its parts the object and, Compound, its visible children (their tags then belong to the body); `shape.rs` — colliders from the shown mesh (Polygon key: the evaluated mesh — generated, deformed; else the primitive's): Box / Sphere from the bounds (½ cm thinnest), Convex Hull (box if flat), Static Mesh (triangulated; dynamic bodies use a hull), Automatic; mass properties computed per collider (density, Custom Mass spread by volume, Rotational Mass dividing the inverse inertia), bounce combined by Max, friction by Multiply, damping as a rate; `step_to` / `step`: sub-steps, animated colliders moved to `world_matrix_at` at each sub-step's end (kinematic); `SimCache` (`BodyKey::Node`, start scales, frames × poses, `world(frame, body)` with the start state before the start, `last`, `memory`); tests: a cube rests on a floor at its half-size, a ball's bounces fall lower, a box slides far further on an icy 20° slope, an animated paddle pushes a box, identical runs give identical caches, Compound / disabled / settings off |
| 11.3 ✅ | L | **Playback, cache and renders** — done 2026-10-30 per [`reference/simulation-cache.md`](reference/simulation-cache.md) (new spec): `grid4d-eval::Overrides` and `EvaluatedScene::set_overrides` (world matrices by node, applied after animation and Target tags, subtrees following by their delta, a re-evaluation only when they change; hot-path test); `grid4d-sim` — `SimCache::overrides(frame)`, `Simulation::of(doc)`, `Replay` (a cache copy simulating what it lacks); `ui::simulation::Sim` in `EditorState` — an inputs key (settings, fps, start; per body its start world matrix, its subtree's data without names, what above it animates or hides it) checked on revision changes, the cache dropped only when it differs, `advance` (8 ms per refresh, 40 ms baking, a repaint while unfinished), `overrides`, `bake` / `stop_baking` / `clear`, `replay`; the viewport applies the overrides each refresh; the timeline's green cache bar; Simulate ▸ Bake Simulation / Clear Simulation Cache (`EditorCommand::{BakeSimulation, ClearSimulation}`), status bar progress, Esc stops a bake; renders (`Renders::start` / `start_view`) take a `Replay`; Current State to Object and `file::exportable_at` place simulated objects. Clone overrides come with 11.4 |
| 11.4 ✅ | M | **Clones as bodies** — done 2026-10-30 per [`reference/simulation-clones.md`](reference/simulation-clones.md) (new spec): `Overrides::set_clone(cloner, n, delta)` — a clone's move since the start applied to all its parts after the instances are built, the Cloner's bounds made again from its moved clones; `grid4d-sim` — a Dynamics tag on a Cloner makes each clone (`BodyKey::Clone(cloner, n)`) one body framed by its first part, its parts' shown meshes (a cloned group: one body), the tag's settings for all; `SimCache::overrides` gives each clone `world(f) × world(start)⁻¹`; the editor's inputs key also holds a tagged Cloner's effectors and Object mode object (place and data); Make Editable bakes simulated clones (Current State to Object, export and renders already read the overrides) |
| 11.5 ✅ | M | **Saved caches and limits** — done 2026-10-30 per [`reference/simulation-cache.md`](reference/simulation-cache.md) (extended): `scene::{SimulationCache, CachedBody}` (plain data); `.g4d` v3 `simulation_cache` (start, frame count, bodies by object index with clone number and scale) and `simulation.bin` (frames × bodies × seven little-endian `f32`, read against the claimed size, capped at 2 GiB, finite numbers only); `grid4d_io::{to_bytes_with_cache, from_bytes_with_cache, save_with_cache, load_with_cache}` (the plain functions unchanged); `SimCache::{to_saved, from_saved, agrees}`; the editor saves its fuller cache and, on opening, adopts saved frames that agree with a fresh simulation's start (same bodies, scales, start poses), simulating on past them; `SimulationSettings::memory_limit_mb` (Cache Memory, default 256 MB, left out of the inputs key) stops the cache with a status-bar warning; `criterion` 0.8 (dev, no default features) benchmark — 1 000 bodies built in ≈ 2.3 ms, a frame of 1 000 falling cubes in ≈ 4.6 ms; the manual's Simulation chapter (§16) |

**Acceptance:** a stack of cubes knocked over by a dynamic sphere; 1 000 spheres
from a Cloner poured into a bowl (a static mesh collider); a kinematic paddle
animated by keys pushing boxes; scrubbing back and forth shows identical frames;
changing a bounce or moving a body re-simulates, selecting or renaming does
not; the Render View, final renders, Current State to Object and glTF / OBJ
export show the simulated state; a baked cache is saved and reopens playing;
everything undoable.

Phase 11 is complete (2026-10-30): the acceptance is met — the paddle, the
falling, bouncing and sliding in the simulation tests; clones falling as
bodies in tests and on screen; the cache, renders, bakes and saved frames in
the editor's tests. One difference: the 1 000-body case is a box of cubes in
the benchmark (≈ 4.6 ms a frame) rather than spheres poured into a bowl,
which works the same way (a static mesh collider) but has no test of its own.

## Phase 12 — Workspace (v0.12)

**Goal:** the workspace becomes the user's own. Panels dock, split, tab and
float where you drag them, with a few ready layouts; **Layers** sort a scene
into switchable groups; **Takes** keep variations of one scene (another camera,
another colour, an object hidden) and render the one you pick; **Preferences**
hold the user's units, undo depth, autosave, theme and language; and the whole
interface reads in **Greek** as well as English.

**Behaviour specs** (written per step before implementing, from the public
manual — clean-room): `reference/workspace-layouts.md` (12.1),
`workspace-preferences.md` (12.2–12.3), `manager-layer.md` (12.4),
`manager-take.md` (12.5), `workspace-language.md` (12.6).

### Design

* **Docking (12.1): `egui_tiles` 0.18**, not `egui_dock`. The plan asked to
  check `egui_dock` first: its newest release (0.21.1, 2026-08) is still on
  egui 0.36, while `egui_tiles` 0.18 (2026-10-19; MIT OR Apache-2.0, MSRV 1.95,
  from egui's own authors) is built for egui 0.37 and serialises its tree
  (decision D13). A `ui::workspace::Workspace` owns an `egui_tiles::Tree<Pane>`
  whose panes are the existing panels — Viewport, Objects, Takes, Attributes,
  Coordinates, Layers, Timeline, F-Curve, Materials — in tab containers and
  splits; the menu bar, command palette, mode palette and status bar stay
  fixed around it. Its `Behavior` dispatches each pane to its panel's `show`,
  so panels stay independent; the hand-made tab strips
  (`widgets::manager_tabs`, the bottom tabs) and `EditorState::{managers,
  bottom}` give way to the tree, and commands that "show" a panel
  (Shift+F2, Shift+F5, Tab over the bottom panel) focus its tab, opening it if
  it was closed. Tabs drag to another container or to an edge to split; a
  pane's tab can be closed except the Viewport's. **Layouts:** *Standard*
  (today's arrangement), *Modeling* (a larger viewport, the Coordinate
  Manager beside the Attribute Manager, no timeline), *Animate* (the F-Curve
  editor tall under the viewport, the timeline beside it); the menu bar's
  "Layout:" label becomes a menu of them, Window ▸ Reset Layout restores the
  current preset, and the tree is saved with the window (eframe storage — the
  binary saves it, as it saves recent files).
* **Preferences (12.2)** are user settings, not document data: a
  `Preferences` struct in `EditorState`, edited in **Edit ▸ Preferences…**
  (a window, Ctrl+E as in the reference if verified), saved by the binary in
  eframe storage. Contents: **units** for showing lengths (cm, m, mm, inch,
  foot — values stay centimetres inside; every length field goes through one
  `widgets::length` that converts and suffixes), **undo depth**
  (`History::with_limit`), **autosave** (off / every *n* minutes: a recovery
  copy in the user data folder while the document has unsaved changes,
  offered on the next start after a crash), **viewport defaults** (shading,
  grid and HUD for new views), and the theme and language of 12.3 and 12.6.
* **Themes (12.3):** *Dark* (today), *Darker* and *Light*. The palette's
  colours are constants today; they become a `Palette` struct with one value
  per theme, read through `theme::palette()` (the current theme kept in an
  atomic, so reading stays free), and egui's own visuals follow. A test checks
  every palette keeps its text readable against its backgrounds (contrast
  ratio ≥ 4.5).
* **Layers (12.4)** are document data like materials: an ordered `Layers`
  table with stable `LayerId`s in `Scene` (`SceneEdit::SetLayers`), each with a
  name, colour and switches — **Solo**, **View** (editor), **Render**,
  **Manager** (listed in the Object Manager), **Locked** (not selectable or
  editable), **Animation** (off: shown unanimated), **Generators**,
  **Deformers** (off: they act as off); `NodeData::layer: Option<LayerId>`
  (children inherit it unless they have their own). Evaluation reads the
  switches (`Purpose::shows` grows a layer check; generators, deformers and
  animation honour theirs); picking and selection skip locked objects; the
  Object Manager shows a layer colour chip and hides Manager-off objects. The
  **Layer Manager** is the Layers pane: create, rename, recolour, the switch
  columns, drag objects onto a layer, select a layer's objects. `.g4d` v3
  additive; Merge remaps layers like materials.
* **Takes (12.5)** are variations of the document kept in `Document::takes`:
  the **Main** take (the document as it is) and named takes, each holding
  overrides per object by group — transform, visibility switches, the
  object's parameters (`kind`), its tags (material links), display colour —
  plus an optional camera and render settings. The document always holds the
  **active** take's state, so every panel, evaluation and the renderer work
  unchanged: switching takes is one undoable `SceneEdit::SwitchTake` that
  swaps the overridden groups in and the previous take's out (Main's values
  for every overridden object are kept beside the takes). While a take other
  than Main is active, every edit is recorded as an override of that take
  (**Auto Take**, always on — the reference's default workflow), and the
  Take Manager (the Takes pane) lists takes with their overrides, makes one
  active, adds, renames, deletes, and clears an override. Render Settings
  gain **Take** (Current or a named one): a render job applies that take to
  its copy of the document (`Takes::apply`, pure). `.g4d` v3 additive.
* **Language (12.6):** every UI string goes through `i18n::tr("English text")`
  (the English text is its own key, so code stays readable and an untranslated
  string still shows in English); a `Language` preference picks the table —
  English (the keys) or **Greek** (`i18n/el.rs`, a sorted table of pairs).
  Formatted strings use numbered placeholders (`tr_args("{0} objects", …)`).
  A test scans the sources for `tr("…")` literals and fails if the Greek table
  lacks one, so new strings cannot be forgotten. The manual stays in English
  (a Greek manual is later). Shortcut letters do not change with the language.
* **Not in Phase 12:** floating panels in separate OS windows, layout files
  shared between machines, take hierarchies and Render Marked Takes, override
  groups, a Greek manual, right-to-left languages, plugin-defined panes.

### Steps

| Step | Size | Work |
|------|------|------|
| 12.1 ✅ | L | **Docking layouts** — done 2026-10-30 per [`reference/workspace-layouts.md`](reference/workspace-layouts.md): `egui_tiles` 0.18; `ui::workspace::{Pane, PaneState, Panels, Workspace}` — an `egui_tiles::Tree<Pane>` of nine panes and a `Behavior` (`Dispatch`) that draws each panel in its frame, marks it shown and hovered, and styles the tab bars with the palette (closable tabs, none on the Viewport, no tab bar while it is alone); the menu bar, command palette, mode palette and status bar stay fixed around it; the manager and bottom tab strips are gone, `EditorState::{managers, bottom, bottom_hovered}` replaced by `EditorState::panes` (shown this frame, hovered, show / layout / reset requests); Standard / Modeling / Animate presets, the Layout menu, a Window entry per pane (reopens a closed one beside its usual neighbours, else fronts it) and Window ▸ Reset Layout; the tree saved in eframe storage (`grid4d.workspace`) and repaired on load (a pane twice or an empty tree gives the preset back, a missing Viewport is added, closed panes stay closed); tests: every preset holds every pane once, a closed pane reopens, a broken tree is repaired, requests and shown marks, clicking a tab and its × (pointer events found through egui's accessibility tree), a serde round trip keeping a closed pane closed, idle frames in every layout leave the document alone |
| 12.2 ✅ | M | **Preferences** — done 2026-10-30 per [`reference/workspace-preferences.md`](reference/workspace-preferences.md): `ui::preferences::{Preferences, Units, ViewDefaults}` in `EditorState::preferences` (serde, checked into range on load; shading stored by name), saved by the binary in eframe storage; the Edit ▸ Preferences… window (`panels/preferences.rs`; no shortcut — Ctrl+E is only in third-party lists); `widgets::{length, length_per}` (a `DragValue` over centimetres with a formatter and parser in the unit read from the egui context, typed symbols converted) for every length field, speeds and gravity, and the grid HUD; `History::{limit, set_limit}` applied by `commands::apply_pending`; `ui::autosave::Autosave` — a recovery copy per running session written by a background thread (to `.part`, then renamed) while the document is dirty, removed when clean or on exit (`App::on_exit`), each session holding a file lock (`File::try_lock`) so only crashed sessions' copies are offered; the Recover dialog (`panels/recovery.rs`, `EditorCommand::Recover`): the newest copy opens unsaved and linked to its file, the rest are removed; `ViewportPanel::apply_defaults` at start and `EditorCommand::ApplyViewDefaults`; tests: unit formatting and parsing round trips, a length field showing metres and taking "25 mm", stored preferences, the undo limit lowered at once, autosave only while dirty and its lock, the newest crashed copy offered and recovered, view defaults |
| 12.3 ✅ | M | **Themes** — done 2026-10-30 per [`reference/workspace-preferences.md`](reference/workspace-preferences.md) (extended): the `theme::palette` constants became `theme::Palette` with `DARK` (the old values), `DARKER` and `LIGHT`, read through `theme::palette()` (the theme in an `AtomicU8`; every `p::NAME` became `p().name`); `theme::view_palette()` for what is drawn over the 3D view (HUD, triad, gizmo, handles, component colours), which stays dark in Light; `theme::{apply, follow}` install egui's style per theme (`egui::Theme::Light` for Light); `Preferences::theme` with Preferences ▸ Interface ▸ Theme; tests: every theme's contrast (text ≥ 4.5 : 1 on all backgrounds and selected rows, dimmed text, accent, highlight and icon tints ≥ 3 : 1), switching installs the style |
| 12.4 ✅ | L | **Layers** — done 2026-10-30 per [`reference/manager-layer.md`](reference/manager-layer.md): `scene::{Layer, LayerId, LayerSwitches, Layers}` (ordered table, colours in turn, `shows` with Solo, `shows_own`, `all_animate`) in `Scene` with `SceneEdit::SetLayers`; `NodeData::layer`; `Scene::{effective_layer, layer_switches, animates, is_locked, protection}`, `world_matrix_at` honouring Animation; evaluation: a layer stack in the walk (View / Render / Solo hide a subtree, Animation reads the document's values, Generators off makes a generator a group — Cloners too, Locked leaves no pick shape), generator inputs check their own layers, deformers on a Deformers-off layer do nothing; the gizmo and the Coordinate Manager treat locked objects as fully protected; `.g4d` v3 additive (`layers`, each object's `layer` index; non-finite colours refused both ways); `Objects::layers`, Merge adds copies with unique names; the Layer Manager pane (`panels/layers.rs`: chip with live colour picker, rename, eight switch columns, New Layer, a row menu, objects dropped from the Object Manager through `EditorState::object_drag`); the Object Manager's layer chips and Manager switch; the Basic tab's Layer field; `EditorCommand::{AssignLayer, SelectLayerObjects}`; tests: the table and visibility rules, evaluation of every switch, generators and deformers off, save and load, Merge, assigning with undo, the Object Manager hiding rows, a drop onto a layer, idle frames with a layer |
| 12.5 ✅ | L | **Takes** — done 2026-10-30 per [`reference/manager-take.md`](reference/manager-take.md): `scene::{Take, TakeId, Takes, Override, OverrideGroup, ObjectOverrides}` in `Document::takes` (overrides by object in five groups — Transform, Visibility, Parameters (not an editable object's points), Tags, Display Color — plus a camera and own render settings; Main's values for what the active take overrides kept beside it; the take to render); `Document::switch_take`, a pure swap (Main's values back, the take's written, Main's kept; it reaches objects deleted for now, so undo is exact); `Document::record_in_take` (Auto Take from an edit's inverses: `SetData`, `Move`, `SetRender`; a kind change drops parameter overrides); `SceneEdit::{SwitchTake, SetTakes}` in the random undo/redo test; `History::record` records in the active take (`record_as_is` for the takes' own edits); `EditorCommand::{SwitchTake, NewTake, DeleteTake, ClearOverride}` (a take's camera becomes the view's on switching); renders of another take switch a copy and look through its camera; the Take Manager pane (`panels/takes.rs`); Render Settings ▸ Take; `.g4d` v3 additive (`takes`: each take with its overrides, the active one, Main's values, the render take); tests: switching and recording, the table, saving with an active take, the commands with undo, rendering another take, idle frames with a take |
| 12.6 ✅ | L | **Greek** — done 2026-10-30 per [`reference/workspace-language.md`](reference/workspace-language.md): `ui::i18n` — `tr` (the English text is the key; English is the default and the fallback), `trf` (numbered placeholders `{0}`, `{1}` that a translation may move; it replaces the planned `tr_args`), `key` (a `const fn` marking texts in constants and match arms, translated where shown), `Language` in an atomic (`set_language`, as for the theme: no restart); the Greek table `i18n/el.rs` (1 012 sorted pairs, binary search); every interface text routed through it — widget literals, the label-drawing helpers (`attr_label`, `section_heading`, the Attribute Manager's rows — so key buttons keep English keys), menus, tooltips, hints, dialogs, alerts, undo step names as the menus show them, enum labels from every crate; document data (names) never translated; `Preferences::language` with Preferences ▸ Interface ▸ Language; attribute labels clipped to their column (full text on hover); the About window names the languages; tests: a source scanner finds every shown text (`tr`/`trf`/`key` literals, helper arguments, undo and alert names, the bodies of `fn label`/`hint`/`title`) missing from the table or left over, the table is sorted and keeps its placeholders, English is the default |

**Acceptance:** dragging the Attribute Manager under the Object Manager and
the timeline beside the viewport survives a restart, and *Animate* / *Reset
Layout* arrange the panels at once; lengths show in metres when the units say
so; the Light theme is readable everywhere; a scene's lights on a layer switched
off for the renderer do not render, a locked layer cannot be picked; a take with
another camera, a hidden object and a red material renders differently from
Main and switching back restores Main exactly; the whole interface reads in
Greek; everything undoable where it is document data, and saved.

Phase 12 is complete (2026-10-30): every part of the acceptance is met —
the arrangement survives a restart (on screen) and is repaired when broken
(tests); metres (tests and on screen); the Light theme's contrast (a test);
layers hiding renders and locking (tests and on screen); a take hiding
objects renders differently and switching back restores Main (tests); the
interface in Greek (a completeness test and on screen); all undoable and
saved (tests).

## Phase 13 — Extensibility (v0.13)

**Goal:** Grid4D grows without changing its core. **Plugins** — Rust crates
built into the application — add new generators, deformers and commands
that look and behave like built-in ones (parameters in the Attribute Manager,
animation, undo, takes, saving); and **scripts** written in the application
— small programs that read and change the document — automate repetitive
work, each run one undo step.

**Behaviour specs** (written per step before implementing, from the public
manual and SDK docs — clean-room): `reference/extend-plugins.md`
(13.1–13.3), `extend-scripting.md` (13.4–13.5).

### Design

* **Plugins are crates built in, registered explicitly** (decision D16). A
  new crate **`grid4d-plugin`** (depends on `grid4d-geom` and
  `grid4d-scene`) holds the API: a **parameter schema** (`ParamSpec`: name,
  kind — float with unit and range, integer, bool, choice, vector, colour —
  and default), the traits **`GeneratorPlugin`** (parameters → a
  `grid4d_geom::PolyMesh`), **`DeformerPlugin`** (parameters, a point, the
  deformer's space → a moved point) and **`CommandPlugin`** (the document and
  selection → `SceneEdit`s), and the **`Registry`** of them by a stable,
  namespaced type id (`"grid4d.gear"`). The binary builds the registry at
  start from a plain list (`src/plugins.rs`) and installs it once
  (`grid4d_plugin::install`, a `OnceLock`); tests install their own. Plugin
  crates live in `plugins/` in the workspace (`grid4d-plugin-gear`,
  `-wave`, `-scatter`) and depend only on `grid4d-plugin`. No dynamic
  libraries: `unsafe` stays forbidden and Rust has no stable ABI.
* **Plugin objects are one more `ObjectKind`:** `ObjectKind::Plugin(PluginObject
  { type_id, values })`, `values` a list of named `PluginValue`s matching the
  schema. Everything that works on objects works on them: `SetData` edits,
  undo, takes (Parameters group), layers, the Attribute Manager (built from
  the schema), animation (`ObjectParam::Plugin(index)` for float values,
  KeyRows by parameter name), the file format (`KindDto::Plugin` with the
  type id and values by name). **A document whose plugin is missing still
  opens:** the object keeps its values, shows as a group with a "missing
  plugin" note, and saves back unchanged; values the schema does not know
  are kept, missing ones take their defaults.
* **Evaluation:** `grid4d-eval` (which now depends on `grid4d-plugin`) asks
  the registry for generator plugins' meshes — cached per object and values
  like the built-in generators, the result validated (finite points, valid
  indices; otherwise nothing and a warning) — and treats deformer plugins
  like built-in deformers (in the deformer stack, their box drawn as a
  curve). The path tracer and export see the results as they see built-in
  generators'.
* **Commands** appear in **Plugins ▸** (and the Command palette's search,
  later): with options (a schema) they open a small dialog built from it;
  their `SceneEdit`s are one undo step.
* **Scripting: Rhai** (decision D17) — `rhai` 1.27 (2026-10-21; MIT OR
  Apache-2.0; pure Rust, MSRV 1.66), not Python through `pyo3`: no Python
  installation to find or bundle, nothing outside the process, a sandbox
  that can cap operations and has no file or network access unless given
  one, the same build on every platform. A new crate **`grid4d-script`**
  (depends on `grid4d-scene` and `grid4d-plugin`) wraps an engine with a
  document API: objects (find by name, children, selection), their name,
  position / rotation (degrees) / scale, visibility, colour, parameters by
  name (built-in and plugin), create / delete / reparent, materials and
  layers (assign), the current frame, `print`, and the plugin commands.
* **Scripts change the document only through `SceneEdit`s** (decision D18):
  a run works on a copy of the document, records the edits it makes and,
  if it finishes, applies them as **one undo step** (an error or a stop
  applies nothing). A script is stopped after a set number of operations
  (an infinite loop cannot hang the application); errors report the line
  and column.
* **The Script Manager** (a window: Script ▸ Script Manager…, as the
  reference's) edits a script in a monospace editor with line numbers, runs
  it (Ctrl+Enter), shows its output and errors in a console, and keeps
  scripts as `.rhai` files in the user's script folder (in the user data
  folder, beside the recovery copies); **Script ▸** lists them to run with a
  click. Example scripts ship with Grid4D.
* **Not in Phase 13:** plugins loaded from libraries at run time, Python,
  scripts that run every frame (expression tags, node-based expressions),
  scripts with file or network access, plugin panels or tools that take
  pointer input in the viewport, a plugin marketplace.

### Steps

| Step | Size | Work |
|------|------|------|
| 13.1 ✅ | L | **Plugin API and generator plugins** — done 2026-10-30 per [`reference/extend-plugins.md`](reference/extend-plugins.md): the `grid4d-plugin` crate (`ParamSpec`, `ParamKind`, `Unit`, `Values` — typed reads by name with defaults and ranges — `GeneratorPlugin`, `Registry` with `new_object` and a checked `build`, `install` / `registry` in a `OnceLock`); `scene::{PluginObject, PluginValue}` and `ObjectKind::Plugin` (`ObjectKindTag::Plugin`, outside `ALL`); evaluation builds a plugin's mesh in the walk, cached per object and values, drawn and picked like a polygon object (deformers, materials, layers apply), nothing for a missing plugin (a group); `editable_mesh` covers plugins, so Make Editable and export do; `.g4d` v3 additive `KindDto::Plugin` (type id, values in order by name; unknown ones kept; non-finite numbers refused); the Object tab from a description (lengths in the user's unit, angles, percent, integers, check boxes, choices, vectors, colours; hints as tooltips) and the missing-plugin rows; **Create ▸ Plugins**; `EditorCommand::CreatePlugin`; the binary's list (`src/plugins.rs`) installed in `main`; the **Gear** (`plugins/gear`, `grid4d-plugin-gear`: teeth, radius, depth, width, thickness, hole); the translation scanner reads plugins' texts; tests: values and ranges, the registry, the gear's faces facing out with and without a hole, evaluation and its cache, a missing plugin, the file round trip, creating, undoing and making editable, idle frames with a plugin's parameters shown |
| 13.2 ✅ | M | **Deformer plugins and animated plugin parameters** — done 2026-10-30 per [`reference/extend-plugins.md`](reference/extend-plugins.md): a common `Plugin` trait; `DeformerPlugin` (`size_param`, optional `handle_param`, `deform` on a slice of points in its space) with the box, handle and checked deformation in `grid4d-plugin`; `Registry::{add_deformer, deformer, plugin, plugins}`; `grid4d_eval::is_deformer` and `Deformer::Plugin` in the chain (deform_mesh now runs link by link over all points); the example **Wave** (`plugins/wave`: size, amplitude — the handle — wavelength, phase, direction, within box); `ObjectParam::Plugin(index)` (a value's place, the first 64; `ParamSet` ignores bits past 128), `ObjectKind::param_label` / `NodeData::param_label`; key buttons on plugin numbers; the F-Curve editor's names and units from the plugin; tracks saved by name (`ParamDto::Plugin`); Create ▸ Plugins lists deformers after a line, Shift puts one under the selection fitted (`CreatePlugin { child }`, Fit to Parent); tests: the wave's shape, a deformer plugin's box, handle and failure, a wave rippling a plane and following its keys, the file round trip of a plugin track, creating under the selection, undo and a take, the handle, a key button for every plugin number |
| 13.3 ✅ | M | **Command plugins** — done 2026-10-30 per [`reference/extend-plugins.md`](reference/extend-plugins.md): `CommandPlugin::run(Context, Values) → Result<Changes, message>`; `Context` (the document, the selection in order, the active object, the frame, `mesh` — shapes the interface evaluates — and `world_matrix`); `Changes` (inserts under the root, an object or a `NewObject` added before, and `SceneEdit`s); `Registry::{add_command, command, commands}`; the **Plugins** menu ("…" when options come first; "No commands installed"); the options dialog (`panels/plugin_command.rs`, fields from `widgets::plugin_value`, shared with the Object tab; options kept per session in `EditorState::plugin_commands`); `EditorCommand::RunPlugin` applies the changes in order as one undo step, all or nothing, the new top-level objects selected, or shows the message; the example **Scatter** (`plugins/scatter`: copies with their subtrees over the active object's surface, area-weighted, seeded SplitMix64, aligned, turned and scaled at random, under a new Null); the translation scanner reads commands' `Err(…)` messages; tests: changes referring to new objects, the scatter's placement, determinism and messages, running as one step and undoing it, the message with nothing selected |
| 13.4 ✅ | L | **Scripting core** — done 2026-10-30 per [`reference/extend-scripting.md`](reference/extend-scripting.md): the `grid4d-script` crate (`rhai` 1.27; 100 million operations, call and size limits, no `eval`): `run(document, selection, frame, shapes, source) → Outcome | ScriptError` works on a copy, applying `SceneEdit`s and keeping their inverses; the API — `objects`, `roots`, `find`, `selection` / `select`, `frame`, `create` (Create-menu kinds and plugins), `delete`, `run_command` (plugin commands with options as a map), `print`; an `Object`'s `name`, `kind`, `position`, `rotation` (degrees), `scale`, `color`, switches, `parent` (keeping its world place), `children`, `material` / `layer` by name, `get` / `set` of parameters by their Object-tab names in display units; errors with line and column; `Changes::apply` moved into `grid4d-plugin` for scripts and the interface; `EditorCommand::RunScript` (the copy becomes the document, one "Run Script" step, the selection, output or error in a message), **Script ▸ Run Script File…** (`FileDialogs::pick_script`); tests: a grid of coloured cubes undone in one go, a failing script (line, column, nothing changed), a syntax error, a deleted object, an endless loop stopped, Scatter from a script, parameters, placement, hierarchy and materials, the command's one step and its error message |
| 13.5 ✅ | M | **The Script Manager** — done 2026-10-31 per [`reference/extend-scripting.md`](reference/extend-scripting.md): the library (`ui/src/scripts.rs`, `Scripts` in `EditorState::scripts`: `<storage dir>/scripts` given by the binary, made with five example scripts built in from `crates/ui/scripts/` — Grid of Cubes, Number Selected Objects, Random Colours, Scatter on the Last Selected, Spread Selected Objects — listed by name, safe names only, the edited script, its unsaved mark, a console of 1000 lines); the window (`panels/script_manager.rs`: New, Open from the library, Save under the Name field, Run and Ctrl+Enter in the editor, line numbers, the console with errors in the warning colour, Clear); **Script ▸** lists the library (a click runs one), then Script Manager… and Run Script File…; `EditorCommand::{RunLibraryScript, NewScript, OpenScript, SaveScript}`; output and errors go to the console, and into a message only while the window is closed; tests: the examples installed once, save / open / names, the console's length, every example running on the demo scene, Ctrl+Enter, the console from commands, a name needed to save |
| 13.6 ✅ | M | **SDK documentation and finish** — done 2026-10-31: [`plugins.md`](plugins.md) (adding a plugin crate, `Plugin` and the parameter schema, generators, deformers, commands with `Context` and `Changes`, missing plugins, translations, testing, a checklist) and [`scripting.md`](scripting.md) (running scripts, Rhai in a page, values, the whole API, the library's examples and more, errors) — a test runs every Rhai block of it on the demo scene and checks the built-in examples are in it word for word; the manual's Plugins chapter (21: objects and deformers, commands, missing plugins) and Scripts chapter (22) with a pointer from Creating objects; Ctrl+Enter in the shortcut lists; the Greek checked on screen — a long translated Fit to Parent hint widened the Object tab's middle column and hid the key buttons of deformers, now wrapped; the phase's acceptance test (the Gear edited, undone, keyed, overridden in a take, saved and saved back unchanged) |

**Acceptance:** the Gear generator is created from Create ▸ Plugins, its
parameters edit, animate, undo, override in a take and save; the same file
opened in a Grid4D without the plugin shows the object as missing, and saved
there it still opens with the gear in a Grid4D that has it; the Wave
deformer bends a cube and is animated; Scatter places copies in one undo
step; a script that builds a coloured grid of cubes runs as one undo step,
an error points at its line, an endless loop is stopped; everything new
reads in Greek.

## Phase 14 — Stabilisation (v1.0)

**Goal:** Grid4D 1.0 — something a stranger can download, install, use and
trust. Nothing big is added; what is there becomes dependable: the `.g4d`
format is **frozen** and documented, files and edits survive random and
broken input, a crash leaves a report and loses no work, the performance
budget is **measured and met**, every licence is in order, and each tagged
release builds **installable packages for Windows, macOS and Linux**. The
documentation is complete for users, plugin and script writers and
contributors.

**Behaviour specs** (our own, for what users see): `reference/app-release.md`
— the version and licences in Help ▸ About, the crash report and the offer
to recover, the frame-time readout (14.3–14.5).

### Where things stand (2026-11-01)

* CI: rustfmt, clippy and tests on Windows / Linux / macOS, MSRV 1.95 — all
  green. Its actions still run on Node.js 20 (deprecated: `actions/checkout@v4`).
  No `cargo deny`, no fuzzing, no release builds.
* `.g4d` version 3 has grown additively since Phase 3 (golden fixtures
  `demo_v3.g4d` and `demo_v3_initial.g4d`; versions 1 – 2 read-only). It is not
  documented outside the code.
* Tests: 751 unit and integration tests, the random undo/redo round trip,
  `extract_reuses_capacity`, golden files and the path tracer's reference
  image. Not yet: property tests of file round trips, headless UI tests
  (`egui_kittest`), the counting allocator for steady frames.
* Benchmarks: one (`falling_cubes`). The performance budget (below, under
  Cross-cutting) has never been measured end to end.
* A panic ends the program; autosave (12.3) recovers unsaved work on the next
  start, but nothing records what happened.
* The workspace version is still `0.1.0`; there is no changelog and no list
  of third-party licences (the Ubuntu font egui embeds has its own licence).
* egui 0.37 / wgpu 30 are the newest releases — no major upgrade is due
  before 1.0 (R1).

### Design

* **The format freeze** (decision D20). 14.2 reviews version 3 field by field
  against the scene model (names, units, defaults, what is optional) and
  decides: freeze v3 as it is, or — if something must change — write v4 with a
  migration from v3 and freeze that. From 1.0 on a released version never
  changes: new data needs a new version and a migration, older versions keep
  opening, and Grid4D refuses (with a clear message) files from newer
  versions. `docs/FILE_FORMAT.md` specifies the frozen version (the zip's
  entries, `document.json` field by field, the binary mesh and simulation
  layouts, the rules for unknown plugin values), so other programs can read
  and write `.g4d`.
* **Hardening by machine** (14.2–14.3): `proptest` generates random
  documents (objects of every kind, hierarchy, tags, tracks, materials,
  layers, takes, plugin values) and checks save → load gives them back; a
  `cargo fuzz` target feeds `grid4d_io::from_bytes` mutated archives
  (seeded with the fixtures) and must never panic, hang or allocate what a
  file merely claims — run for a while on every push to `main` (Linux,
  nightly) and longer weekly; a random-command test runs long sequences of
  `EditorCommand`s with undo and redo on the demo scene and checks the
  invariants (no panic, undo all returns the start, redo all returns the
  end, every id valid).
* **Crash reports, local only** (decision D23): a panic hook writes a report
  (version, commit, OS, GPU, the panic message and a backtrace, the last log
  lines) into `<data dir>/crashes/`, and the next start offers to recover the
  autosaved work and shows where the report is. Nothing is sent anywhere.
* **Performance, measured**: `criterion` benches for each hot path that can
  run without a GPU — evaluation of 10 000 objects (static and animated),
  `FrameData::extract`, triangulation and subdivision of a 1 M-triangle mesh,
  generators, `.g4d` save / load of a 50 MB document, the path tracer on a
  small scene — and the **counting global allocator** test that a steady
  frame of evaluation and extraction allocates nothing. A stress-scene
  generator (`cargo run --release --example stress`) writes the budget's
  scenes as `.g4d` files; a **frame-time readout** (View ▸ Show ▸ Frame
  Time, off by default) shows the frame time and the slowest part; the GPU
  budgets are measured by hand on the reference machine (RTX 3050 Laptop,
  Windows 11) and recorded in `docs/performance.md`, with each miss fixed or
  explicitly accepted.
* **Licences and supply chain** (decision D21): `cargo deny` in CI
  (licences on an allow-list — MIT, Apache-2.0, BSD, ISC, Zlib, Unicode, MPL-2.0
  only where a file-level copyleft is acceptable, the Ubuntu Font Licence for
  the embedded font — advisories, duplicate major versions reported, crates
  only from crates.io); `cargo about` generates `THIRD_PARTY_LICENSES.txt`,
  shipped with every package and shown from Help ▸ About ▸ Licences.
* **Releases from a tag** (decision D19): pushing `v1.0.0` runs a release
  workflow that builds optimised binaries on the three systems and publishes
  a GitHub Release with: Windows `.zip` (the exe with its icon and version
  resources) and an `.msi` installer (Start menu entry, `.g4d` file
  association); macOS `.dmg` holding `Grid4D.app` (Info.plist, `.icns` icon,
  `.g4d` document type; universal binary: x86-64 and arm64); Linux `.tar.gz`
  and an **AppImage** (with a `.desktop` file and icons); SHA-256 sums for
  all. Packages are **not code-signed** for 1.0 (the README explains how to
  open them on Windows SmartScreen and macOS Gatekeeper); signing is a later
  step. The version follows SemVer (decision D22): 1.x minor versions add
  features and open every 1.x file; the plugin API (`grid4d-plugin`) and the
  script API may grow, and only a major version may break them.
* **The interface pass** (14.5): headless UI tests with `egui_kittest` for
  what posted window messages cannot reach on screen — Object Manager drag
  and drop, renaming, typing into fields, dragging viewport handles,
  Shift / Ctrl clicks — and a walkthrough of the whole manual on screen in
  English and Greek and in the Dark and Light themes, fixing what reads or
  behaves wrong. One table of shortcuts feeds Help ▸ About, and a test
  keeps the manual's table equal to it.
* **Not in Phase 14:** new modelling, animation or rendering features; code
  signing and notarisation; automatic updates; telemetry; a web build;
  dynamic plugins.

### Steps

| Step | Size | Work |
|------|------|------|
| 14.1 ✅ | S | **CI and supply chain** — done 2026-11-01 per [`reference/app-release.md`](reference/app-release.md): actions on Node 24 (`actions/checkout@v7`; `Swatinem/rust-cache@v2` already on Node 24); `deny.toml` (permissive licences, the Ubuntu font's `Ubuntu-font-1.0` and `OFL-1.1`, MPL-2.0 only for `smartstring` — Rhai's, used unmodified; vulnerabilities and yanked crates fail, duplicates warn, crates.io only) and a `cargo deny` CI job (advisories reported without blocking); `ttf-parser` (grid4d-text) is unmaintained (RUSTSEC-2026-0192): ignored with a reason, **to be replaced by `skrifa`**; `about.toml` + `about.hbs` → `assets/THIRD_PARTY_LICENSES.txt` (plain text, CR removed; 184 licence texts) and a CI job that regenerates it and fails on a difference; `build.rs` gives the commit and its date (`GRID4D_COMMIT`, `GRID4D_COMMIT_DATE`, deterministic), `BuildInfo` in `EditorState`; Help ▸ About shows "Version 0.13.0 (commit, date)" and a Licences page (rows laid out only in view, unwrapped); the workspace version 0.13.0; `cargo update` (49 crates, yanked `clipboard-win` gone) |
| 14.2 ✅ | L | **File format freeze** — done 2026-11-01: the review of v3 found nothing that must change (every later addition optional with its default, references by index, angles in radians except the two `_deg` fields) — **v3 is frozen as it is**, no v4; [`FILE_FORMAT.md`](FILE_FORMAT.md) specifies it (the zip, `document.json` field by field, `meshes/<n>.bin`, `simulation.bin`, limits, versions); a second golden fixture `full_v3.g4d` holds everything v3 can (every kind, tag, track type, plugin value type, materials, layers, takes, settings, a simulation with a clone) and must save back byte for byte after loading; `proptest` round trips of random documents (256 a run; 3 000 tried) through save → load → save, equal bytes; `damaged.rs` (stable): fixtures with flipped, cut, inserted and overwritten bytes, and valid archives with hostile `document.json` (numbers out of range, wrong types, lines gone or doubled) — 81 000 tried, none panics, whatever loads saves and loads again; the `cargo fuzz` target `fuzz/` (outside the workspace, `load`: what loads must save and load again), a 2-minute CI job and a weekly / on-demand `Fuzz` workflow keeping failing inputs; **found and fixed:** loading normalised every saved rotation (a file did not save back identically) and turned a zero quaternion into NaNs |
| 14.3 ✅ | M | **Robustness** — done 2026-11-01 per [`reference/app-release.md`](reference/app-release.md): `ttf-parser` replaced by `skrifa` 0.44 in grid4d-text (already in the tree through egui; outlines identical to the byte; the `deny.toml` exception gone — `ttf-parser` stays only under winit's Wayland decorations); crash reports (`ui::crash`: `install_log` keeps the last 200 log lines, `install` sets a panic hook writing `<data>/crashes/crash-<UTC>-<pid>.txt` with version, system, adapter, thread, message, location, backtrace and log; `CrashReports` keeps the newest 20 and points to a new one once, in Recover Unsaved Work? or "Grid4D Stopped Unexpectedly", Show Report → `file::reveal`); the random-command test (`ui/src/random_commands.rs`: 12 seeds × 400 commands of ~60 kinds through `apply_pending`, undo / redo and frame changes mixed in; a history state always shows the same document, selections valid, undo all → start, redo all → end; 300 seeds tried); `ui::startup`: the data folder checked before the window (an alert on the first frame), "Grid4D Cannot Start" (rfd message box) when `run_native` fails; autosave's first failure an alert; `grid4d_io::read_file` reserves memory first (`OutOfMemory` error, never an abort) for open, merge, import and glTF buffers; a test that a save which cannot be written keeps the old file |
| 14.4 ✅ | L | **Performance budget** — done 2026-11-01, results in [`performance.md`](performance.md), spec [`reference/app-release.md`](reference/app-release.md): `criterion` benches `geom/benches/mesh.rs` (topology, BVH, triangulation, subdivision of 1 M triangles), `eval/benches/evaluate.rs` (10 000 objects after an edit and while playing; Subdivision Surface, Extrude of a text, Cloner of 1 000), `render/benches/extract.rs` (10 000 objects; component overlays of 1 M triangles), `io/benches/files.rs` (50 MB load / save), `raytrace/benches/trace.rs`; a CI job builds them (`cargo bench --workspace --no-run`); `render/tests/steady_frame.rs` counts allocations per thread (`allocation-counter`, dev only): playing, orbiting and selecting 10 000 objects allocate nothing; `examples/stress.rs` writes `stress-10k.g4d` (with a camera on a turning rig), `stress-1m.g4d`, `stress-50mb.g4d`; **View ▸ Frame Time** (`viewport/frame_time.rs`: averaged time between frames and fps, the worst of the last 128 frames, the slowest of Evaluation / Extraction / Drawing); an ignored, headless Select All timing; every budget met on the reference machine (10k playing 150 – 160 fps, 1 M triangles 132 – 196 fps in every mode, Select All one frame +5 ms, 50 MB open 1.3 – 1.7 s); `Topology::build` without hashing (580 → 57 ms; subdivision 74 → 32 ms) and the BVH build reading boxes in order (348 → 270 ms) |
| 14.5 ✅ | M | **The interface pass** — done 2026-11-02 per [`reference/app-release.md`](reference/app-release.md): `grid4d_ui::shell::Shell` (the whole interface; the binary only adds eframe storage); `crates/ui/tests/interface.rs` with `egui_kittest` — Shift / Ctrl selection, drag to re-parent, rename, a typed Coordinates value, a gizmo drag, a Bend handle drag, Create ▸ Object ▸ Torus, each with its undo; `shortcuts::TABLE`, the one list (About shows it, a test keeps manual §23 equal); accessible names (Coordinates fields by label, the fps menu, Object Manager rows as tree items); the walkthrough (`crates/ui/tests/walkthrough.rs`, ignored: the interface rendered headless with wgpu in English and Greek, Dark and Light — workspace, every menu, the view menus, the managers, the windows) instead of clicking on screen (posted mouse input no longer reaches the window); fixed: untranslated Coordinate Manager titles, characters outside the built-in font (▸ → ›, ✕ → ×), the Optimize dialog's width; egui's `default_fonts` for the tests (egui 0.37 has none by default); fresh `shortcuts.png` and `optimize.png` |
| 14.6 ✅ | L | **Packaging and the release workflow** — built 2026-11-02 per [`reference/app-packaging.md`](reference/app-packaging.md): version `1.0.0-rc.1`; `assets/icon/png/` (16 – 1024, rendered from the SVG masters) and `grid4d.icns`; `assets/macos/Info.plist` (document type, version from the tag), `assets/linux/grid4d.desktop` + `grid4d.xml` (MIME type), `assets/windows/grid4d.wxs` (WiX 5, Start menu, `.g4d` association, upgrades); `.github/workflows/release.yml` (version check against the tag; Windows `.zip` + `.msi`, macOS universal `.dmg` with an ad-hoc signed bundle, Linux `.tar.gz` + AppImage on Ubuntu 22.04; SHA-256 sums; a draft release from the `CHANGELOG.md` section; butler to itch.io when `BUTLER_API_KEY` exists; a dry run by hand); `CHANGELOG.md` begun; the Windows `.zip` built and started locally. **Dry run on GitHub:** every package built (2026-11-02); the Windows `.zip` and `.msi` tried (installed, started, uninstalled). **Left:** the `.dmg` and AppImage on a Mac and a Linux system. Planned: **Packaging and the release workflow:** the workspace version to `1.0.0-rc.1`; `assets/` for packaging (`.icns`, Linux icons, `.desktop`, `Info.plist`, WiX source); the release workflow (on a `v*` tag: three builds, the `.zip`, `.msi`, `.dmg`, `.tar.gz`, AppImage, SHA-256 sums, a draft GitHub Release with the changelog's section; the same packages pushed to itch.io with `butler` — channels `windows`, `mac`, `linux`, the version from the tag, a `BUTLER_API_KEY` secret, skipped when it is absent; the page's fields in [`assets/itch/page.md`](../assets/itch/page.md)); a release candidate built and each package installed and opened on a clean machine or VM of each system (the demo, every example file, a script, a plugin object) |
| 14.7 ✅ | M | **Documentation and 1.0** — done 2026-11-02: README for 1.0 (download table, features, fresh pictures from a showcase scene rendered headless), `CHANGELOG.md` (1.0.0 and the road by version), CONTRIBUTING (where things go, releases), the R2 review (interface names no other product; docs name Cinema 4D only with the trademark note; "MoGraph" removed), the manual's stale notes; version `1.0.0`. **Left:** the `v1.0.0` tag. Planned: **Documentation and 1.0:** `docs/ARCHITECTURE.md` (the crates and their rules, the data flow of a frame, where to add things — first written 2026-11-01; brought up to date), `CHANGELOG.md` (1.0.0, and the road there by phase), the README for 1.0 (what Grid4D is, screenshots, installing, building, documentation), `CONTRIBUTING.md` (the rules, the docs to update, how to add a panel / object / plugin), a final pass over the manual and guides; the trademark and trade-dress review (R2) of menus, labels, icons and docs; the version to `1.0.0` and the `v1.0.0` tag after the acceptance |

**Acceptance:** CI green with `cargo deny` and the short fuzz run; the
`v1.0.0` tag produced a GitHub Release with the Windows `.zip` and `.msi`,
the macOS `.dmg` and the Linux `.tar.gz` and AppImage, each installed and
started on a clean system, opening the demo and every example document and
running a script; `.g4d` frozen and specified in `docs/FILE_FORMAT.md`, files
of every earlier version opening; the fuzz target ran for at least four hours
without a failure; every budget of the performance table measured on the
reference machine and met or accepted in `docs/performance.md`; a forced
panic leaves a report and the next start recovers the unsaved work; Help ▸
About shows 1.0.0 and the licences; the manual walked through in English and
Greek without a fault; `CHANGELOG.md`, `ARCHITECTURE.md`, `FILE_FORMAT.md`,
the README and the guides complete.

---

## After 1.0 — the road to a complete suite

> Written 2026-11-02, before 1.0 is tagged. Phase 15 is planned in detail;
> Phases 16 – 24 are outlined (goal, scope, the order of their steps) and get
> their full design when they start, as every phase so far did.

### Where Grid4D stands at 1.0 — an honest comparison

What an artist who knows the established 3D suites (the commercial one whose
workflow Grid4D follows, and the leading open-source one) will look for, and
how far Grid4D 1.0 is from it.

| Area | Grid4D 1.0 | What artists expect | Gap |
|------|-----------|---------------------|-----|
| Polygon modelling | Components, extrude, inset, bevel, bridge, cuts, weld, melt, dissolve, optimize, snapping, SDS | Booleans, symmetry, soft selection, slide, knife variants, edge creases, normal editing, remesh, polygon reduction | **Medium** |
| Procedural modelling | Extrude, Lathe, Sweep, Loft, Text, 5 deformers | Booleans and symmetry as live objects, instances, spline masks, many more deformers (wrap, spline, displacer, shrink-wrap, smooth) | Medium |
| Sculpting and painting | — | Brushes, multi-resolution, masks, vertex and weight painting | **High** |
| UVs and textures | Imported UVs only; no images | UV editor, unwrap, projections, image textures in viewport and render | **High** |
| Materials | Colour, roughness, metallic, emission | Layered PBR (normal / bump maps, transparency, glass, subsurface, clearcoat), material previews, node graphs | **High** |
| Lighting and rendering | Omni / spot / sun, CPU path tracer, PNG / EXR | Area lights, sky and HDRI, glass, depth of field, motion blur, render passes, denoising, GPU rendering, real-time preview | **High** |
| Animation | Keys on transforms and parameters, F-curves, autokey | Dope sheet, motion layers / clips, constraints, path animation, drivers / expressions, onion skin, sound | Medium |
| Characters | — | Joints, skinning, weights, IK, pose morphs, skinned glTF | **High** |
| Motion graphics | Cloner (4 modes), Plain and Random effectors | A fields system (falloffs that layer), a dozen effectors, matrix, text cloning, fracture, tracer | **High** (this workflow's signature) |
| Simulation | Rigid and collider bodies | Particles and forces, cloth, soft bodies, ropes, hair; fluids later | High |
| Pipeline | `.g4d`, glTF, OBJ | FBX, Alembic, USD; materials, skins and animation in glTF; STL / PLY | Medium |
| Extending | Rhai scripts, Rust plugins built in | Python, node-based expressions, plugins loaded at run time, an asset library | Medium |
| Interface | Docking layouts, themes, EN / EL | Node editor, asset browser, customisable shortcuts, more languages, a navigation cube | Medium |

**Where Grid4D can win** (and what the phases protect): a calm, familiar
layout without hunting through modes; motion graphics as a first-class
workflow; a small, fast native program that opens in a second; free and open
source with a frozen, documented file format; undo for everything. **What
Grid4D will not chase:** a video editor, a game engine, 2D drawing in 3D,
compositing — the open-source suite does those, and they are not why people
choose this workflow.

**The order** follows one question: *can someone finish a real project?*
Today an object cannot carry a picture, so every render looks like plastic —
materials and textures come first. Then the render itself (light, glass,
noise), then the modelling power and the procedural tools that define the
workflow, then characters, simulation, nodes, speed and the pipeline.

### The release train after 1.0

| Version | Phase | Theme | "Done" means |
|---------|-------|-------|--------------|
| v1.1 | 15 | Materials, UVs and textures | Image textures on every channel, UVs on everything, a UV editor with unwrapping, textures in the viewport and the render, glTF / OBJ materials in and out |
| v1.2 | 16 | Render quality | Area lights, sky and HDRI, glass and transmission, depth of field, motion blur, render passes, a denoiser, a render queue |
| v1.3 | 17 | Modelling 2 | Booleans, Symmetry, Instance and Connect objects, soft selection, slide, knife variants, edge creases, more deformers |
| v1.4 | 18 | Fields and motion graphics 2 | Layered fields for effectors, deformers and selections; vertex maps; a dozen effectors; Matrix, text cloning, Fracture, Tracer |
| v1.5 | 19 | Sculpting and painting | Brushes on a multi-resolution mesh, masks, symmetry; vertex colour and weight painting |
| v1.6 | 20 | Characters and animation 2 | Joints, skinning, weights, IK, constraints, pose morphs; a dope sheet and motion layers; skinned glTF |
| v1.7 | 21 | Simulation 2 | Particles and forces, cloth and soft bodies (XPBD), ropes; caches on disk |
| v1.8 | 22 | Nodes | A node editor: material graphs and expressions that drive parameters |
| v1.9 | 23 | GPU rendering | The path tracer on the GPU (wgpu compute), matching the CPU reference; a real-time preview mode |
| v2.0 | 24 | Pipeline | FBX, Alembic and USD where licences and pure-Rust tools allow; Python scripting; an asset library; plugins at run time |

Each minor version keeps the rules of 1.x (D22): every older file opens, the
plugin and script APIs only grow. A version that adds document data brings a
new `.g4d` format version with a migration (D24) — one per release at most.

---

## Phase 15 — Materials, UVs and textures (v1.1)

**Goal:** an object can wear a picture. Every primitive and generator has
UVs; a Texture tag projects them where needed; a UV editor shows, edits and
unwraps them; materials take image textures on every channel; the viewport
and the path tracer show them; glTF and OBJ carry materials and textures in
and out.

### Where things stand (2026-11-02)

* `Material` holds four numbers (colour, roughness, metallic, emission);
  `Tag::Material` links one; children inherit (`Scene::effective_material`).
* `PolyMesh` can hold one UV per corner (`set_uvs`), filled only by glTF / OBJ
  import; primitives and generators make none; mesh operations carry UVs
  linearly.
* No image is ever decoded except PNG for tests; the viewport shader has no
  texture bindings; the path tracer shades from the four numbers.
* `.g4d` v3 is frozen (D20): any of this data needs v4.

### Design

* **The `.g4d` format v4** (D24): materials with channels, the document's
  textures (each linked by a path relative to the document, or embedded in
  the archive as `textures/<n>.<ext>`), Texture tags, UVs on polygon objects
  already stored. A migration from v3 (materials' four numbers become the
  base channels); `FILE_FORMAT.md` gains v4; a v4 golden fixture; v3 stays
  readable forever. Grid4D 1.1 writes v4; 1.0 refuses v4 with its existing
  "newer version" message.
* **Images** (D25): decoded with the `image` crate limited to PNG, JPEG,
  TIFF, WebP, HDR and EXR (permissive licences, no C); loaded in the
  background, cached by path and modification time, mip-mapped once;
  colour data in sRGB, data maps (roughness, normal, bump) linear — a
  per-texture *colour space* setting with the usual default by channel.
  A missing file shows a magenta checker and a warning in the Texture
  Manager, never an error on opening.
* **The material model** (D26): one fixed, principled set of channels —
  *Base colour, Metallic, Roughness, Specular, Normal / Bump, Emission,
  Opacity (cut-out), Transmission + IOR, Clearcoat* — each a value or a
  texture (with tiling, offset and a value range), plus a *Two-sided*
  switch. The viewport, the path tracer, glTF and OBJ all read this one
  model; Phase 22's node graphs will compile into it. Our own names and
  layout, not another program's channel list.
* **UVs everywhere:** every primitive, spline generator, Subdivision Surface
  and Cloner result makes UVs at evaluation (per corner, in the mesh cache);
  Make Editable keeps them. The **Texture tag** (material + projection:
  *UV, Box, Spherical, Cylindrical, Flat, Camera*, with offset, tiling and
  rotation) evaluates per corner in `grid4d-eval`; several Texture tags
  layer in order (the last one wins where it is opaque).
* **The UV editor** (a pane): the active object's UVs in 2D over its
  texture; Points / Edges / Polygons selection shared with the viewport;
  move, scale, rotate; **Unwrap** (LSCM, from the selected polygons or the
  whole mesh, seams from the selected edges or sharp edges), **Project**
  (the Texture tag's projections baked into the mesh's UVs), **Pack
  islands**, **Relax**. All of it pure geometry in `grid4d-geom` (`uv.rs`),
  undoable like any mesh edit.
* **Viewport:** textures uploaded once per image (GPU memory budgeted,
  smaller mips first), a texture array per material, normal maps in the
  shader (tangents computed per corner, MikkTSpace-compatible), opacity
  cut-out, emission maps; shading modes unchanged otherwise. The
  allocation-free frame rule holds: uploads happen when an image arrives,
  not per frame.
* **Path tracer:** bilinear texture sampling with mip selection from the
  ray's footprint, normal and bump mapping, opacity cut-out, transmission
  (glass, rough glass) with IOR; the reference image test grows a textured
  scene.
* **Material Manager:** a preview ball per material (a small path-traced
  render in the background, cached), drag a texture file onto a channel or
  a material onto an object; a **Texture Manager** listing every image the
  document uses (missing ones flagged, *Relink…*, *Embed / Unembed*).
* **Interchange:** glTF PBR metallic-roughness with textures (embedded
  `.glb` images, external `.gltf` images), `KHR_texture_transform`; OBJ
  `.mtl` maps (`map_Kd`, `map_Bump`, `map_Ns`, `map_d`); export writes them
  back.

### Steps

| Step | Size | Work |
|------|------|------|
| 15.1 ✅ | M | Specs (`reference/material-channels.md`, `uv-editor.md`, `texture-tag.md`, `format-v4.md`); `.g4d` v4 with the migration from v3, `FILE_FORMAT.md` v4, a v4 golden fixture, the fuzz target and property tests on v4 — done 2026-11-06: five specs ([`app-format-v4.md`](reference/app-format-v4.md), [`material-channels.md`](reference/material-channels.md), [`material-textures.md`](reference/material-textures.md), [`tag-material.md`](reference/tag-material.md) — the "Texture tag" is the existing Material tag grown by projections and layers —, [`manager-uv-editor.md`](reference/manager-uv-editor.md)); version 3's module became `v4.rs` (the current format, additive until 1.1 ships), the new `v3.rs` upgrades a version 3 `document.json` as JSON and removes whatever version 4 adds; golden `demo_v4.g4d` / `full_v4.g4d`, the `_v3` fixtures frozen and checked to read exactly as before; a property test of documents labelled version 3; the fuzz corpus holds both versions. Each later step adds its own data to v4 |
| 15.2 ✅ | M | UVs from every primitive and generator (evaluation, mesh cache, Make Editable, export); tests that every kind's UVs cover 0 – 1 without NaNs — done 2026-11-06: `grid4d_geom::uv` (conventions, planar and box projections, `flipped_faces`); every primitive's unit mesh has UVs (size-independent); Extrude, Lathe, Sweep and Loft walls and caps; the Subdivision Surface's merged cage keeps UVs (Box for parts without, mirrored parts mirrored); plugin generators without UVs get Box; Make Editable, Current State and glTF / OBJ export carry them; layouts in [`tag-material.md`](reference/tag-material.md); tests that every kind's UVs lie in 0 – 1, are finite and show no face mirrored |
| 15.3 ✅ | L | Images: the `image` crate (limited features, `cargo deny` / licences), background loading and cache, mip-maps, colour spaces, the document's texture table (linked / embedded), the Texture Manager pane, missing-file handling — done 2026-11-06: `scene::Textures` (`SceneEdit::SetTextures`), `grid4d_io::decode_texture` (six formats under limits, mip-maps, half floats for wide pictures), `.g4d` v4 `textures` + `textures/<n>.<ext>` (relative paths against the file's folder), `ui::textures::TextureCache` (one worker thread, file changes noticed), the Texture Manager pane with Add / Relink / Relink Folder / Embed / Unembed / Reveal / Delete / Remove Unused, dropped pictures, a status-bar count of missing pictures; Greek; licences regenerated |
| 15.4 | L | The material channels: scene model, `SceneEdit`s, the Material Manager and Attribute Manager pages, preview balls, the Texture tag with its projections — split in three: **15.4a ✅** (2026-11-06) the channels (`Material` with `Channel` / `TextureSlot`, v4 `materials` fields and `maps`, the Attribute Manager's grouped page with a texture row per channel and Load…, Remove Unused aware of them); **15.4b ✅** (2026-11-07) the Material tag's projections and layers (`MaterialTag`: projection, offset, tiling, rotation, placement or Fit to Object, camera; several per object; `grid4d_geom::uv::project`; `MaterialLayer`s in evaluation; Shift + drop adds a layer; Bake to UVs); **15.4c ✅** (2026-11-07) preview balls (`ui::previews`: a path-traced ball per material on a worker thread, one at a time, transparent background, in the Material Manager and the Attribute Manager) |
| 15.5 | L | Viewport textures: GPU upload and budget, sampler and mip-maps, tangents and normal maps, opacity, emission; shader size tests; a counting-allocator frame with textures |
| 15.6 | M | Path tracer: texture sampling, normal / bump, opacity, transmission and IOR; a textured reference image |
| 15.7 | L | The UV editor pane: display over the texture, selection, transform; Unwrap (LSCM), seams, Project, Pack, Relax — algorithms in `grid4d-geom::uv` with tests (no flipped faces, area distortion bounds) |
| 15.8 | M | glTF and OBJ materials and textures, in and out; round-trip tests against sample files |
| 15.9 | S | Manual chapter "Materials and textures" with screenshots in both languages; the walkthrough extended; performance budget re-measured with textured scenes; release v1.1 |

### Acceptance

A user imports a textured glTF model and it renders with its textures; makes
a cube, applies a photo with a Box projection, unwraps a character-like mesh
from marked seams, paints nothing but arranges the UV islands in the UV
editor, gives a glass material to a sphere and a brushed-metal one (with a
normal map) to a plane, and renders a still that looks like the viewport —
saved as `.g4d` v4, reopened identically, exported to glTF and read back with
its materials. A 1.0 file opens in 1.1 unchanged.

**Not in Phase 15:** texture painting, procedural noise textures (Phase 22's
nodes), displacement (Phase 16), UDIMs, video textures.

---

## Phase 16 — Render quality (v1.2) — outline

**Goal:** stills and animations that can be shown to a client: real light,
real glass, clean images in reasonable time.

* **Lights:** area lights (rectangle, disc, sphere; visible or not), a
  physical sky with a sun, an HDRI dome (equirectangular EXR / HDR, importance
  sampled), light linking (include / exclude), light groups.
* **Materials in the render:** subsurface scattering (random walk), thin
  film and sheen if cheap; displacement (subdivided at render time).
* **Camera:** depth of field (aperture, focus distance, a focus object),
  motion blur (camera and objects, deformation), bokeh shape.
* **Output:** render passes / AOVs (beauty, alpha, depth, normal, albedo,
  object and material IDs, light groups) into multi-layer EXR; a render
  queue (several documents or takes, one after another); a render region.
* **Speed and noise:** adaptive sampling; a **denoiser** guided by albedo
  and normal (D27: our own first, an optional OIDN build later); the Render
  View re-rendering while editing (interactive preview).
* Steps (outline): area lights and sky → HDRI and importance sampling →
  depth of field and motion blur → passes and multi-layer EXR → adaptive
  sampling and the denoiser → render queue and region → interactive Render
  View → SSS and displacement → manual and release.

## Phase 17 — Modelling 2 (v1.3) — outline

**Goal:** the modelling tools a hard-surface or product artist reaches for
every day.

* **Live objects:** Boolean (union, subtract, intersect; robust predicates,
  quads where possible), Symmetry (with welding at the plane), Instance,
  Connect (join children into one mesh), Spline Mask (2D booleans of
  splines), Polygon Reduction.
* **Tools:** soft selection (proportional falloff for every transform),
  slide (edges and points), knife variants (plane, loop, path), polygon pen,
  extrude along a spline, mirror, array of selection, bevel improvements
  (profiles, point mode), edge creases and weights for the Subdivision
  Surface, normal editing (smoothing groups, weighted normals).
* **More deformers:** wrap, spline, displacer (with Phase 15's textures),
  shrink-wrap, smoothing, correction, explosion.
* Risks: robust mesh booleans (R6) — budget a spike step first; every
  operation keeps component selections through `Rebuilt::carry`.

## Phase 18 — Fields and motion graphics 2 (v1.4) — outline

**Goal:** the procedural animation workflow that is this kind of program's
signature — strong enough to make broadcast-style animation without keys.

* **Fields:** shape fields (linear, spherical, box, cylinder, cone, torus,
  radial), random and noise fields, a formula field, a field from a texture;
  **layers** with blend modes, remapping, delay and decay; one field list
  drives effectors, deformers, selections and **vertex maps**.
* **Effectors:** shader, delay, formula, step, time, target, inheritance,
  push apart, volume, spline.
* **Objects:** Matrix (placements without geometry), text cloning (letters
  as clones), Fracture (by connected parts or Voronoi), Tracer (trails as
  splines), a Cloner on splines and on a polygon's surface by area; clone
  selections and weights.
* Naming: our own names throughout (R2); "motion graphics" in the
  interface, never another product's brand terms.

## Phase 19 — Sculpting and painting (v1.5) — outline

**Goal:** organic shapes and painted data.

* A multi-resolution mesh (subdivision levels kept as offsets), brushes
  (grab, draw, smooth, inflate, pinch, crease, flatten, clay, mask), stroke
  options (spacing, falloff, pressure from a tablet), symmetry, masks and
  freezing; baking detail into normal / displacement maps (with Phase 15).
* Painting **vertex colours and vertex maps** (weights) with the same brush
  engine — the base of Phase 20's skin weights.
* Performance: brushes on a million polygons at interactive rates
  (partial BVH updates, GPU upload of changed ranges only).

## Phase 20 — Characters and animation 2 (v1.6) — outline

**Goal:** rig and animate a character, and organise longer animations.

* **Rigging:** joints and hierarchies, a skin deformer with weights
  (painted in Phase 19's engine, automatic weights by heat diffusion), IK
  (two-bone and chain, pole targets), constraints (parent, aim, position,
  rotation, path), pose morphs (blend shapes), mirroring a rig.
* **Animation:** a dope sheet, motion layers / clips with blending, path
  animation (align to spline), onion skinning, a sound track for timing.
* **Interchange:** glTF skins, morph targets and animation, in and out.

## Phase 21 — Simulation 2 (v1.7) — outline

**Goal:** the effects that rigid bodies cannot do.

* Particles: emitters (from objects, surfaces, splines), forces (gravity,
  wind, turbulence, attractor, drag, vortex), collisions with the scene,
  particles rendered as clones, points or spheres.
* Cloth and soft bodies on XPBD (one solver, in `grid4d-sim`), ropes and
  simple hair; pinning, self-collision, tearing later.
* Caches on disk beside the document (our own format, streamed per frame),
  so long simulations neither live in memory nor in the `.g4d`.

## Phase 22 — Nodes (v1.8) — outline

**Goal:** one visual graph editor for what fixed panels cannot express.

* A node editor pane (typed sockets, groups, comments, search-to-add,
  undo through `SceneEdit`s like everything else).
* **Material graphs** compiling to Phase 15's channels (and a WGSL / path
  tracer evaluator): procedural noises, gradients, mixing, maths.
* **Expression graphs** driving any animatable parameter from others (time,
  distance, user data) — our own design, evaluated in `grid4d-eval` each
  frame without allocation (compiled once per edit).

## Phase 23 — GPU rendering (v1.9) — outline

**Goal:** final renders an order of magnitude faster.

* The path tracer on wgpu compute (BVH on the GPU, wavefront kernels),
  matching the CPU tracer's reference images within noise (D28); the CPU
  tracer stays the reference and the fallback.
* A real-time preview mode in the viewport (image-based lighting,
  screen-space reflections, the Phase 15 materials) for look development.
* Risks: compute portability across Vulkan / Metal / DX12 (R7); a GPU memory
  budget for big scenes.

## Phase 24 — Pipeline (v2.0) — outline

**Goal:** work with every studio's other tools.

* FBX (through a permissively licensed reader; a C dependency only if
  packaging and `cargo deny` allow it — D29), Alembic caches, USD export
  (USDA / USDC written by our own code), STL / PLY.
* Python scripting beside Rhai (an optional component, D30), an asset
  library (objects, materials, presets) with thumbnails, plugins loaded at
  run time (WebAssembly, sandboxed), customisable shortcuts, more languages.
* 2.0 is the version that may change the plugin and script APIs (D22).

---

## Cross-cutting: testing, performance, CI, docs

### Testing

* Unit tests next to the code (current style) for every new type and operation;
  property tests (`proptest`) for `SceneEdit` inverses, mesh ops topology
  invariants and file round-trips.
* UI tests with `egui_kittest` (headless) for panel interactions that matter:
  Object Manager drag-reparent, rename, Attribute Manager edits → undo.
* WGSL: every new shader added to `shaders_validate`; every `#[repr(C)]` struct
  gets a size/alignment test.
* Golden files: `.g4d` fixtures per format version; reference images for the
  path tracer.

### Performance budget (tracked from Phase 3 on, `criterion` benches in `benches/`)

| Scenario | Budget (release, mid-range GPU) |
|----------|----------------------------------|
| 10 000 primitive objects, orbiting | ≥ 60 fps, 0 allocations/frame in eval/extract/prepare/paint |
| 1 M triangles single mesh | ≥ 60 fps orbit; component display ≥ 30 fps |
| Select All on 10 k objects | no frame-time spike (A1 fix) |
| Load 50 MB `.g4d` | < 2 s |

Add a counting global allocator in a test-only binary to assert "zero allocations
per steady-state frame" for the hot path, complementing `extract_reuses_capacity`.

### CI

`fmt --check` · `clippy -D warnings` · `test` on Windows/Linux/macOS · MSRV job
(1.95) · from Phase 14: `cargo deny` (licences, advisories), fuzzing the `.g4d`
parser (`cargo fuzz`, nightly: short on push, long weekly), `cargo bench
--no-run`, and the release workflow on `v*` tags (D19).

### Documentation

* [`ARCHITECTURE.md`](ARCHITECTURE.md) is the rules file (crates, data flow per
  frame, rules, where to add things); update it when a phase adds crates or
  panels.
* `docs/FILE_FORMAT.md` (versioned `.g4d` spec), user-facing shortcut list
  generated from `shortcuts.rs` so Help ▸ About never drifts.

---

## Risks & open decisions

| # | Topic | Recommendation | Decide by |
|---|-------|----------------|-----------|
| D1 | Undo of selection-only changes | Not separate history entries in v0.2; restored as part of each group. Revisit for component selection in Phase 4 (C4D does undo selections). | Phase 1 |
| D2 | Soft delete vs. id remapping for undo | Soft delete (keeps `NodeId`s stable; simplest correct option) | Phase 1 |
| D3 | File container | JSON v1 → zip(JSON + binary geometry) v2 | Phase 2 / 3 |
| D4 | n-gons vs. tris/quads only | Support n-gons in storage from the start | Phase 3 |
| D5 | Mesh undo granularity | Whole-mesh `Arc` snapshots; switch to deltas only if memory profiling demands | Phase 3 |
| D6 | CPU vs. GPU path tracer | CPU first (determinism, tests), GPU compute later | Phase 8 |
| D7 | License | MIT OR Apache-2.0 (Rust default, as the README proposes) | Phase 0 |
| R1 | egui/wgpu major upgrades (egui-wgpu pins wgpu's major) | Upgrade both together in a dedicated PR per egui release; never mix wgpu majors | ongoing |
| R2 | Trademark / trade-dress | Keep "inspired by" wording, original icons only, no C4D file formats; review menus/labels before each release | ongoing |
| D8 | Clones: nodes or instances | **Instances** in a separate `EvaluatedScene::instances()` list built after animation, so the node list keeps its pre-order invariants (animation, Target tags, component modes) and thousands of clones stay one instanced draw | Phase 10 |
| D9 | What can be animated | Real-valued object parameters (10.1); counts follow when a step needs them (the Cloner's count, rounded); enums and booleans are not (the reference animates them stepwise — revisit if needed) | Phase 10 |
| D10 | Physics engine | `rapier3d` 0.36 in its own crate (`grid4d-sim`), single-threaded for determinism; it brings glam 0.33 (via `glamx`) beside our 0.34, converted through arrays at the crate's edge — accepted until rapier moves to glam 0.34 (a hand-written solver would cost far more and be less robust) | Phase 11 |
| D11 | Simulation units | Metres inside the engine (its tolerances assume them), centimetres everywhere else; gravity shown in cm/s² | Phase 11 |
| D12 | Where the cache lives | Editor state, filled on demand within a frame budget and keyed by the simulation's inputs (selection and unrelated edits keep it); a baked cache is also saved in the `.g4d` (11.5) | Phase 11 |
| D13 | Docking library | `egui_tiles` 0.18 (egui 0.37, from egui's authors, serialisable tree) instead of `egui_dock`, whose newest release (0.21.1) is still on egui 0.36; no own splitter tree needed | Phase 12 |
| D14 | How takes reach panels and renders | The document always holds the active take's state; switching is one undoable swap of the overridden groups (Main's values kept beside the takes), edits in a take are recorded as its overrides (Auto Take), and renders apply a take to their own copy — so panels, evaluation and the renderer need no take awareness | Phase 12 |
| D15 | Translation keys | The English text is the key (`tr("Object Manager")`): readable code, English as the fallback, a test that finds every `tr` literal missing from the Greek table | Phase 12 |
| D16 | How plugins are added | Rust crates built into the binary and registered explicitly from a list in the composition root (`src/plugins.rs`), behind a `grid4d-plugin` API crate; plugin objects are one more `ObjectKind` holding a type id and schema-typed values, so undo, takes, layers, animation and files need no plugin awareness; documents with missing plugins open and save back unchanged. No dynamic libraries (`unsafe` is forbidden; Rust has no stable ABI) | Phase 13 |
| D17 | Scripting language | Rhai (`rhai` 1.27, pure Rust, sandboxed, no external runtime) rather than Python via `pyo3` (needs a Python installation, complicates builds and distribution) | Phase 13 |
| D18 | How scripts change the document | Only through `SceneEdit`s recorded while running on a copy, applied as one undo step when the script finishes (nothing on an error or stop); an operations limit stops endless loops | Phase 13 |
| R3 | Scope creep | Ship the release train in order; each phase must leave the app usable and tests green | ongoing |
| D19 | How releases are built | A workflow of our own on a `v*` tag: optimised builds on the three systems, a Windows `.zip` and WiX `.msi`, a macOS `.dmg` with a universal `Grid4D.app`, a Linux `.tar.gz` and AppImage, SHA-256 sums, a draft GitHub Release, the packages also on itch.io (`butler`) — rather than `cargo-dist`, so every step is visible and the app bundle and file associations are ours. Not code-signed for 1.0 | Phase 14 |
| D20 | The file format at 1.0 | Review v3 and freeze it (or a v4 with a migration, if the review finds something to change); released versions never change again — new data means a new version and a migration; every older version keeps opening; newer files are refused with a message; the frozen version is specified in `docs/FILE_FORMAT.md` | Phase 14 |
| D21 | Licences | `cargo deny` with an allow-list in CI, `cargo about` for the third-party licence file shipped with the packages and shown in Help ▸ About | Phase 14 |
| D22 | Versioning after 1.0 | SemVer for the application: minor versions add features and open every 1.x file; the plugin and script APIs may grow in a minor version and break only in a major one | Phase 14 |
| D23 | Crash reports | Written locally into the data folder and pointed to on the next start (beside the autosave offer); never sent anywhere | Phase 14 |
| R4 | Unsigned packages | Windows SmartScreen and macOS Gatekeeper warn about unsigned apps; the README explains how to open them; signing and notarisation after 1.0 | Phase 14 |
| D24 | The file format after 1.0 | A release that adds document data brings one new `.g4d` version (v4 in 1.1) with a migration from the previous one, its own golden fixture and its section in `FILE_FORMAT.md`; older versions open forever; a version is frozen when its release ships | Phase 15 |
| D25 | Decoding images | The `image` crate limited to PNG, JPEG, TIFF, WebP, HDR and EXR (permissive, no C); colour space per texture (sRGB for colour, linear for data); linked by relative path or embedded in the `.g4d` | Phase 15 |
| D26 | The material model | One fixed, principled set of channels shared by the viewport, the path tracer and interchange; Phase 22's node graphs compile into it rather than replacing it | Phase 15 |
| D27 | Denoising | A guided denoiser of our own first (albedo and normal passes, pure Rust); an optional Open Image Denoise build later, if its C++ library fits the packaging and licence checks | Phase 16 |
| D28 | GPU rendering | wgpu compute, never a vendor API; the CPU path tracer stays the reference (tests compare against it) and the fallback | Phase 23 |
| D29 | Formats beyond glTF / OBJ | Pure-Rust readers and writers first; a C dependency only when no other way exists and it passes `cargo deny` and packaging on all three systems | Phase 24 |
| D30 | Python | Optional, beside Rhai (which stays the sandboxed default); the same `SceneEdit`-based API, one undo step per run | Phase 24 |
| R5 | Scope against capacity | The suites Grid4D is compared with have large teams. Ship one phase per minor release in the order above; a phase may be cut, never the rules (undo, tests, docs, the frame budget) | ongoing |
| R6 | Robust mesh booleans | Coplanar and degenerate cases break naive booleans; Phase 17 starts with a spike on exact predicates and a fuzzed test corpus before any interface | Phase 17 |
| R7 | GPU compute portability | Shader behaviour differs between Vulkan, Metal and DX12; CI renders the reference scenes on every backend it has and compares with the CPU tracer | Phase 23 |
| R8 | Feature names | Other products' feature names are often trademarks; every new feature gets a plain, descriptive name of our own (reviewed with R2 at each release) | ongoing |

---

**Next action:** the `v1.0.0` tag (the release workflow builds the packages
and a draft release); then Phase 15 (materials, UVs and textures, v1.1),
starting with its specs and the `.g4d` v4 format.
