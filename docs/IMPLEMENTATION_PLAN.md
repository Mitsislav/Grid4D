# Grid4D — Implementation Plan

> Status of this document: **v1, 2026-10-18.** Written after a full read of the
> v0.1 codebase (≈ 6.1 k lines, 27 tests). It turns the short roadmap in
> `ARCHITECTURE.md` into ordered, PR-sized work with design decisions, acceptance
> criteria and the files each step touches. Update it when a phase lands.
>
> **2026-10-19: the project was renamed from OpenC4D to Grid4D** — crates
> `oc4d-*` → `grid4d-*`, binary `grid4d`, documents `.oc4d` → `.g4d` (old files
> still open), original app icon in `assets/icon/`. Older entries below use the
> new names.

All rules in `ARCHITECTURE.md` still apply to every step below — dependency direction,
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
13. [Phase 9+ — Splines, generators, deformers, MoGraph, extensibility](#phase-9--splines-generators-deformers-mograph-extensibility)
14. [Cross-cutting: testing, performance, CI, docs](#cross-cutting-testing-performance-ci-docs)
15. [Risks & open decisions](#risks--open-decisions)

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
          ▲     ▲               ▲     (io: .g4d, glTF, OBJ — serde lives here, not in scene)
grid4d-render  grid4d-raytrace  │     (raytrace: Phase 8)
          ▲     ▲               │
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
| `grid4d-ui` | exists | all of the above, `egui`, `rfd` | — |

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
| v1.0 | Stabilisation | — | File format frozen (with migrations), docs, packaging, perf budget met |

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
| 0.2 | S | `git init`, first commit of v0.1, add `LICENSE-MIT` + `LICENSE-APACHE` (README already proposes the Rust default), `CONTRIBUTING.md` (points to `ARCHITECTURE.md` rules). |
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
directly for live feedback (ARCHITECTURE.md rule), but commit an undo step **on
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
| 1.5 ✅ | S | Toolbar Undo/Redo enabled state + tooltip with label ("Undo Move"); Edit menu entries; shortcuts `Ctrl+Z`, `Ctrl+Y`, `Ctrl+Shift+Z`; history limit preference (default 100) — done 2026-10-18; shortcuts are ignored while a mouse button is held (a drag commits its own step). The limit is `History::DEFAULT_LIMIT` = 100; its UI waits for the preferences window (Phase 9) | `ui/panels/{toolbar,menu_bar,about}.rs`, `ui/shortcuts.rs` |

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
| 3.4 | S | ~~`GeometryHandle`-keyed batches + `GpuMeshCache`~~ — done early in 3.2 for primitives (`MeshKey`); what remains is a key for editable meshes (mesh `Arc` + revision) once 3.6 adds them |
| 3.5 ✅ | M | Offscreen viewport target (colour/depth/MSAA, resize handling) — done 2026-10-19: `grid4d_render::ViewportTarget` (`Rgba8Unorm` gamma colour because egui samples textures as gamma, 4× MSAA when supported, `Depth32Float`, transparent clear over the UI's gradient), `ViewportRenderer::render` submits its own pass, `ui/viewport/gpu_view.rs` registers and re-points the egui texture on resize; the paint callback, `Arc<Mutex<FrameData>>` and the window's depth/MSAA are gone. Checked on screen, maximized and resized |
| 3.6 | M | `ObjectKind::Polygon`, Make Editable (`C`), BVH picking, `.g4d` **v3** (zip + binary geometry) with migration (v2 was taken by segment counts); v3 may also change the `"format"` tag from `"oc4d"` to `"grid4d"` |

**Acceptance:** 10 000 cubes and a 1 M-triangle mesh render at interactive rates
(see performance budget); editing a primitive's segments updates instantly;
Make Editable + undo round-trips; steady-state frames allocate nothing in
eval/extract/prepare/paint.

---

## Phase 4 — Polygon modelling & Subdivision Surface (v0.4)

**Goal:** the C4D component workflow on polygon objects.

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
| 4.1 | M | Component modes, display, picking of points/edges/polys |
| 4.2 | M | Live / Rectangle selection, Loop / Ring, Grow / Shrink / Invert / Connected |
| 4.3 | M | Component transform via gizmo + undo |
| 4.4 | M | Extrude, Inner Extrude (interactive drag + numeric) |
| 4.5 | M | Loop/Path Cut, Knife (line), Connect points/edges, Dissolve, Delete |
| 4.6 | M | Bevel (edges/points, offset + subdivision), Bridge, Weld, Optimize |
| 4.7 | L | Generator framework + Subdivision Surface generator |
| 4.8 | S | Snapping (grid, point, edge, polygon centre) — gives `EditorState::snapping` its behaviour |

Shortcuts follow Cinema 4D where one exists (e.g. `C` Make Editable, `D` Extrude,
`I` Inner Extrude, `K~L` Loop/Path Cut, `M~S` Bevel, `U~L` Loop Selection) —
verify each against Maxon's public documentation before binding; two-key chords need
a small chord state machine in `shortcuts.rs`.

**Acceptance:** model a simple object (box → extrude → loop cuts → bevel → SubD)
entirely with tools, undo each step, save and reload it.

---

## Phase 5 — Interchange (glTF, OBJ) (v0.5)

| Step | Size | Work |
|------|------|------|
| 5.1 | M | glTF 2.0 import (`gltf` crate): hierarchy, TRS → HPB, meshes → `Polygon`, cameras, `KHR_lights_punctual`; unit conversion (glTF metres → cm ×100) and Y-up already match |
| 5.2 | M | glTF 2.0 export (`.gltf`/`.glb`): primitives exported via their generated meshes; generators exported evaluated |
| 5.3 | S | OBJ import/export (`tobj` for import; writer by hand) |
| 5.4 | S | Later in Phase 6: materials (PBR metallic-roughness) and animation tracks in glTF |

FBX needs a proprietary SDK → out of scope. USD is attractive but its Rust
ecosystem is immature → revisit after v1.0.

**Acceptance:** glTF sample models (Khronos sample assets: Box, Duck, …) import
correctly; export → re-import round-trips hierarchy, transforms and geometry.

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
| 6.1 | M | Tags data + UI |
| 6.2 | M | Tracks/curves/keys + evaluation + tests (interpolation, tangent continuity) |
| 6.3 | M | Record Active Objects (`F9`), Autokeying, key indicators in the Attribute Manager, keys drawn on the timeline (select, move, delete) |
| 6.4 | L | New panel `fcurve.rs` — F-Curve editor (pan/zoom, tangents, interpolation menu); dock as a tab beside the timeline |
| 6.5 | M | New panel `coordinates.rs` — Coordinate Manager (P/S/R, Object/World/Size modes, Apply) |
| 6.6 | M | Materials: `Material { name, base_color, roughness, metallic, emission, … }` in the document; new panel `materials.rs` (Material Manager, thumbnails rendered later); Material tag; viewport uses base colour (then PBR in Phase 7) |

**Acceptance:** keyframe a bouncing ball (position + scale), edit curves, play
back at document fps, save/reload, export to glTF with animation.

---

## Phase 7 — Viewport: cameras, lights, 4-view, shading (v0.7)

| Step | Size | Work |
|------|------|------|
| 7.1 | M | Scene cameras: Cameras ▸ use scene camera, look-through, camera `fov` used; camera frustum drawn in the viewport |
| 7.2 | M | 4-view layout (Perspective, Top, Front, Right) with orthographic cameras; Panel menu arrangements; each view its own offscreen target and settings (from Phase 3C) |
| 7.3 | M | Display modes: Gouraud, Gouraud + Lines, Quick Shading, Wireframe, Box; per-view |
| 7.4 | M | Light objects affect the viewport (storage buffer of N lights; "default light" when the scene has none); PBR-lite shading from materials |
| 7.5 | S | Filter menu (hide object types, grid, HUD per view) |
| 7.6 | M | Optional: shadow map for the key light, SSAO — behind a per-view toggle |

Hot-path rules apply to every new per-frame buffer (light buffer, per-view
uniforms): grow geometrically, size tests next to each `#[repr(C)]` struct.

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

---

## Phase 9+ — Splines, generators, deformers, MoGraph, extensibility

Lower resolution on purpose — plan in detail when v0.8 lands.

* **Splines** (`grid4d-geom::spline`): Linear, Cubic, Bezier; primitives Circle,
  Rectangle, Helix, Text (font outlines via `ttf-parser`); Pen tool.
* **Spline generators:** Extrude, Lathe, Loft, Sweep (all via the Phase 4
  generator framework).
* **Deformers:** Bend, Twist, Taper, Bulge, FFD — evaluated after generators.
* **MoGraph-lite:** Cloner (linear/radial/grid/object), Random and Plain effectors.
* **Simulation:** rigid bodies via `rapier3d` (menu already lists Simulate).
* **Workspace:** Takes, Layers, customisable docking layouts (e.g. `egui_dock`),
  preferences window, theme variants, i18n.
* **Extensibility:** a statically-registered Rust plugin trait for object/tag/tool
  types first; scripting (Rhai or Python via `pyo3`) after the API stabilises.

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
(1.95) · `cargo deny` (licenses, advisories) · later: fuzzing the `.g4d` parser
(`cargo fuzz`, nightly, scheduled) and release builds via `cargo-dist`.

### Documentation

* Keep `ARCHITECTURE.md` as the rules file; update its workspace tree and roadmap when a
  phase adds crates/panels.
* `docs/ARCHITECTURE.md` (diagram from §2, data flow per frame),
  `docs/FILE_FORMAT.md` (versioned `.g4d` spec), user-facing shortcut list
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
| R3 | Scope creep | Ship the release train in order; each phase must leave the app usable and tests green | ongoing |

---

**Next action:** Phase 0 (toolchain, git, CI, A1–A5), then Phase 1 step 1.1
(`NodeData` split + soft delete) — the foundation every later phase builds on.
