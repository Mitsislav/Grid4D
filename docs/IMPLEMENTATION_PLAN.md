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
13. [Phase 9 — Splines & spline generators](#phase-9--splines--spline-generators-v09)
14. [Phase 10+ — Deformers, MoGraph, simulation, workspace, extensibility](#phase-10--deformers-mograph-simulation-workspace-extensibility)
15. [Cross-cutting: testing, performance, CI, docs](#cross-cutting-testing-performance-ci-docs)
16. [Risks & open decisions](#risks--open-decisions)

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
| v0.9 | Splines | 9 | Spline objects and primitives, point editing and the Pen, Extrude / Lathe / Sweep / Loft, Text |
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
| 9.6 | M | Sweep generator: a profile spline along a path spline — end scale and rotation, start/end growth, banking, caps; a parallel-transport frame along the path |
| 9.7 | M | Loft generator: a skin through child splines in order — subdivisions along and across, loop, linear or organic; caps |
| 9.8 | M | Text spline: font outlines via `ttf-parser` and a bundled open (OFL) font; text, height, alignment, spacing; segments with holes, so Extrude makes solid letters |

**Acceptance:** a Circle, Rectangle or Text spline extruded into a capped solid,
a profile lathed into a vase and swept along a helix, two circles lofted into a
tube; splines edited point by point and drawn with the Pen; everything undoable
and saved, and the generators' results rendered by the path tracer.

---

## Phase 10+ — Deformers, MoGraph, simulation, workspace, extensibility

Lower resolution on purpose — plan in detail when v0.9 lands.

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
