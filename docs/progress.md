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
| Current phase | 5 — Interchange: glTF, OBJ (v0.5) |
| Last step done | 5.1 — glTF 2.0 import, File ▸ Merge (2026-10-20) |
| Next step | 5.2 — glTF 2.0 export |
| Tests | 354, all passing; clippy and rustfmt clean |

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
| 4.6 (`c73d732`) | Bevel (edges and points, rounded profile), Bridge (polygons, border edges), Weld, Optimize (dialog) | 308 |
| 4.7 (`d0047fc`) | Generators; the Subdivision Surface (Catmull-Clark on n-gons, Linear Pre-Subdivision, border rules); Alt-create wraps the selection, Q switches, Make Editable bakes | 328 |
| 4.8 (`babca6d`) | Snapping (Shift+S): vertex, edge, edge mid point, polygon centre, surface, axis, grid point and line; 2D/3D; the Move handles and Tweak; highlight with the target's name; settings in the palette menu, Tools ▸ Snap and the Move tool. Phase 4 acceptance test: box → extrude → loop cut → bevel → Subdivision Surface, each step undone and redone, saved and reloaded | 342 |

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

*Step 4.7 — problems and solutions:*
- **Where a generator's result lives.** The scene stays plain data: a generator is an
  object kind with parameters and nothing else. The evaluation layer builds the result
  and keeps it per generator. It rebuilds only when an input changes: the cage's
  meshes (by id), primitive parameters, their placement relative to the generator,
  or the options. Idle frames, moving the generator itself and selecting cage
  components rebuild nothing.
- **The cage is hidden but must stay editable.** Its objects stay in the evaluated
  scene (component modes and framing use them) but are neither drawn nor picked.
  In component modes the cage's points and edges are drawn over the smooth surface.
- **A disabled generator** acts as a plain group, so its cage shows as it is; a
  hidden one hides everything below it.
- **The Make Editable button asks every frame** whether the selection can be made
  editable. A first version answered by baking the meshes, which would have
  subdivided every frame. The check is cheap again; only the command bakes.
- **Make Editable with the cage selected as well** would have converted the cage and
  then removed it. Objects inside a cage that is being removed are now skipped.
- **Alt-create is two structural edits** (insert the generator where the object was,
  move the object under it). They are recorded as a single undo step.
- **Defaults the reference does not document**: 2 viewport and 3 render levels and
  the *Edge and Corner* border rule are our choice, written down in the spec. `Q`
  was verified in Maxon's shortcut list before binding it.
- **Screenshots.** The Attribute Manager is too short at 1280 × 800 for all the
  parameters, so the window was maximised for one capture. The app remembers the
  window state, so it was restored afterwards. Reducing the large smooth gradients
  to a 256-colour palette produced bands, so the manual's images are now dithered.

*Step 4.8 — problems and solutions:* (see also the bugs list above)
- **Where to look for targets.** The first version looked around the position the
  moved element would have without snapping. On screen the result was confusing: the
  target was highlighted somewhere between the gizmo and the cursor. The reference
  describes the snap radius as an area around the *cursor*, so targets are now
  looked for under the cursor: you point at what to snap to, and the element lines
  up with it along the dragged axis.
- **Precision of the ray under the cursor.** Inverting the camera matrix in single
  precision (near plane 2 cm, far plane 2 km) put a 2D snap 0.03 px off. The
  ray is now computed in double precision.
- **A layout bug found while taking screenshots.** A long hint in the Attribute
  Manager's Tool mode does not wrap inside its grid, so it widened the whole right
  panel. The app remembers panel sizes, so the wider panel was saved. The hint now
  wraps, and the panel was dragged back to its old size.
- **Screen coordinates.** The test machine runs at 125 % display scaling. egui works in
  points (pixels ÷ 1.25), which at first made the highlight look misplaced in the
  captures. A temporary log of the cursor and target positions showed the snap was
  correct.
- **On-screen testing.** Snapping needs real drags. Posted mouse messages are not
  enough, so these captures move the real cursor over the window for a moment
  (and put it back). Posted right-clicks do not open menus at all, so the snap
  button's menu is checked by a headless test instead.

## Phase 5 — Interchange (2026-10-20 – )

| Step | Built | Tests |
|------|-------|-------|
| 5.1 | glTF 2.0 import (`.gltf` + `.bin`, embedded buffers, `.glb`): hierarchy, transforms, meshes (welded, with UVs), base colours, cameras, lights; Open makes a new document, File ▸ Merge (Ctrl+Shift+O) and drag and drop add to the current one (one undo step); Merge also takes `.g4d` files | 354 |

*Problems and solutions:*
- **The `gltf` crate's default features decode textures** (they pull in an image
  library). We use it only to parse the file. Buffers are loaded by our own code:
  the `.glb` binary chunk, `data:` URIs (a small base64 decoder) and files next to
  the glTF, by relative path only, so a file cannot make Grid4D read anything else.
- **Exporters split vertices** at every normal or UV seam, so a glTF cube has 24
  vertices. Imported as they are, the faces would not be connected and modelling
  tools or the Subdivision Surface would tear them apart (the reference's manual
  suggests running Optimize afterwards). Identical positions are now merged on
  import; the UVs live per corner, so the seams survive.
- **Merging must be one undo step**, but each new object needs its parent's id,
  which exists only after the parent is inserted. The objects are inserted one by one
  and the inverses recorded as a single step. If one insert fails, the earlier ones
  are taken back.
- **A broken file must never hang or crash the app.** Node graphs with cycles are cut
  (each node read once). Short buffers, out-of-range indices, non-finite numbers
  and absolute or remote URIs are errors, shown in an alert. Tests feed such files.
- **Display scaling bit us again on screen.** An earlier panel resize had left the
  right panel 10 px narrower; it was dragged back to its original width.

---

## How this file is kept

After every step (and every fix worth noting) this file gets the new step in its
phase's table, its problems and solutions, and an updated Status block. The manual is
updated in the same change.
