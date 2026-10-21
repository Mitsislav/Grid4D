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
| Current phase | 8 — Offline renderer (v0.8) |
| Last step done | 8.1 — The path tracer core (2026-10-21) |
| Next step | 8.2 — Render Settings in the document |
| Tests | 485, all passing; clippy and rustfmt clean |

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
| 5.1 (`02b7d94`) | glTF 2.0 import (`.gltf` + `.bin`, embedded buffers, `.glb`): hierarchy, transforms, meshes (welded, with UVs), base colours, cameras, lights; Open makes a new document, File ▸ Merge (Ctrl+Shift+O) and drag and drop add to the current one (one undo step); Merge also takes `.g4d` files | 354 |
| 5.2 (`e54fbdb`) | glTF 2.0 export (File ▸ Export…, `.glb` or `.gltf` + `.bin`): hierarchy, transforms, triangulated meshes with the viewport's normals and UVs, primitives and Subdivision Surfaces baked, colours as materials, cameras, lights; checked in the app by exporting, validating, reopening and merging | 362 |
| 5.3 (`f6c67ab`) | Wavefront OBJ import and export: polygons keep their corners both ways, per-corner UVs (V flipped), colours through `.mtl`, objects split by `o`/`g`; Open, Merge, drop and Export accept `.obj`; exported generators take their cage's colour | 368 |

*Problems and solutions:*
- **The `gltf` crate's default features decode textures** (they pull in an image
  library). We use it only to parse the file. Buffers are loaded by our own code:
  the `.glb` binary chunk, `data:` URIs (a small base64 decoder) and files next to
  the glTF, by relative path only, so a file cannot make Grid4D read anything else.
- **Exporters split vertices** at every normal or UV seam, so a glTF cube has 24
  vertices. Objects as they are, the faces would not be connected and modelling
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

*Step 5.2 — problems and solutions:*
- **Exporting needs baked geometry, but `grid4d-io` must not depend on evaluation.**
  The UI first bakes the document into an object list (`file::exportable`):
  primitives become their meshes and generators their results without the cage.
  `grid4d-io` then only writes polygon objects, cameras, lights and empty nodes.
  The importer's object list (renamed `Objects`) serves both directions.
- **glTF forbids empty arrays and a scene needs nodes.** An empty document is
  written without them, and the tests check that.
- **Testing in the app.** The demo with a Subdivision Surface was exported through
  File ▸ Export (both `.glb` and `.gltf` with a space in its name). Each file was
  checked by a separate Python script: GLB layout, buffer bounds, min/max,
  indices, unit normals, the hierarchy. The file was then opened again and merged
  into the original: the smoothed object came back with exactly its 98 points and
  192 triangles. The imported table scene, exported again, keeps every node's
  transform to within 5·10⁻⁸.
- **That round trip found a bug in the 5.1 importer.** Welding compared positions
  bit by bit, so `0.0` and `-0.0` counted as different points. A sphere's pole,
  written with zeros of both signs, was only partly merged and kept degenerate
  triangles. Zeros are now normalised before comparing (a test covers it).

*Step 5.3 — problems and solutions:*
- **OBJ has no transforms or hierarchy.** Objects are written with their points in
  world space (from the object list's parents). An object mirrored by a negative
  scale has its polygons written in reverse order, so they still face outwards.
- **The `tobj` reader invents texture coordinates.** For faces without `vt`, in a
  file that has some elsewhere, it repeats one stand-in value. A constant UV map
  carries nothing, so it is dropped on import.
- **The reader numbers points in the order faces use them**, not as in the file.
  The round-trip tests therefore compare point sets, not indices.
- **Testing in the app found an inconsistency in both exporters.** A Subdivision
  Surface was written in its own display colour, while the viewport draws its
  result in the cage's. Exports now use the cage's colour.
- **Checks in the app:**
  - the demo with a Subdivision Surface, exported to OBJ through File ▸ Export,
    passed an independent Python check (references in range, unit normals, every
    material defined);
  - it kept its polygons: 96 quads, and two 36-sided caps on the cylinder;
  - opened again, the smoothed object came back with 98 points and 96 polygons;
  - merged into the original, every object lay on its original;
  - the imported table scene (hierarchy, rotations), exported to OBJ, matched
    world-space positions computed straight from the glTF to 3·10⁻⁶ cm.

## Phase 6 — Tags, animation, materials, managers (2026-10-20 – )

| Step | Built | Tests |
|------|-------|-------|
| 6.1 (`becd43a`) | Tags on objects (saved, undoable): Phong (smoothing angle, in the viewport and in exports), Protection (no gizmo, read-only Coordinates for protected channels), Target (turns the object's −Z towards another object, children follow); icons in the Object Manager, the tag view in the Attribute Manager, Tags menu, Delete removes a selected tag | 378 |
| 6.2 (`2e1086a`) | Animation data: tracks of keys per object (position, rotation, scale), Spline / Linear / Step with clamped auto or manual tangents, evaluated at the current frame (children follow, the document untouched), saved in `.g4d` | 386 |
| 6.3 (`09acb58`) | Recording keys: Record Active Objects (F9), Autokeying (Ctrl+F9, red view frame), key buttons in the Coord. tab, temporary (unrecorded) changes, the timeline's key strip (select, move, delete, Ctrl+click records), previous / next key (Ctrl+F / Ctrl+G); the gizmo and the Coordinates work at the current frame | 403 |
| 6.4 (`4724314`) | The F-Curve editor, a tab beside the Timeline (Tab switches): track list, curves with keys, selecting (click, Ctrl, box, curve), moving keys (Shift constrains), tangent handles (Shift breaks), Ctrl+click adds a key, interpolation and tangent presets, zoom, pan, H / S framing, the selected key's frame and value | 415 |
| 6.5 (`dfdc469`) | The Coordinate Manager, a tab beside the Attribute Manager (Shift+F7 / Shift+F5): position, scale / size / size+ and rotation of the active object in object or world space, or of the selected points, edges or polygons (centre, size, relative rotation); Apply | 421 |
| 6.6 (`4745867`) | Materials: the Material Manager (a bottom tab, Shift+F2), materials with colour, roughness, metallic and luminance, Material tags (drag a material onto an object, Apply), inheritance by children, the viewport in the material's colour, saved in `.g4d` and carried by glTF and OBJ; Merge keeps materials and object references | 434 |

*Problems and solutions:*
- **Where tags live.** Tags sit in `NodeData`, the unit of attribute edits. Adding,
  editing and removing them are therefore ordinary data edits: undo and saving
  needed no new machinery.
- **A per-object Phong angle with cached GPU meshes.** Meshes are cached on the GPU
  by their key, and before 6.1 the smoothing was the same for everything. The
  cache key is now the mesh key plus the shading angle (kept as bits, so it can be
  hashed). Two cubes with the default angle still share one instanced mesh; a
  flat-shaded one gets its own.
- **Target without changing the document.** The tag is evaluated: the evaluated
  world matrix of the tagged object is replaced, and a correction matrix is passed
  down its subtree so children follow. The document's rotation values stay as the
  user set them. Targets that are hidden, missing, the object itself or inside it
  are ignored.
- **Tags refer to other objects.** In the file they are stored by object index:
  every object is indexed first, then written. On load the references are resolved
  once all objects exist, and an index that names nothing is dropped.
- **Two deliberate differences from the reference**, written in the spec and the
  manual:
  - objects without a Phong tag keep Grid4D's 80° smoothing, so existing
    documents look unchanged;
  - the Target tag aims −Z, the axis Grid4D's cameras look along.
- **Checked in the app:**
  - the tag icons appear;
  - a click on a tag shows its settings, and clicking the name shows the object
    again;
  - a tag added from the Tags menu is selected, and Delete removes only the tag;
  - a sphere tagged Phong 0° is flat-shaded;
  - a protected box has no Move gizmo and grey Coordinates.

*Step 6.2 — problems and solutions:*
- **Tracks belong to their object.** The plan sketched one list of tracks naming
  their nodes. Storing them in the object's data instead keeps the animation with
  its object through deletes, undo, merges and saving, with no extra bookkeeping.
- **Playback must not touch the document.** The evaluation computes animated world
  matrices in pre-order, so children follow, and leaves the object's own values
  alone. It re-evaluates on a frame change only if the last evaluation found
  animation; a static scene costs nothing during playback.
- **Spline segments as functions of time.** A Bézier segment is a curve in
  (frame, value). Its parameter is found by bisection on the frame, and the
  handles are kept within their segment so the frame always increases.
  - Auto tangents use the slope through the neighbouring keys, with handles a third
    of the way to each neighbour, so the curve is smooth through every key.
  - Clamping flattens peaks, dips and equal neighbours, so a bouncing ball never
    goes through the floor. Tests check both.
- **Checked in the app:** a document with a bouncing, squashing ball and a turning
  bar carrying a sphere, stepped with G to frames 8, 15 and 30. Every object was
  where its curves put it, and the child turned with its parent.

*Step 6.3 — problems and solutions:*
- **Editing an animated object while playback never writes the document.** Before
  6.3 the gizmo and the Coordinates fields worked on the object's own values, which
  an animated parameter ignores, so a drag moved nothing. Now everything the user
  sees and edits is the transform *at the current frame*, and one function writes
  an edit back: a key with Autokeying, otherwise a **temporary value**.
- **Where a temporary value lives.** The reference keeps a changed but unrecorded
  value until the time moves. Grid4D keeps it in the object's base transform (unused
  by an animated parameter) with the frame it belongs to, so the evaluation shows
  it only at that frame and undo covers it. When the frame changes, the command
  step drops it (not an undo step); it is never saved.
- **The gizmo sat at the document's position**, not the animated one: it now uses
  world matrices at the current frame.
- **Rotation keys from the gizmo.** Converting the rotation back to H/P/B angles
  jumps from 180° to −180°, and keys would then spin the object the long way round.
  The gizmo keeps each angle within half a turn of its previous value.
- **Undo labels for the key buttons.** The Attribute Manager records its edits per
  tab; a key button click reports its own label ("Set Key", "Delete Track").
- **Idle frames between keys.** A test now runs every panel on a document animated
  between two keys and checks that nothing is written while nothing happens.
- **A bigger `NodeData`** (the temporary value) made `EditorCommand` larger, and
  clippy flagged the chord table that holds one. It is built per key press and
  never stored, so the lint is acknowledged there.
- **Deliberate differences from the reference**, written in the spec: a track whose
  last key is deleted goes (the reference keeps it empty), and the key strip shows
  the keys of all selected objects merged per frame.

*Bugs found by testing the app on screen, and fixed:*
- **The transport buttons ran into the range fields** at 1280 × 800 (125 %
  scaling), now that there are nine of them. They shrink to fit narrow windows.
- **A key at the current frame hid under the playhead.** The playhead is now drawn
  under the key marks.
- **On-screen testing:** the first posted click after the app starts is lost, so
  runs now begin with a harmless click on the status bar. The scenario (F9 at frame
  0, frame 31, a gizmo drag, F9, frame 16, select the key, Autokeying) showed every
  state: the yellow ring, red dots, the in-between value, the selected mark and the
  red frame.

*Step 6.4 — problems and solutions:*
- **One drag, several objects.** Moving keys can change several objects at once,
  but a live edit recorded one object's previous data. A new command records the
  previous data of all of them as one undo step (and nothing when nothing
  changed).
- **Tab switches the panel, but egui uses Tab to move keyboard focus.** Handled
  like other shortcuts, the key arrived after egui had already given focus to the
  first widget (the File menu), which then swallowed every shortcut. Tab is now
  taken out of the window's input before egui sees it, only while the pointer is
  over the bottom panel and nothing has focus.
- **Ctrl+wheel arrives as a zoom gesture and Shift+wheel as sideways scrolling**
  in egui. The graph reads both: Ctrl zooms time only, Shift values only.
- **Values of very different sizes share one graph.** Rotations are shown in
  degrees, so a quarter turn (90) sits next to positions in centimetres instead of
  1.57 next to 100.
- **Handles stay inside their segment.** Manual handles are stored as dragged; the
  curve shortens them to the segment when it evaluates, as since 6.2, so dragging
  one far past the next key cannot make the curve go back in time.
- **Two details found on screen:** the axis showed "-0" for values a hair below
  zero, and keys at the edge of the range sat under the frame of the graph. Labels
  now round to zero, and framing leaves a wider margin.
- **On-screen testing:** the scenario recorded keys at frames 0 and 31, opened the
  F-Curve tab, selected the last key (its frame and value appeared in the header)
  and dragged its handle with the real cursor: the curve took the new shape. The
  rest (Ctrl+click, boxes, Shift, menus, which posted input cannot reach) is
  covered by headless tests of the real panel.

*Step 6.5 — problems and solutions:*
- **World values do not round-trip exactly.** Converting a world position or
  rotation to the parent's space and back changes the last digits, so writing the
  fields as shown would have changed the object (and, with Autokeying, keyed every
  channel). Only the columns the user changed are written; entering the values
  shown changes nothing, which a test checks for every mode.
- **A size is not a parameter of every object.** Sizes are reached through the
  object's scale (a new size multiplies the scale by new ÷ old), so it works the
  same for primitives, polygon objects and generators' cages. Objects without
  extent (Nulls) show no size; Size+ measures the children in the object's own
  space.
- **Component rotations are relative.** The fields read 0; an edit turns the
  selection from the mesh as it was when the interaction began, so a drag never
  accumulates rounding, and the field reads 0 again afterwards.
- **The panel did not fit** the default 1280 × 800 window: the mode lists were cut
  off at the bottom. The heading went and the fields scroll when the manager is
  short.
- **On-screen testing:** the fields, the mode lists and a typed value were checked
  in the app; the Box inside the Group showed its world position (the Group's plus
  its own) and its 80 cm size. Typing through posted key messages doubles
  characters (the same happens in the Attribute Manager), so typed values are
  checked by the tests instead.

*Step 6.6 — problems and solutions:*
- **Where materials live.** Materials belong to the document, but evaluation only
  sees the scene, and skips frames by the scene's revision. The table sits in the
  scene, so editing a material re-colours the viewport through the usual revision
  check, with nothing new to watch.
- **Undo without new machinery.** The whole table is replaced by one edit (it is
  small), ids included, so tags keep pointing at the same materials across undo
  and redo. A deleted material's id is never reused, so a tag left pointing at it
  simply shows nothing until undo brings the material back.
- **Inheritance on the hot path.** Children without a Material tag show their
  parent's. The evaluation already walks the scene in order, so it keeps a stack of
  the materials of the current ancestors, reused every frame (no allocation).
  Exporters, which get flat object lists, compute the same from the parents.
- **Merging a document mixed up references.** A merged object's tags kept the ids
  of the other document; a Target tag could point at an unrelated object of this
  one (a bug since 6.1, found while adding materials). Object lists now carry their
  source ids and materials: merging adds the materials (with unique names) and
  points the tags at the new materials and objects, all in one undo step.
- **Drag and drop between panels.** A material is dragged from the bottom panel to
  the Object Manager (drawn before it) or the viewport (drawn after it). The drag is
  shared editor state; every panel can drop it on the frame of the release, and it
  ends the frame after.
- **Two details from testing:** a swatch could not start a drag in the frame it
  first appeared (nothing can be hit before it has been laid out once), and the
  large preview in the Attribute Manager pushed the settings out of a small window;
  it is smaller now.
- **On-screen testing:** a material was made from the Create menu, coloured with
  the colour picker, and dragged with the real cursor onto the Sphere's row and the
  Group's row: the Sphere and the Group's Cylinder and Box turned yellow and the tags
  showed the colour. Posted double-clicks do not reach the app (the second click is
  lost), so the double-click and Ctrl+N are covered by headless tests.

Phase 6 is complete except one part of its acceptance test: glTF export does not
write animation yet (planned).

## Phase 7 — Viewport: cameras, lights, 4-view, shading (2026-10-21 – )

| Step | Built | Tests |
|------|-------|-------|
| 7.1 (`ed7102e`) | Scene cameras: look through a camera (Cameras ▸ Use Camera, a double-click on its icon) or any object, with its position, bank and field of view; navigation moves it (one undo step per gesture, Autokeying, Protection); wire pyramids for the other cameras; new cameras clone the view | 441 |
| 7.2 (`ebbf93b`) | Four views (Perspective, Top, Right, Front) with orthographic projections: Panel menu arrangements, F1–F4 / F5, middle click shows a view alone; every view has its own camera, projection (Cameras menu) and Options; the active view takes input and a click elsewhere activates and acts; the grid lies in the view's plane | 448 |
| 7.3 (`96cc183`) | Display modes per view (Display menu, N~ chords): Gouraud, Quick and Constant Shading, each also with lines, Hidden Line and Lines; Wireframe (polygon edges) or Box (each object as its bounding box); selected objects' lines in orange | 452 |
| 7.4 (`6a7846c`) | Lights light the viewport: Omni, Spot (cone with inner and outer angle) and Infinite lights with falloff, the default light when there are none, Quick Shading always with it; materials' roughness, metallic and luminance shade the surfaces; spot cones and infinite arrows in the view | 459 |
| 7.5 (`88e2579`) | The Filter menu per view: Polygon, Generator, Subdivision Surface, Camera and Light objects, World Grid, World Axis and HUD on or off; All, None, Ctrl+click keeps one; hidden types are neither drawn nor picked or snapped to | 464 |
| 7.6 (`6e2c2d2`) | Shadows and SSAO per view (Options menu): the key light — the first with a Shadow, soft or hard — casts shadows (Spot, Infinite, Omni); ambient occlusion darkens contact areas and corners; the light's Shadow setting | 469 |

*Problems and solutions:*
- **The view camera had no bank.** The turntable camera kept world Y up, so a banked
  scene camera could not be shown. It now has a roll angle that its right and up
  vectors, the view matrix and the picking rays all follow, plus conversions to and
  from an object's world matrix (tested to round-trip, bank included).
- **One source of truth.** The view does not keep a copy of the camera object: every
  frame it takes the object's evaluated placement (so animation and Target tags are
  followed), and when navigation moved the view it writes the object back through
  the same path as the gizmo (temporary values, Autokeying, one undo step per
  gesture). Framing (H, O) happens outside the viewport's frame; a view that differs
  from what was last taken from the object is written back first.
- **Idle frames stay idle.** Writing only when the view actually moved keeps the
  document untouched while looking through a camera; a test covers it.
- **Unlinking** (Default Camera, or the object deleted) restores the editor camera
  where it was before linking.
- **On-screen testing:** the scene's Camera was linked through Cameras ▸ Use Camera:
  the view showed the scene from it, with its name as the title and its icon
  highlighted; dragging the orbit icon moved the camera object (the document became
  modified) and the view stayed on it; Default Camera brought the editor view back.
  The wire pyramid of the camera was visible from the editor view. Posted Alt and
  double-clicks do not reach the app, so Alt-navigation, the icon double-click and
  Protection are covered by headless tests.
- **One panel, four views, no duplicated tools.** The viewport's tools (gizmo,
  selection, modelling, snapping) all read the panel's camera and rectangle. Rather
  than threading a view index through all of them, the panel keeps the active
  view's state in its own fields and swaps another view's state in while that one
  is drawn. Only the active view handles input, so the tools never see a
  half-swapped state.
- **Orthographic cameras.** A parallel projection has no eye position, but the
  overlays, occlusion tests and snapping all use one. The camera puts its eye far
  behind the target (50 000 cm) and uses `distance` as the zoom (the visible
  height); occlusion and snapping cast rays from the view plane along the viewing
  direction instead of from the eye, and overlay lines are pulled a fixed distance
  towards the viewer (a share of the distance would shift them sideways). A test
  checks that a top view picks the corner on top of two corners on one pixel.
- **The grid in the view's plane.** The grid shader now takes the plane (XZ, XY or
  ZY) and a parallel flag in a spare uniform component, and picks the axis colours
  for that plane; fading towards the horizon is off in parallel views. Grid
  snapping still assumed the floor, so in Front and side views (whose rays run
  parallel to the floor) it never found the grid — it now uses the same plane.
- **One renderer for all views.** Pipelines and the GPU mesh cache are shared; each
  view has its own offscreen target and egui texture, made on first use. Views
  render one after another, each submitting its own pass.
- **Narrow headers.** With four views on a small window the navigation icons were
  drawn over the Panel menu. They are left out when there is no room.
- **Bug found on screen: a click did not activate another view.** The test driver's
  move, press and release can arrive in one frame together with the pointer leaving
  the window, so the pointer's last position was already empty at the end of the
  frame. The activation now takes the position from the press event itself.
- **On-screen testing:** F5 showed the four views with the demo scene exact from
  every side; a click in the Front view made it active (blue frame) and deselected;
  a click on the cylinder in the Top view selected it, with the gizmo in all four
  views; F4 / F3 showed Front / Right alone; a middle click showed Top alone and a
  second one brought the four back; the Panel and Cameras menus and the
  side-by-side arrangement worked. Posted Alt does not reach the app, so
  Alt-panning in orthographic views is covered by a headless test.
- **Lines without a new per-frame cost.** A wireframe built from the scene every
  frame would grow with the scene's edge count. Instead each mesh key gets a line
  list (its polygons' edges, from the topology) next to its triangles, built the
  first time a frame draws lines, and the line pass draws it instanced with the
  same per-object data as the surfaces. Edges come from the polygons, not the
  triangles, so n-gons show their outline.
- **Box mode** builds a second mesh per key, its bounding box, shaded or as
  twelve edges. A flat object (a plane) gets a hair of thickness so every side
  has a normal.
- **Hidden Line** draws the surfaces into the depth buffer only, then the lines:
  what is behind a surface is hidden, and the background (and the grid's
  depth test) show through.
- **Lines against their own surfaces.** Edges lie exactly on their polygons, so
  they would flicker against them; the line shader pulls them a tiny share of
  their distance towards the viewer (a fixed distance in parallel views, like the
  component overlay).
- **One cache, several views.** Views can now draw the same polygon mesh in
  different ways (as is, as a box). Polygon meshes used to be freed as soon as one
  render did not draw them, which would rebuild them every frame with mixed
  styles; they now stay for a few renders.
- **On-screen testing:** N~B, N~C, N~E, N~F, N~G, N~K and N~A were pressed in
  turn in the perspective view: lines over shaded surfaces (the selected cube's in
  orange), the flatter Quick Shading, flat Constant Shading, Hidden Line with the
  back edges and the grid hidden behind the surfaces, all lines in Lines, wire
  boxes, then shaded boxes. With four views, the Top view showed Lines, Front
  Hidden Line and Right Gouraud Shading (Lines) next to the shaded perspective
  view; the Display menu marked each view's own mode.
- **Lights needed more than colour and intensity.** A viewport lit by point lights
  only would not show what a scene's lighting is about, so the Light object got
  its type (Omni, Spot, Infinite) and the cone and falloff of the reference's
  Details tab. They are stored next to colour and intensity; the file format
  writes them only when they differ from the defaults, so every existing file —
  and the golden test file — stays byte for byte the same.
- **Lights where they really are.** Lights are collected after animation and
  Target tags have placed everything, so an animated or aimed spot lights the
  scene from where it is drawn. Like cameras, lights shine along −Z.
- **Any number of lights, no per-frame cost.** The lights go to the GPU in a
  storage buffer that only grows (to the next power of two) and is rewritten each
  frame from a reused list; the bind group is rebuilt only when the buffer grows.
- **Materials in the viewport.** Each instance now carries its material's
  roughness, metallic and luminance (160 bytes per instance). The shader sums the
  lights with a highlight whose size follows the roughness; metals have no
  diffuse colour and tint their highlights.
- **Black metal.** Under a single light a chrome sphere was almost black, because
  a metal only reflects and there was nothing to reflect. A dim studio reflection,
  brighter overhead, now shows on metals (dielectrics barely reflect it).
- **Bug found on screen: selecting a light made the document "modified".** egui's
  colour button converts the colour on every frame and writes it back (0.9 becomes
  0.90000004), so showing a light's colour — or a material's, or an object's
  display colour — changed the document, recorded an undo step and asked to save
  on closing. The swatch now writes back only when the user changes the colour;
  tests show every light setup and a colour without changing them.
- **On-screen testing:** a test scene (a floor, chrome, glossy red and matte blue
  spheres, a glowing cube, a spot light pointing down and a bluish Infinite light)
  showed the spot's soft-edged pool of light, the highlights following roughness,
  the chrome reflecting and the cube glowing; Quick Shading lit everything evenly
  with the default light; the demo scene is now lit by its light. The Spot's
  Type, Use Inner, angles and Falloff showed in the Attribute Manager, and the
  document stayed unmodified.
- **One filter for drawing, picking and snapping.** The evaluated nodes now carry
  their object kind, and a small bit set per view (`ViewFilter`) says which kinds
  and elements it shows. The same value goes to the frame extraction, picking,
  rectangle selection, snapping and the overlays, so a hidden type can never be
  clicked or snapped to in the view that hides it, and the views stay
  independent. The grid, triad and HUD toggles became entries of the same filter
  (Cinema 4D keeps them in Filter too), so All and None cover them.
- **On-screen testing:** the Filter menu listed the entries with their checks;
  turning Camera off removed the camera pyramid, World Grid the grid, None
  everything but the selected cube's gizmo. With four views, Generator off in
  the Top view removed the primitives there only, while its light marker and the
  other views stayed. (A menu closes after each choice, so each toggle is one
  click on the menu.)
- **Shadows without a fixed scene size.** A shadow map only works if it covers
  the scene tightly. The frame now gathers the world bounds of what it draws
  (markers left out), and the key light's views are fitted to them: an
  orthographic box along an Infinite light, a perspective view through a spot's
  cone, six views for an Omni light — slightly wider than 90° each, so filtering
  near a face's edge stays on that face. The main pass picks the face by the
  dominant axis, so one depth array texture serves every light type.
- **Acne and gaps.** Surfaces shadowing themselves are avoided by a slope-scaled
  depth bias in the shadow pass plus an offset along the normal of 1.5 shadow
  texels (computed at the scene's centre), which keeps contact shadows where
  objects meet the floor.
- **Markers cast nothing.** A light's own marker sphere would have put the whole
  scene in its shadow. Instances now flag markers, and the shadow and SSAO
  passes clip them.
- **SSAO needs depth and normals the main pass does not keep.** The main pass is
  multisampled and discards its depth, so a single-sample pre-pass writes
  view-space normals and depth; the occlusion and blur passes run on that, at
  the view's size. Their textures belong to each view's target (made the first
  time SSAO is on), so four views of different sizes never thrash shared
  textures. Parallel views reconstruct positions with their orthographic
  projection.
- **SSAO that shows.** Darkening only the soft light was correct but nearly
  invisible in scenes lit by their own lights (the soft light is weak there).
  It now also darkens the direct light by 60 %, closer to how the reference
  shows it.
- **Bug found on screen: the app did not start.** wgpu rejected the placeholder
  occlusion texture, made without the usage flag that lets shaders read it. The
  GPU passes are only checked by wgpu when the app runs; the WGSL is validated
  by the tests.
- **On-screen testing:** with Shadows on, a spot light with soft shadows put a
  soft shadow behind each sphere; an Omni light with hard shadows cast sharp
  shadows in every direction (below, beside and behind the objects); SSAO
  darkened the bases of the spheres and the cube; with four views, the Top view
  (orthographic) showed both effects while the Right and Front views, without
  them, stayed plain.

Phase 7 is complete.

## Phase 8 — Offline renderer (2026-10-21 – )

| Step | Built | Tests |
|------|-------|-------|
| 8.1 | The path tracer core: a new crate `grid4d-raytrace` that renders the scene (render visibility and levels) through a camera — every light with its Shadow setting, diffuse and glossy/metal materials, luminance, two bounces, the automatic light, a transparent background — repeatably and in parallel; an example renders documents to PNG; a reference-image test | 485 |

*Problems and solutions:*
- **What the renderer sees.** The evaluation was built for the viewport (editor
  visibility, viewport levels). It now has a purpose: `EvaluatedScene::for_render`
  takes the objects visible in the renderer and builds Subdivision Surfaces at
  their render level, through the same code and caches.
- **One BVH for the whole scene.** Picking keeps a BVH per mesh in object space;
  a path tracer wants one over everything in world space. The renderer bakes
  every triangle into world space as its own face of a single `PolyMesh` and
  reuses `grid4d-geom`'s tested BVH; the polygon a ray hits is then the
  triangle, whose barycentric weights interpolate the smoothed normals.
- **The same brightness as the viewport.** A light of strength 1 on a white
  matte surface facing it shows 1 in both: the renderer's reflection carries the
  factor π that makes its physically based material match the viewport's
  simpler one, and colours are linear until the sRGB conversion at the end.
- **Repeatable in parallel.** Each pixel sample draws from its own random
  stream seeded by the pixel, the sample number and the render's seed, so the
  picture is identical whether one thread or sixteen render the rows (a test
  renders both ways).
- **Fireflies.** Bounces that landed on the sharp highlight of a glossy or
  metal sphere left bright specks on the floor. Light picked up after a bounce
  is now capped, which removes them at the cost of a little caustic light.
- **Reference images across machines.** Floating-point details differ slightly
  between processors, so the reference test compares within a tolerance (a
  mean difference of at most 2 of 255, at most 1 % of values off by more than
  32) rather than exactly; `GRID4D_UPDATE_REFERENCE=1` rewrites the picture
  after an intended change.
- **Checking the result:** the example rendered the demo scene through its
  camera (lit by its light, without shadows, as its Shadow is None) and the
  test scenes: soft spot-light shadows, hard Omni shadows in every direction,
  the chrome sphere reflecting the floor and the other spheres, the glossy red
  highlight, the glowing cube, a transparent background. 960 × 540 at 64
  samples took under half a second on 16 threads for these small scenes.

---

## How this file is kept

After every step (and every fix worth noting) this file gets the new step in its
phase's table, its problems and solutions, and an updated Status block. The manual is
updated in the same change.
