# Grid4D — Progress log

What was built in each step, and the problems we met along the way and how we
solved them. Newest steps at the bottom of each phase.

- **Where we are going**: [`IMPLEMENTATION_PLAN.md`](IMPLEMENTATION_PLAN.md). The
  plan's step tables carry the detailed technical notes.
- **How to use what exists**: [`manual.md`](manual.md).
- **Every detail of a step**: its commit message (`git log`).

## Status

| | |
|-|-|
| Current phase | 4 — Polygon modelling & Subdivision Surface (v0.4) |
| Last step done | 4.6 — Bevel, Bridge, Weld, Optimize (2026-10-20) |
| Next step | 4.7 — Generator framework + Subdivision Surface |
| Tests | 308, all passing; clippy and rustfmt clean |

---

## Baseline — v0.1 editor shell (2026-10-18)

The starting point, `64d00bf`:

- the Standard layout (menu bar, palettes, viewport, Object and Attribute
  Managers, timeline, status bar);
- a scene graph of primitives, nulls, cameras and lights;
- a wgpu viewport with a grid;
- object selection, and Move/Scale/Rotate gizmos.

## Phase 0 — Foundations (2026-10-18)

**0.1–0.7** (`2a192a8`)

*Built:*
- dual MIT/Apache licence, CONTRIBUTING, LF line endings everywhere;
- GitHub Actions CI (rustfmt, clippy, tests on Windows/Linux/macOS, MSRV 1.95);
- the first unit tests for commands and selection (38 tests).

*Problems and solutions:*
- **Selection lookups were linear on the render path.** Every frame asked "is this
  node selected?" for every node, which made Select All quadratic. The selection now
  keeps an O(1) membership index next to its ordered list.
- **The gizmo geometry was built twice per frame.** It is now built once and shared
  by interaction and drawing.
- **`_ =>` match arms hid new variants.** They were replaced by exhaustive matches,
  so adding a tool or object type points the compiler at every place to update.

## Phase 1 — Undo/redo (2026-10-18)

| Step | Built | Tests |
|------|-------|-------|
| 1.1 (`821bd61`) | Node data split from hierarchy links; soft delete (`detach`/`attach`); ordered moves | 46 |
| 1.2 (`45eaa0d`) | `SceneEdit` + `Document::apply` returning the exact inverse; atomic batches | 56 |
| 1.3 (`0a5730c`) | `History` with undo groups; create, delete and re-parent as undo steps | 66 |
| 1.4 (`b6fe0fa`) | Undo for live edits: gizmo drags, attribute fields, settings, Object Manager | 76 |
| 1.5 (`082fe45`) | Undo/redo buttons, Edit menu, Ctrl+Z / Ctrl+Y | 78 |

*Problems and solutions:*
- **Undoing a delete had to bring back the very same objects** (same IDs, so later
  steps still refer to them). Deleting became a *soft* delete: the nodes stay stored
  but invisible, and undo re-attaches them. They are purged only when no step can
  restore them any more.
- **Fields and gizmos write the scene every frame** while you drag, but one drag must
  be one undo step. The `LiveEdit` tracker records the value at the start of an
  interaction (mouse held or text focused) and commits a single step at the end.
- **Ctrl+Z in the middle of a drag** would have undone underneath a live edit. Undo
  shortcuts are ignored while a mouse button is held.

## Phase 2 — Files (2026-10-18)

| Step | Built | Tests |
|------|-------|-------|
| 2.1 (`bc9e68e`) | Versioned document format (JSON, then a zip in v3) with golden test files | 90 |
| 2.2 (`77164ec`) | Open / Save / Save As with native dialogs; "Save changes?" prompts | 104 |
| 2.3 (`02e1a85`) | Recent files; window and panel layout remembered | 110 |
| 2.4 (`9b90a30`) | Open a document by dropping it on the window | 113 |

*Problems and solutions:*
- **The file format must never change by accident.** Every format version has a
  golden fixture file that the tests compare exactly. Released versions are
  frozen; a change means a new version with a migration.
- **A crash during saving could destroy the document.** Saving writes a temporary
  file and renames it over the old one.
- **JSON cannot store NaN or infinity.** Non-finite numbers are rejected when saving
  and when loading.
- **"Unsaved changes" had to survive undo.** A document counts as changed when the
  history state differs from the state at the last save, so undoing back to it is
  clean again.
- **A stray control character (BEL) slipped into ARCHITECTURE.md.** Every commit is now
  scanned for CR and control bytes first.

## Phase 3 — Geometry core and viewport (2026-10-18 – 19)

| Step | Built | Tests |
|------|-------|-------|
| 3.1 (`54cd6c7`) | `grid4d-geom`: n-gon meshes, topology, normals, triangulation | 134 |
| 3.2 (`903b846`) | Parametric primitives with segments (+ cone, torus, disc, capsule), instanced drawing | 150 |
| — (`cb64445`) | Start maximized; never open a window larger than the screen | 150 |
| 3.3 (`2fa364a`) | `grid4d-eval`: evaluated scene with revisions; render and picking read it | 164 |
| 3.5 (`07809ef`) | The viewport renders into its own offscreen target (MSAA, depth) | 166 |
| — (`2532c1a`) | Renamed OpenC4D → Grid4D; original app icon; `.g4d` files | 168 |
| 3.6 (`287fdfb`) | Polygon objects, Make Editable, BVH picking, file format v3 (zip) | 194 |

*Problems and solutions:*
- **The window did not fit a 1080p screen at 125 % scaling.** The title bar pushed the
  timeline behind the taskbar. Grid4D now starts maximized, and a restored size is
  clamped to the screen.
- **Idle frames must cost nothing.** Every scene change bumps a revision, and
  evaluation is skipped while it is unchanged. The Attribute Manager used to write
  the scene every frame, which bumped revisions for nothing. It now writes only real
  changes, and a test renders every panel for idle frames and fails if the revision
  moves.
- **Colours came out wrong in the offscreen target**, because egui samples textures
  as gamma-encoded. The target is `Rgba8Unorm` and the shaders encode sRGB
  themselves.
- **The rename must not break existing files.** Old `.oc4d` documents still open;
  the format tag inside released versions stays as it was.
- **The icon had to be original**: a sphere on a perspective grid, deliberately
  unlike Maxon's sphere-in-a-cube logo.
- **Picking huge meshes**: a BVH makes a pick on a million triangles take under a
  microsecond after the first build.

## Phase 4 — Polygon modelling (2026-10-19 – )

| Step | Built | Tests |
|------|-------|-------|
| 4.1 (`4133bb3`) | Points/Edges/Polygons modes: display, picking and selection of components | 218 |
| 4.2 (`e428034`) | Live, Rectangle, Loop and Ring selection; the Select menu commands | 240 |
| 4.3 (`15aba27`) | The gizmo moves, scales and rotates components; Tweak mode | 246 |
| 4.4 (`93d0ffb`) | Extrude and Inset (drag or Apply); fixes from testing the app | 269 |
| 4.5 (`781e547`) | Delete, Melt, Dissolve, Connect, Loop/Path Cut, Line Cut; two-key shortcuts | 292 |
| docs (`163ac7c`) | The user manual and this progress log; the shortcuts dialog fits small windows again | 292 |
| 4.6 | Bevel (edges and points, rounded profile), Bridge (polygons, border edges), Weld, Optimize (dialog) | 308 |

Before each step we write a behaviour spec in [`reference/`](reference/README.md) from
Maxon's public manual, in our own words (clean-room).

*Problems and solutions:*
- **Selections must survive mesh edits.**
  - Selecting never creates a new mesh: selected and hidden components are stored
    next to the mesh, not in it.
  - Extrude and Inset keep every existing index and append new ones.
  - Operations that remove or split components (4.5) return the mesh together with
    the origin of each point and polygon, and selections are carried across through
    it.
- **Dragging components creates a new mesh every frame.** Old meshes would have piled
  up on the GPU, so the renderer frees a polygon object's mesh as soon as a frame no
  longer draws it.
- **Shortcuts must match Cinema 4D.** We bind a key only after checking it against
  Maxon's documentation. The official shortcut list confirmed `0`, `U~L`, `K~L`,
  `M~M` and others. The two-key chords needed a small state machine, with the pending
  key shown in the status bar.
- **Extrude on a piece that touches itself at one point** produced a non-manifold
  wall: four walls met on one edge. Points on the outline now get one copy per time
  the outline passes through them.
- **Testing the real application.** We drive the app with posted Windows messages
  and capture the window.
  - Clicks and keys are reliable, but drags are not: Windows reports the pointer as
    "left" between posted moves.
  - So drags are verified with headless egui tests that run the real panels; on
    screen we use clicks, keys and the Apply buttons.

*Bugs found by testing the app on screen, and fixed:*
- **4.4 — A new camera filled the viewport with grey.** It is created at the view,
  and its marker cube enclosed the eye. Markers are no longer drawn while the eye is
  inside them.
- **4.4 — Hidden polygons were still drawn shaded.** The viewport now draws (and
  picks) a cached copy of the mesh without them, rebuilt only when the hidden set
  changes.
- **4.5 — A Loop/Path Cut click selected the edge instead of cutting** when the
  pointer left the view in the same frame. The cut now uses the click position, like
  picking does.
- **4.5 (while writing the manual) — the shortcuts dialog no longer fit a 1280 × 800
  window**, and "⌫" showed as an empty box (the font has no such glyph). The list now
  scrolls inside the dialog, and the key is written "Backspace".
- **Open question settled (4.5):** Delete in Edges mode removes the edges' polygons,
  as the manual says. Melt and Dissolve keep them.

*Step 4.6 — problems and solutions:*
- **Filling the holes a bevel leaves** at corners, at the ends of a bevelled edge
  inside a surface and at the border needed one general rule rather than a case per
  shape. After the strips are built, the edges among the new points of each replaced
  point that no polygon has from the other side form the hole's outline. Each closed
  outline becomes one polygon; an open one (at the border) needs none.
- **A rounded bevel ending at a flat side** leaves a thin region between the curved
  end and the side's straight edge. It becomes its own polygon, in the side's plane,
  so it looks seamless (merging it into the side is a possible refinement).
- **Bridging polygons whose points are already joined** (the top and bottom of a
  1-segment cube) puts the new walls on the existing vertical edges, four polygons per
  edge. The spec's example was wrong and now uses the middle polygons of a segmented
  cube.
- **With "Delete Original Polygons" off**, the band must run the other way round,
  because it sits next to the two polygons instead of replacing them. A test caught
  this bug.
- **Optimize's "Polygons" option** (removing one- and two-point polygons) is always on
  in Grid4D, which cannot store such polygons.
- **On-screen testing**: Shift cannot be sent as posted input (winit reads the real
  keyboard state), and posted mouse-wheel scrolling was unreliable this time. The
  tools' results were therefore checked by opening documents built with the real
  algorithms, and the interaction by the headless tests, which run the real viewport
  code.

---

## How this file is kept

After every step (and every fix worth noting) this file gets the new step in its
phase's table, its problems and solutions, and an updated Status block. The manual is
updated in the same change.
