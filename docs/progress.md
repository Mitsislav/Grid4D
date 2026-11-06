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
| Current phase | 15 — Materials, UVs and textures (v1.1) |
| Last step done | 15.1 — Specs and the `.g4d` format version 4 (2026-11-06) |
| Next step | 15.2 — UVs from every primitive and generator |
| Tests | 783 (and 4 ignored, run by hand: two timings, the walkthrough and the README pictures), all passing; clippy and rustfmt clean |

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
- **A stray control character (BEL) slipped into a docs file.** Every commit is now
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
| 8.2 | Render Settings (Ctrl+B, Render menu, toolbar): a window with Output (size, presets, Lock Ratio, frame range and step), Save (file, PNG, alpha), Sampling (samples, bounces) and Options (Auto Light) pages; the settings live in the document, are undoable and saved in `.g4d`; the renderer and its example take them | 499 |
| 8.3 | The Picture Viewer (Shift+R, Render menu, toolbar): renders run in the background and sharpen pass by pass, can be stopped, are saved where Render Settings say, and stay in a history; zoom, pan, Save As | 511 |
| 8.4 | The Render View (Ctrl+R, Render menu, toolbar): the active view rendered in place at its size, sharpening pass by pass, cleared by Esc or as soon as the view, the scene, the settings or the frame change | 514 |
| 8.5 | Frame ranges and OpenEXR: Render to Picture Viewer renders every frame of the Render Settings' range (at its step, through a linked camera that may move) and saves each finished frame as a numbered PNG or OpenEXR file (Format, Name on the Save page) | 521 |

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

**8.2 — Render Settings**

*Problems and solutions:*
- **Where the settings live.** They describe the document, not the editor, so
  `RenderSettings` is a field of `Document` next to the time settings, changed
  through a new `SceneEdit::SetRender` that validates (clamps) every value — so
  an undo step and a loaded file can never hold an impossible size or sample
  count. The random undo/redo test now also changes render settings.
- **The file format without a new version.** Version 3 grows additively: a
  `render` entry is written only when the settings differ from the defaults, so
  every existing file — and the golden test file — stays byte for byte the
  same, and files without it load with the defaults.
- **A window, not a modal dialog.** Like the reference, the settings stay open
  beside the workspace. Its fields write the document live and record one undo
  step per interaction (`LiveEdit`); closing the window in the middle of typing
  records that change too.
- **Lock Ratio without drift.** Recomputing the other side from the current
  proportions on every step of a drag would let rounding creep in (1280 × 720
  dragged slowly would end up taller than 16:9). The proportions are taken from
  before the interaction, with integer arithmetic.
- **A window that jumps.** Centred on its middle, the window moved whenever a
  page of another height was shown, so the next click landed elsewhere. It is now
  placed by its top-left corner (centred the first time) and every page is as
  tall as the tallest.
- **A separator that stretched the window.** A vertical separator between the
  page list and the page grew to the window's full height, which in turn grew the
  window. The page list is now a well of its own height.
- **Paths that travel.** Choosing the file with **…** stores a path relative to
  the document's folder (with `/`) when the file is inside it, so a project
  folder can move or change machines; a relative path needs the document saved
  before a render can be saved.
- **Testing the window headless.** egui gives no access to where a widget is, so
  the UI tests reach fields with Tab (close button, title bar, the page buttons,
  then the fields) and type into them: a typed Samples value is one undo step, a
  typed Width with Lock Ratio sets the Height, a typed From frame makes the range
  Manual.
- **On-screen testing:** the gear button and Ctrl+B open and close the window;
  Lock Ratio on and Width 1920 gave a Height of 1080 and the Preset showed HD
  1080; All Frames showed 91 frames (0 to 90); the Save page took
  `renders/shot` and explained it waits for the document to be saved; each
  change marked the document modified and could be undone. The example
  renderer took the demo document's settings (1280 × 720, 16 samples, an RGB
  PNG without alpha).
- **Driving the app:** the first click after the window appears is swallowed,
  and posted drags do not reach egui's drag values; a harmless first click and
  typing into fields worked instead.

**8.3 — The Picture Viewer**

*Problems and solutions:*
- **A picture that sharpens without changing the result.** Rendering in passes
  would normally give a slightly different picture than one render, because
  floating-point sums depend on their order. The renderer now keeps, per
  pixel, the running sum of its samples and adds each pass's samples in sample
  order — exactly what a single render does — so the finished picture is
  identical (a test compares them byte for byte), and the reference image
  still matches. `render` itself is now one pass of that.
- **Never blocking the editor.** A render works on its own thread over a copy of
  the scene (meshes are shared handles, so the copy is cheap); it evaluates the
  scene for the renderer there, so even a heavy Subdivision Surface does not
  pause the UI. The editor takes the worker's news once a frame and, while a
  render runs, asks egui to repaint every 100 ms even with the window closed,
  so a finished render is saved without waiting for the mouse to move.
- **Stopping at once.** Every row checks a shared flag before it starts, so
  Stop takes effect within a row; the pass that was cut short is dropped and
  the picture keeps the last complete pass. A render that is replaced or
  dropped stops its worker the same way.
- **Passes that stay frequent.** Passes of 1, 1, 2, 4 … samples, at most 32,
  show a first picture at once and keep refreshing on long renders; each pass
  sends two 8-bit pictures (with and without transparency).
- **Where the file goes is decided at the start.** The document may be saved
  elsewhere or its settings changed while a render runs, so the target (or the
  reason it is not saved) is fixed when the render starts. A missing folder
  (`renders/`) is created.
- **Paths with mixed separators.** On Windows `renders/shot` showed as
  `…\renders/shot.png`; typed paths are now rebuilt from their parts.
- **PNG writing.** `png` moved from the renderer's examples into `grid4d-io`
  (`save_png`, atomic like documents), the crate that writes files.
- **On-screen testing:** the toolbar button opened the window and rendered the
  demo through the editor camera (1280 × 720, 16 samples in 0.5 s); with 4096
  samples the picture sharpened while the bar and the history entry counted the
  samples, and Stop kept the picture at 320 samples, marked stopped and not
  saved; a second render joined the history, newest first; 100 % showed one
  picture pixel per screen pixel. Without a file in the Render Settings the
  status line said so.
- **Driving the app:** the display now runs at 100 % scaling, so last week's
  click positions (125 %) landed elsewhere — one created a sphere in a run that
  was then discarded. Posted mouse-wheel messages did not reach the viewer, so
  the wheel zoom is checked by a headless test instead.

**8.4 — The Render View**

*Problems and solutions:*
- **One worker, two uses.** The Picture Viewer's job was taught to spawn and
  drain generically, so the Render View is a second job beside it with its own
  picture; both can run at once and neither waits for the other. The view's
  picture is never saved and stays out of the history.
- **Knowing when the picture is no longer true.** Rather than reacting to each
  kind of change, the Render View remembers what it was made of — the view, its
  camera, its size in pixels, the scene's revision, the render settings and the
  frame — and the viewport compares that with the present at the start of every
  frame; any difference clears it and stops its worker. Selection lives outside
  the scene's revision, so selecting keeps it, as intended. An edit applied at
  the end of a frame is noticed on the next one.
- **Exactly the view.** The render uses the view's own camera and pixel size,
  so the picture lines up with the overlays drawn over it (a test compares it
  with a full render at that size, and on screen the gizmo and the selection
  corners sat on the rendered cube).
- **Esc without stealing it from tools.** Esc clears the Render View only when
  no mouse button is held; drags (Live Selection, mesh tools) keep using Esc
  to cancel themselves, and a pending chord takes it first.
- **On-screen testing:** the toolbar button rendered the perspective view in
  0.1 s at its size (16 samples), with the status line in the corner and the
  button lit; Esc brought the normal view back. Clearing on camera moves, edits,
  settings, frames and view changes is covered by a headless viewport test.

**8.5 — Frame ranges and OpenEXR**

*Problems and solutions:*
- **OpenEXR without a heavy dependency.** The format's simplest form — one
  part, scanlines, 16-bit floats, ZIP blocks of 16 lines — is written by hand
  (about a hundred lines): the header attributes, an offset table filled in as
  blocks are written, and each block's bytes split into even and odd halves and
  delta-coded before zlib, as the format expects. zlib comes from `flate2` with
  the pure-Rust backend `zip` already uses, half floats from `half` (both were
  already in the dependency tree). A writer checked only against itself proves
  little, so the tests decode the files with the independent `exr` crate (dev
  only): every value, both channel sets, a short last block, clamping.
- **Linear light for EXR, screen colours for PNG.** The worker now hands every
  finished frame back in the renderer's own linear, premultiplied values; the
  editor converts to sRGB bytes only for PNG, and writes EXR straight from them,
  so highlights brighter than white survive.
- **Saving as frames finish.** Each frame is saved the moment it is done, not
  at the end, so a long animation that is stopped (or a crash) keeps what was
  rendered; a failing folder shows one alert, not one per frame.
- **Animated cameras.** A render that looks through a camera object follows
  it: the worker evaluates the scene at each frame and places the camera where
  it is drawn then (animation, Target tags), falling back to the view's camera
  if it was deleted. A test renders three frames of a moving camera and gets
  three different pictures.
- **Older files.** Documents saved with 8.2's Render Settings have no format or
  naming; both are optional in the file (omitted while default), so those files
  load with PNG and `Name0000`, and the golden file is unchanged.
- **One frame, no number.** Numbering only when the range gives several
  pictures, so a still keeps the plain name it had in 8.3 (`shot.png`).
- **On-screen testing:** a document with Frame Range 0–4, Frame Step 2,
  OpenEXR, Name.0000, Alpha Channel and File `anim/shot` rendered three frames
  in 0.3 s and wrote `anim/shot.0000.exr`, `shot.0002.exr`, `shot.0004.exr`
  next to it; the Picture Viewer showed one History entry ("3 frames"), the
  frame count, and the transparent background over the checkerboard; the Save
  page said "Saves 3 frames as …shot.0000.exr …".
- **Driving the app:** posted key presses go through the user's (Greek)
  keyboard layout, which added Greek letters to a typed path, and character
  messages alone are ignored; the test document was prepared instead (the
  golden file with Render Settings added) and opened from the command line.

Phase 8 is complete.

---

## Phase 9 — Splines & spline generators (2026-10-29 – )

| Step | Built | Tests |
|------|-------|-------|
| — | Phase 9 planned in detail: splines, point editing and the Pen, Extrude / Lathe / Sweep / Loft, Text (`IMPLEMENTATION_PLAN.md`) | 521 |
| 9.1 | Spline primitives: Circle, Rectangle, Arc, n-Side, Star, Helix and Flower — curves with a plane, Reverse and intermediate points; drawn as lines in every display mode, picked near their line, framed, filtered, undone and saved | 540 |
| 9.2 | Editable splines: Make Editable on spline primitives; Points mode on a spline — click, rectangle and brush selection, the gizmo on points (tangents along), Bezier handle drags (Shift breaks them), Delete, Select All / Invert; Type and Close Spline in the Attribute Manager | 548 |
| 9.3 | Spline commands and the Pen: Mesh ▸ Spline — Hard / Soft Interpolation, Reverse Sequence, Set First Point (selection follows the points); the Pen tool — click adds a point on the grid's plane, a drag pulls its tangents, a click on the first point closes, Esc / Enter finish, Type option; one undo step per point | 552 |
| 9.4 | Extrude generator: a spline pushed along Auto / X / Y / Z / Custom / Absolute by an offset, subdivided; start and end caps (N-Gons or Triangles) with holes cut out (even-odd nesting, hole bridging), outward whichever way the outlines run; Reverse Normals, Hierarchical; Alt wraps the selected spline, Make Editable bakes the solid; saved; rendered | 564 |
| 9.5 | Lathe generator: a profile turned around the Lathe's Y axis — angle, subdivision, movement (screws, springs), scaling (spirals), Reverse Normals, caps where a closed profile's turn stays open; a full turn closes and shares the points on the axis; saved; rendered | 572 |
| 9.6 | Sweep generator: a profile moved along a path — banking (parallel transport), level or parallel movement, constant cross section at corners, end scale and rotation, start / end growth, caps; Alt wraps two selected splines as profile and path; Make Editable removes both; saved; rendered | 582 |
| 9.7 | Loft generator: a skin over child splines in order — Subdivision U / V, through the sections (Catmull-Rom) or Organic (B-spline), Linear, Per Segment, Loop, caps; sections turned the same way round; Alt wraps every selected spline in selection order; Make Editable removes them; saved; rendered | 589 |
| — | Phase 9 review: export leaves every generator input out and uses the colour shown; only the Subdivision Surface hides its first child; generators follow animated inputs (placed at the evaluated frame) | 592 |
| 9.8 | Text spline: letters' outlines from the built-in font (Ubuntu Light) as closed Bezier segments — multi-line text, Align, Height, Horizontal / Vertical Spacing, plane, reverse; Extrude makes solid letters with their holes; Make Editable; saved; framing (H / O) uses evaluated bounds for texts and generator results | 600 |

**9.1 — Spline primitives**

*Problems and solutions:*
- **Five curve types, one sampler.** Linear, Cubic (Catmull-Rom), Akima, B-Spline
  and Bezier splines are all turned into chains of cubic Bezier spans — Hermite
  tangents become Bezier controls, a B-spline span's four points convert
  exactly — so the intermediate-point modes (Natural, Uniform, Adaptive,
  Subdivided) are written once. Natural and Uniform give the reference's
  documented point counts; Adaptive measures how far each span turns and cuts
  it into just enough even pieces (a circle at 5° gets 72), rather than halving
  pieces, which would give powers of two.
- **Primitives that stay editable.** Each primitive builds a real control
  spline — a circle is four Bezier points with tangents, rounded corners are
  Bezier quarter arcs between straight sides — so Make Editable (9.2) can keep
  it as a spline instead of a polyline. A general corner rounder serves the
  Rectangle and the n-Side, clamping the radius so neighbouring roundings never
  overlap.
- **Seven kinds, one variant.** Adding seven object kinds would have meant
  seven new arms in a dozen exhaustive matches. Splines are one
  `ObjectKind::SplinePrimitive` holding a shape, while `ObjectKindTag` keeps a
  tag per shape for names, icons and menus.
- **Lines in every mode.** The renderer already drew polygon edges with an
  instanced line pipeline; curves reuse it, with a line list built once per
  sampled curve (named by a `CurveId`, like a polygon object's `MeshId`), drawn
  even in shaded modes and in the object's own colour (the shader's unlit
  flag), never filled, never in shadows, ambient occlusion or the path tracer.
- **Picking thin things.** A ray almost never hits a curve, so splines are
  picked on screen: the pointer's distance to the projected polyline, within
  the same few pixels as component picking. A spline under the pointer wins
  over the surface behind it.
- **Files.** The spline kind is additive in version 3; plane, Reverse and the
  intermediate points are left out while default, angles are radians like
  rotations, and counts (sides, points, subdivision) are clamped on loading.
- **On-screen testing:** a document with all seven primitives (made with
  Python's zipfile, since typing letters into the app is unreliable with the
  Greek layout) showed every curve, their icons in the Object Manager, the Star
  in the selection colour with its gizmo and its parameters, and the toolbar's
  spline list.

**9.2 — Editable splines**

*Problems and solutions:*
- **Polygon tools everywhere.** Points mode, its picking, the GPU overlay, the
  selection tools and the commands were all written around polygon meshes.
  Rather than threading splines through every mesh tool, splines got a small
  parallel path: `spline_target` (Points mode on an editable spline), points and
  handles picked and drawn on screen in 2D, and spline branches in the few
  commands that apply (selection, Select All / Invert, Delete). Clicks, frames
  and brush strokes still end in the same `SelectComponents` command, so undo
  and the stored selection work as for polygon points.
- **One point edit for meshes and splines.** The gizmo's `PointEdit` kept the
  original `MeshHandle`; it now keeps either that or a `SplineHandle`. A
  spline's moved points take their tangents along — transformed by the edit's
  linear part — so rotating or scaling points keeps the curve's shape around
  them.
- **Handles that stay smooth.** Dragging a handle moves it in the plane through
  it facing the view; the opposite handle turns to stay in line and keeps its
  own length (Shift breaks the pair). The drag writes live and records one
  "Move Tangent" step; Esc restores the spline as it was.
- **Hot path discipline.** The first version boxed a sampling closure in
  `EvaluatedScene::update`, which the rules forbid on the per-frame path; the
  curve source is now a small enum, sampled by a plain function only when it
  changed.
- **Make Editable keeps the curve exactly.** A primitive's control spline (with
  its plane and Reverse applied) becomes the editable spline, so its samples
  are identical — a test compares them.
- **On-screen testing:** the Circle in the spline test document became a
  "Spline Object" with four points in Points mode; a click selected one, which
  showed its two handles and the gizmo; the Object tab showed Type (Bezier) and
  Close Spline; Delete removed the point and the curve closed through the other
  three. Handle drags, frames and the gizmo's rotation of tangents are checked
  by headless tests (posted drags do not reach egui).

**9.3 — Spline commands and the Pen**

*Problems and solutions:*
- **Selections that follow the points.** Reverse Sequence and Set First Point
  renumber points. The geometry functions return the new spline together with
  an old → new index map, and the command moves the selected and hidden flags
  through it, so the same points stay selected (a test checks it after Set
  First Point).
- **Commands that change nothing.** Set First Point on a point that is already
  first, or Hard on corners, would have made empty undo steps; the command
  compares the result with the spline and its components and records nothing
  when they are equal.
- **Underflow in Set First Point.** `(old - first + n) % n` underflows `usize`
  for points before the new first one; reordered to `(old + n - first) % n`,
  and then rewritten over the map's slice to satisfy clippy's range-loop lint.
- **A click in one frame.** On screen the Pen first added nothing and the click
  deselected the cube: posted clicks arrive as press and release in the same
  egui frame, and by then the pointer's press origin is cleared, so the Pen
  ignored the press and the click fell through to object picking. The Pen now
  falls back to the pointer position and takes every click in the view while
  it is the tool; a headless test sends a one-frame click.
- **When drawing ends.** "Finished" is editor state (`pen_finished`, the spline
  the Pen will not extend). It is cleared as soon as another object becomes
  active, so selecting a spline and picking the Pen again continues it.
- **On-screen testing:** Create ▸ Spline ▸ Pen, four clicks on the floor made a
  Spline object with four points; a click on the first point closed it. In
  Points mode, Select All and Mesh ▸ Spline ▸ Soft Interpolation turned the
  corners into a smooth curve with handles; Mode ▸ Tool showed the Pen's Type
  option. The preview line follows hover, which posted mouse moves do not
  produce in egui; a headless test checks where the cursor meets the floor.

**9.4 — Extrude generator**

*Problems and solutions:*
- **Generators with spline inputs.** The generator code knew only the
  Subdivision Surface (its options were `SubdivisionOptions`, its inputs
  meshes). It now carries `Options::{Subdivision, Extrude}` and `Source` gained
  spline primitives (by parameters) and editable splines (by id and
  interpolation), so the same cache decides when an Extrude is rebuilt: moving
  the Extrude or selecting points changes nothing, a parameter or the spline's
  placement under it does.
- **Which way is out.** A spline can be drawn either way round, and an outline
  can hold holes. The solid projects every closed segment onto the plane across
  the movement, counts how many other segments contain it (even: an outline,
  odd: a hole) and turns outlines counter-clockwise and holes clockwise around
  the movement. Walls built from that order face outwards, and so do caps
  triangulated in the same plane; tests check the enclosed volume for all four
  combinations of outline and hole directions.
- **Caps with holes.** The triangulator only cut simple polygons. Holes are now
  joined to their outline by a bridge from the hole's rightmost point to a
  point of the outline it can see (Eberly's method), and the ear clipper runs
  on the resulting ring, whose bridge points repeat; it already skipped points
  equal to the ear's corners. Triangles that would repeat a point are dropped.
- **The input stays visible.** Generators hide their cage. A spline under an
  Extrude must stay drawn and pickable (it is what you edit), so curves inside a
  generator keep their line and pick shape.
- **On-screen testing:** a document with five Extrudes (a Circle with Ring, a
  rounded Rectangle, a Star, a Flower with three subdivisions and an open Arc)
  showed a tube with a hole, solid shapes and a wall without caps; selecting an
  Extrude showed its Object properties. The same file rendered with the path
  tracer (3110 triangles). Posted drags still do not move egui's panel
  dividers, so the lower Attribute Manager rows were checked headless.

**9.5 — Lathe generator**

*Problems and solutions:*
- **One profile code for two generators.** The Extrude's nesting, outward
  ordering and caps were inside `extrude`. They moved into `profile` (runs
  projected by a closure, nested even-odd, ordered outwards), `caps` (cap
  polygons with holes, counter-clockwise in the projected plane) and `assemble`
  (repeated corners dropped); the Extrude's tests passed unchanged.
- **Which way is out on a lathe.** The profile is measured as (distance from
  the axis, height), so it may lie in any plane through Y. A counter-clockwise
  profile turned the positive way faces the axis, so the shell is flipped for
  positive angles; caps at the start face against the turn. Open profiles are
  ordered by the sign of the area between them and the axis (∑ ρ dy), so a cup
  drawn either way has an outward wall and a downward bottom; tests check
  volumes for both directions and both angle signs.
- **Points on the axis.** A full turn would stack 24 copies of a vase's centre
  point and make zero-area quads. Points on the axis keep one index on every
  ring while nothing but the angle moves them (no Movement, Scaling 100 %);
  faces then lose the repeated corners and become triangles (or vanish).
- **On-screen testing:** a document with five Lathes (a vase from a cubic
  spline, a torus, a four-turn screw with 180 cm of movement, a half turn of a
  rounded rectangle with caps, a spiral shrinking to 25 %) showed all five as
  expected, with the profiles still drawn; the path tracer rendered the same
  file (30 568 triangles).

**9.6 — Sweep generator**

*Problems and solutions:*
- **Two inputs.** Generators took their first child; the Sweep needs the
  first two (profile, path). `ObjectKind::generator_inputs` says how many, so
  Make Editable removes both and nothing inside them is baked on its own;
  `gather_sweep` reads the profile in its own space (moving it changes nothing)
  and the path in the generator's. Alt + create wraps the first two selected
  objects in selection order, moved in one undo step.
- **A frame that does not twist.** Banking transports the profile's X axis from
  one station to the next by the rotation between the path directions, starting
  in the path's mean plane (Newell's normal, largest component made positive,
  so a path on the floor starts level). On a closed path the frame comes back
  turned; the leftover angle is spread evenly round the loop. A saddle-shaped
  loop in the tests has no jump at its seam.
- **Corners.** At a corner the direction is the average of the two edges, and
  the profile is stretched across the corner by 1 / cos(half the turn), so a
  bar swept round a square keeps its width (√2 across the corner); a hairpin is
  limited to 5×.
- **Growth on a closed path.** Partial growth cuts a loop open (once round, by
  length) and it gets caps; end scale and rotation keep following the whole
  path's length.
- **A test that measured the wrong thing.** The closed-tube test first
  compared the seam gap with a step *within* the profile; it now compares it
  with the step between rings, which is what "the seam lines up" means.
- **A panel that grew.** On screen, selecting a Sweep widened the right panel by
  about 16 px: the hint line under its parameters was longer than the panel.
  It now reads "Child 1: profile; child 2: path".
- **On-screen testing:** a document with four Sweeps (a circle along a helix,
  a rectangle along a rectangle, a star along an arc tapering to 15 % with half
  a turn of twist, a ring along a hexagon grown to 70 %) showed a spring, a
  frame, a horn and an open, capped tube; a probe confirmed the horn's
  cross-section (57 cm at the start, 8 cm at the end). The path tracer
  rendered the file (60 296 triangles).

**9.7 — Loft generator**

*Problems and solutions:*
- **Matching sections.** Sections have different point counts and shapes
  (a circle and a square). Each is resampled into the same number of points,
  evenly by length from its first point, so point *j* of every section lines up;
  a closed section gets U − 1 points (its last would repeat the first). Closed
  sections drawn the other way round are turned (keeping their first point)
  so they all run counter-clockwise around the direction of the skin, which
  also makes the skin face outwards; tests loft a frustum with either circle
  reversed and check its volume.
- **Through the sections, or near them.** "Organic Form" was read as an
  approximating surface: the sections become control shapes of a uniform cubic
  B-spline. Mirroring the sections beyond the ends (2·P₀ − P₁) makes both the
  Catmull-Rom and the B-spline curves start and end exactly on the first and
  last section; a test checks that the default skin passes through a narrow
  middle section and the organic one does not.
- **Every child an input.** `generator_inputs` became exhaustive over every
  object kind (no `_` arm) and returns "all" for the Loft, so Make Editable
  removes every section; Alt + create wraps every selected object, in selection
  order, none inside another.
- **On-screen testing:** a document with four Lofts (a vase from four circles
  in the XZ plane, a circle-to-square piece, a looped ring of six sections
  turned around Y, a sheet over four arcs) showed all four as expected with the
  sections drawn; the path tracer rendered the file (6316 triangles).

**Phase 9 review (after 9.7)**

A careful pass over the spline generators before 9.8 found three bugs, all
fixed with tests (592 tests):
- **Export kept the inputs.** `file::exportable` left out only a generator's
  first child and painted the result in that child's colour — right for the
  Subdivision Surface, wrong for the spline generators: a Sweep's path and a
  Loft's other sections were written as empty objects, and every solid came out
  in the splines' light grey. It now leaves out every input
  (`generator_inputs`) and takes the colour the viewport shows
  (`grid4d_eval::generator_color`: the cage's for the Subdivision Surface, the
  generator's own for the others).
- **Unused children hidden.** Every generator hid its first child's subtree as
  a cage; a cube placed under an Extrude disappeared without being used. Only
  the Subdivision Surface hides its cage now.
- **Animated inputs ignored.** Generators read their inputs' placement from
  the base transforms, so an animated Loft section or Sweep path did not move
  the result during playback or in renders. Inputs are now placed at the
  evaluated frame (`Scene::world_matrix_at`), and Make Editable and export bake
  the current frame (`generator_mesh(scene, id, frame)`).
- Docs: the manual's Filter and glTF export sections and the filter spec name
  the spline generators; §12 points to §13.4–13.7.

**9.8 — Text spline**

*Problems and solutions:*
- **A font without a new licence.** The plan asked for a bundled open font.
  egui already embeds Ubuntu Light in Grid4D for its own interface
  (`epaint_default_fonts`, Ubuntu Font Licence), and `ttf-parser` 0.25 was
  already in the dependency tree; using both adds no font file and no licence
  to the repository.
- **Keeping geometry "glam only".** Font parsing went into a new crate,
  `grid4d-text`, between `grid4d-geom` and `grid4d-eval`; the scene stores only
  the parameters (`geom::spline::Text`), with the string as `Arc<str>` so that
  evaluation's per-update clones of curve sources and generator inputs do not
  allocate.
- **Glyph outlines as splines.** TrueType contours mix lines and quadratic
  curves: each quadratic span becomes the equal cubic Bezier (control points
  two thirds of the way to the quadratic one); a contour whose last point lands
  on its first is merged into it. Holes keep the font's direction; the
  generators nest contours even-odd, so "o", "B", "4" and "D" extrude with
  their holes.
- **Bounds the document does not know.** A text's extent needs its font, so
  `ObjectKind::local_bounds` is `None` for it, and evaluation now takes every
  spline's bounds from its cached curve (no longer re-sampling primitives each
  update). On screen, **H** then framed nothing and reset the view: framing
  walked the document, where neither the text nor the Extrude around it has an
  extent. Framing now uses the evaluated bounds of such objects (texts,
  generator results), so H and O frame extruded letters; a test frames an
  Extrude of a text 2000 cm away.
- **A wrong count in a test.** "Text" has five contours (T, x, t and e with its
  hole), not four.
- **On-screen testing:** a document with an Extrude of "Grid4D" (centred, 160 cm)
  and a two-line text showed solid letters with their holes and the outline;
  H framed both; typing into the Text field changed the spline live. The path
  tracer rendered the letters (1728 triangles).

Phase 9 is complete.

---

## Phase 10 — Deformers & MoGraph (2026-10-29 – )

| Step | Built | Tests |
|------|-------|-------|
| — | Phase 10 planned in detail: animated parameters, Bend / Twist / Taper / Bulge / FFD, Current State to Object, Cloner (Linear, Radial, Grid, Object) and Plain / Random effectors; Phases 11–13 (simulation, workspace, extensibility) sketched (`IMPLEMENTATION_PLAN.md`) | 600 |
| 10.1 | Animated parameters: every real-valued Object-tab parameter keyable — key buttons beside the fields, Autokeying and temporary values, F-Curve editor and key strip; evaluated at the frame everywhere (viewport, generators, renders); Make Editable bakes the frame; saved by name | 609 |
| 10.2 | Deformers and the Bend: a deformer reshapes its parent and the parent's subtree (deepest first), inside a cage before smoothing or beside it on the result; Bend with Mode, Strength, Angle, Keep Y-Axis Length, Origin; its box drawn bent and picked; Shift + create under the selection, Fit to Parent; cached; animatable; saved; rendered | 621 |
| 10.3 | Twist, Taper and Bulge (curvature, fillet) beside the Bend; an orange strength handle on the selected deformer, dragged in the view (Shift steps, one undo step, Esc), before the gizmo; animatable; saved; rendered | 629 |
| 10.4 | FFD: a lattice (2–20 points per axis) over the box whose moved points reshape the object by Bernstein blending; edited in Points mode like a spline's points (selection, gizmo, Tweak); Reset Lattice; Size scales it, a new grid regrids it; Delete leaves it; saved with its selection; rendered | 635 |
| 10.5 | Current State to Object (Mesh ▸ Conversion): a polygon copy of what the viewport shows at the current frame (deformers, generators, animation), hierarchy kept, deformers and consumed inputs left out, after the original, selected, one undo step; export writes deformed shapes | 638 |
| 10.6 | Cloner: Linear, Radial and Grid Array layouts, Iterate / Random children, Reset Coordinates; clones as instances drawn in shared batches (1 000 cubes, one draw), picked and selected as the Cloner, framed, filtered, rendered; Make Editable / Current State / export as one object per clone; saved | 649 |
| 10.7 | Cloner Object mode (clones on another object's points or polygon centres, aligned to the normals; the object linked, saved and remapped; hidden objects still carry clones); child groups cloned whole; Cloners inside Cloners cloned with their clones | 653 |
| 10.8 | Plain and Random effectors: Strength, Position / Scale / Rotation, colour, seed, falloff (Infinite, Sphere, Box, Linear; soft edge) drawn as lines; linked through the Cloner's Effectors list (added when created with a Cloner selected); applied in order to clones' placements and colours; saved; rendered | 658 |

**10.1 — Animated parameters**

*Problems and solutions:*
- **Naming parameters.** Tracks needed to name "the radius" of any object
  without a field index that would break when kinds change. `ObjectParam` is
  one closed enum named by meaning (39 values; Radius, Height, Offset, …), with
  `ObjectKind::slot` mapping a (kind, parameter) pair to its field; files store
  the names (`"radius"`, `"size_y"`). A track an object does not have a
  parameter for is ignored.
- **A set of parameters outgrew its bits.** Temporary values are marked in a
  `ParamSet`, a 16-bit mask over the nine transform channels; with 39 object
  parameters after them it became a `u128`. That made `NodeData` larger, and
  clippy then flagged the F-Curve editor's drag enum for very unequal variant
  sizes; the tangent drag now boxes its saved `NodeData`.
- **Evaluation without copies.** Every update reads each object's kind at the
  frame; `NodeData::kind_at` returns the stored kind borrowed (`Cow`) unless a
  parameter is animated, so the hot path still allocates nothing.
- **Which rows get key buttons.** The Object tab is drawn by shared row helpers;
  they now take a `KeyRows` that finds a row's parameter by its label and draws
  the key button after the field (so the label column keeps its alignment).
  A test draws the Object tab of every object and requires exactly one button
  per parameter. It found three real mismatches — a light's falloff radius
  exists only with a falloff, an Extrude's offset and movement depend on its
  Direction, an Arc's inner radius exists only as a Ring — and one label
  clash: the rounding radius row was also labelled "Radius" (now "Rounding
  Radius").
- **Writing edits back.** `NodeData::set_kind_at` keeps animated parameters'
  base values and sends their changes through keys (Autokeying) or temporary
  values, while choices, counts and unanimated numbers are set for all frames.
- **A cut that took too much.** Restricting the falloff radius first went
  through a scripted edit whose end marker matched too late and removed a
  stretch of `param.rs`; the compiler caught it and the code was restored.
- **On-screen testing:** a document with a sphere whose radius is keyed 30 → 110
  over 30 frames, an Extrude of a Star with an animated offset and a Lathe with
  an animated angle: at frame 15 all three were half way, the Radius field read
  70 cm beside a red ring; a click on the ring set a key (red disc, the title
  marked unsaved) and the F-Curve editor showed the Radius curve through three
  keys.

**10.2 — Deformer framework and Bend**

*Problems and solutions:*
- **Who is deformed, cheaply.** The scene walk is pre-order, so a node's
  deformer children are known before its subtree: each visited node pushes its
  switched-on deformer children onto a stack tagged with its depth, and the
  stack is cut back on the way to the next sibling (as the material stack is).
  An object's chain is the stack read deepest group first, siblings in order;
  links hold the matrices into each deformer's space at the frame, so moving or
  animating a deformer is a changed chain.
- **Caching.** Deformed meshes are cached per node by (source mesh id or the
  primitive's parameters, links); idle frames and unrelated edits rebuild
  nothing, and a test moves an unrelated object to prove it. A deformed
  primitive is drawn from its own mesh instead of the shared unit mesh.
- **Generators.** A Bend inside a Subdivision Surface's cage must bend the cage
  before smoothing; generator inputs now carry the chain of deformers between
  them and the generator, applied as the cage is merged. A Bend beside the cage
  is simply in the generator's own chain and bends the result.
- **Drawing the box.** Instead of a new overlay, the box is a polyline: its
  twelve edges cut into pieces and bent by the deformer itself, cached and
  drawn by the curve pipeline and picked like a spline — so the bend shows even
  before an object is attached.
- **Bend maths.** A column turns through `strength · s / height` on a circle of
  radius `height / strength` (keep length) or with its chord the height; points
  to the side follow at their distance, points past the box continue along the
  end's tangent (Limited). Centre origins bend both halves by half the
  strength, mirrored (C) or point-mirrored (S). Strength is stated in degrees,
  not the reference's percentage, and the spec says so.
- **A filter mask one bit short.** The Filter menu's "hide everything" value
  was a fixed nine-bit mask; with the new Deformer entry the existing test of
  every entry failed until it covered ten.
- **Stale menu entries.** Create ▸ Generator still listed Extrude, Lathe, Loft
  and Sweep as greyed "planned" items beside the real ones (left over from
  Phase 9); only the Cloner is planned now, and Create ▸ Deformer lists Twist,
  Taper, Bulge and FFD as planned.
- **A panel that grew, again.** The hint under the Bend's parameters widened the
  right panel by about 46 px on screen; shortened to "Bends its parent's
  subtree".
- **On-screen testing:** a tube bent 120° into an arch, a column curled into an S
  (Center S-Shape, 180°) and a Subdivision Surface bent towards −Z (Angle 90°)
  showed with their boxes bent alike; selecting a Bend showed its rows with key
  buttons. The path tracer rendered the three shapes (6272 triangles).

**10.3 — Twist, Taper, Bulge and the strength handle**

*Problems and solutions:*
- **No formulas to follow.** The manual names the parameters and their effects
  but not the shapes, so the spec states ours: a Twist turns by
  `Angle · t`, a Taper scales distances from the axis by `1 − Strength · g(t)`
  with `g = t^(1 + Curvature)`, a Bulge by `1 + Strength · b(t)` with
  `b = 1 − |2t − 1|^(2 + 2·Curvature)`; Fillet eases with `3x² − 2x³`. The tests
  check the described effects (top to a point, middle doubled, ends kept).
- **Two strengths.** The Bend's strength is an angle, the Taper's and Bulge's a
  percentage; since an `ObjectParam` has one unit, the percentages are a new
  `Amount` parameter labelled "Strength" (labels only need to be unique within
  an object, which the key-button test checks).
- **One handle model.** Each deformer says where its handle is, which plane it
  moves in and what strength a point in that plane means; the viewport code is
  the same for all four. The Bend's inverse has no closed form, so it searches
  along the arc (two turns each way, then halving steps).
- **The gizmo got there first.** A headless drag on a Bend's handle did nothing:
  the gizmo, handled before the other tools, took the press because the
  handle sat on its Y arrow. The handle is now tried before the gizmo, which is
  what a user grabbing it expects.
- **A test with a wrong premise.** "The bounds change" fails for a Taper, whose
  base keeps its width; the test compares the deformed points instead.
- **On-screen testing:** a twisted column (180°), a tapered cylinder (80 %,
  curvature 60 %) and a bulging one (60 %, fillet) showed correctly, the
  selected Bulge with its handle on the X arrow; the path tracer rendered all
  three. Posted mouse drags do not reach the viewport (a gizmo drag did not move
  an object either), so dragging the handle is checked by the headless test.

**10.4 — FFD**

*Problems and solutions:*
- **Editing a lattice without a new editing path.** The plan expected a third
  case in the gizmo's point edits. Instead the lattice is stored as a linear
  spline's points (`SplineHandle`) with `Components`, and
  `ObjectKind::{points, points_mut}` gives "the points Points mode edits as a
  spline's" for both editable splines and FFDs; the point drawing, picking,
  click / rectangle / Live Selection, Select All / Invert, the gizmo and Tweak
  all work on the lattice with a handful of changed matches. Delete is the one
  command kept away from it (removing lattice points would break the grid).
- **A deformer that carries data.** The other deformers are `Copy`; the FFD's
  chain entry holds its lattice (a shared handle), so `Deformer`, `Link` and
  `Active` became `Clone` — still no allocation per update.
- **Size versus lattice.** Lattice points are stored in the deformer's space,
  so a new Size scales them (Fit to Parent too), and a new grid starts a
  regular lattice and clears its selection, since the old points no longer
  correspond.
- **The maths.** Bernstein weights per axis (computed into stack arrays of 20)
  blend each lattice point's offset from its grid place; a test lifts one
  corner and checks that corner moves by the full amount and the opposite one
  not at all. Clippy asked for the weight loops to iterate the weights directly.
- **On-screen testing:** a sphere under an FFD whose top-middle point is lifted
  and one side pushed out became an egg-like shape; in Points mode the 27
  lattice points and their lines showed over it. The path tracer rendered it.

**10.5 — Current State to Object**

*Problems and solutions:*
- **Where the shown shape lives.** Deformed meshes exist only in the
  evaluation's cache. Rather than deforming again in the UI, the command builds
  an `EvaluatedScene` at the current frame and reads what each node draws: a
  generator's result through its drawable's mesh key, and a deformed shape
  through the new `EvaluatedScene::deformed(id)`, which checks the cached mesh
  is still in use (the cache keeps entries for one revision, so a frame change
  alone could leave a stale one).
- **Selections on the copy.** Deforming moves points only, so a deformed
  polygon object keeps its selections and hidden elements — unless hidden
  polygons were left out of the shape, then they are dropped.
- **Inserting a hierarchy as one step.** Children need their parent's new
  `NodeId`, so the copies are applied one by one (parents first) and recorded
  together, newest first, with all-or-nothing rollback, as Merge does.
- **Export.** It now uses the same evaluation for deformed shapes, which the
  10.2 notes left for this step.
- **On-screen testing:** a cube under a Twist, then Mesh ▸ Conversion ▸
  Current State to Object: a polygon "Cube" appeared after the original,
  selected and twisted; switching the Twist off straightened the original while
  the copy stayed twisted. (Typing into the Coordinates fields does not reach
  the app through posted messages, so the copy was not moved aside.)

**10.6 — Cloner framework**

*Problems and solutions:*
- **Clones beside the nodes, not among them.** Making clones extra
  `EvaluatedNode`s would have broken everything that looks a node up by id
  (animation, Target tags, the gizmo). They are a second list,
  `EvaluatedScene::instances()`, built after animation and aims so they follow
  the Cloner; consumers chain it after the nodes (`objects.chain(clones)`), so
  drawing, picking and the path tracer each needed a few lines and no
  allocation.
- **Hiding the children but keeping their shapes.** During the walk the
  Cloner's direct children record what they would draw (drawable, pick shape,
  bounds) as `Source`s, then are hidden like a cage; the instances reuse those
  drawables, so a deformed child or a generator child is cloned as shown, and
  same-shape clones land in one batch.
- **Selecting and framing a Cloner** without a surface of its own: its bounds
  become the union of its clones' (so selection boxes and H / O work
  unchanged), and a new `PickShape::Clones` lets rectangle selection take it
  while a ray never hits the empty space between clones.
- **A disabled Cloner hid its children**: evaluation treated "disabled and not
  a mesh generator" as hidden. The test now asks `is_generator()`, which
  includes the Cloner.
- **Export wrote the child as well as the clones**: the exporter skips a
  generator's inputs only when it has a mesh; a Cloner's children are skipped
  explicitly. Found by the export test.
- **The Attribute Manager widened** with a long hint under the Cloner's rows;
  shortened.
- **On-screen testing:** a Radial Cloner of a cube and a sphere (12 clones,
  aligned) and a 4 × 4 × 4 Grid Array picking children at random; clicking a
  clone selected its Cloner; the path tracer rendered all 10 156 triangles.

**10.7 — Cloner Object mode, groups, nested cloners**

*Problems and solutions:*
- **An object link in a geometry type.** `grid4d-geom` cannot name a
  `NodeId`, so the layout (`Layout::Object(Surface)`) holds only how clones
  sit on a surface, and the link lives in the scene:
  `ObjectKind::Cloner { cloner, object }` (a constructor, `ObjectKind::cloner`,
  kept the 10.6 call sites short). The file stores it by object index and
  resolves it after every object exists, like Target tags; Merge remaps it.
- **Groups and nesting in one rule.** Instead of only the Cloner's children,
  every object inside an enabled Cloner is recorded (its nearest Cloner, and
  whether it is a child), so a clone of a child brings the parts below it.
  Cloners are built deepest first; when an outer Cloner meets an inner one
  among its parts it copies the inner one's instances (already built) under
  its own placement, and the inner ones are then dropped from the list — one
  `retain`, no allocation.
- **The object's shape.** Shown objects give the mesh the viewport draws (a
  generator's result, a deformed shape); a hidden polygon object or primitive
  still works through its own mesh, the primitive's built once and cached
  while its parameters stay the same.
- **Normals on points** are the area-weighted sums of the polygons' Newell
  vectors, accumulated in a reused buffer; Align turns the clone's Y axis
  onto them, transformed by the inverse transpose so scaled objects keep
  correct normals.
- **On-screen testing:** cones on every polygon of a sphere (Object mode,
  Polygon Center, aligned) and ten towers of four turning, shrinking cubes (a
  Linear Cloner inside a Radial one); the path tracer rendered both.

**10.8 — Effectors**

*Problems and solutions:*
- **Where effects go.** An effector changes the clone's layout placement, so
  it is applied to `cloner world × placement` (the falloff weighed at that
  point, in the effector's space), and only then the child's own placement
  and the parts of a cloned group follow — a lifted clone lifts its whole
  group.
- **Colours across parts without allocating.** Several effectors blend one
  after another; the chain collapses to `colour × keep + tint` per clone (two
  numbers kept while the effectors run), applied to every part and to the
  clones of an inner Cloner.
- **Randomness that never flickers.** Random values come from a hash of the
  seed and the clone's number, not from a running generator, so a clone keeps
  its values whatever else changes.
- **A name clash in the file format.** Lights already had a `FalloffDto`; the
  effector's is `EffectorFalloffDto`. Its serde default first gave a zero-size
  falloff when omitted — the round-trip test caught it; it now defaults to the
  effector's own default.
- **Linking in one undo step.** A new effector needs its id before it can join
  a Cloner's list, so `create_effector` applies the insert, then the lists,
  and records the inverses together (as wrapping in a generator does).
- **On-screen testing:** an 11 × 11 grid of cubes, a Random effector scattering
  and turning them and a Plain effector with a soft Sphere falloff lifting,
  enlarging and reddening those near it; the falloff's circles drawn; the path
  tracer rendered the colours.

Phase 10 is complete: deformers, the Cloner and effectors, with animated
parameters, are undoable, saved, picked, exported and rendered.

---

## Phase 11 — Simulation (2026-10-30 – )

| Step | What was built | Tests |
|------|----------------|-------|
| — | Phase 11 planned in detail: rigid and collider bodies as a Dynamics tag, `rapier3d` 0.36 in a new `grid4d-sim` crate (metres inside, arrays at its edge), a per-frame cache in editor state keyed by the simulation's inputs, evaluation overrides, clones as bodies, saved caches (`IMPLEMENTATION_PLAN.md`) | 658 |
| 11.1 | Rigid Body / Collider Body (one Dynamics tag: motion, shape, bounce, friction, mass, damping, start velocity) from the Tags menus, edited, saved; Simulation settings in the Project settings, undoable, saved; the `grid4d-sim` crate on `rapier3d` 0.36: a ball falls ½ g t² and rests on a collider, deterministically | 665 |
| 11.2 | The simulation core: bodies from tagged objects as the viewport shows them at the start frame (Compound groups, box / sphere / convex hull / mesh shapes, mass, bounce, friction, damping, start velocity), stepped in sub-steps with animated colliders following their keys, recorded per frame; not yet shown in the viewport (11.3) | 671 |
| 11.3 | The simulation in the viewport: simulated as the frame advances (a budget per refresh), cached and shown at once when revisited (a green bar on the timeline), dropped only when the motion's inputs change; Bake Simulation (progress, Esc) and Clear Simulation Cache; renders, Current State to Object and export show it | 677 |
| 11.4 | Clones as bodies: a Rigid / Collider Body tag on a Cloner makes every clone a body (a cloned group one piece), colliding with each other and with everything; clones start where the layout and effectors put them; shown, rendered, baked and exported where the simulation has them | 680 |
| 11.5 | Simulated frames saved with the document and shown at once when reopened (kept only if the bodies still start the same); Cache Memory limit with a status-bar warning; a benchmark of 1 000 falling cubes; the manual's Simulation chapter | 683 |

**Planning notes:**
- **The engine's maths library.** `rapier3d` 0.36 (the latest, 2026-10) uses
  glam 0.33 through `glamx`; we are on glam 0.34. Rather than hold glam back
  or write a solver, the physics lives in its own crate and talks to the rest
  through plain arrays (decision D10).
- **Keeping evaluation independent.** The simulation needs evaluated meshes,
  and evaluation must show simulated poses; a general override hook in
  `grid4d-eval` (world matrices by node or clone) keeps the arrow one way:
  eval ◀ sim ◀ ui.

**11.1 — Scene data and the physics crate**

*Problems and solutions:*
- **Two glam versions.** Rapier 0.36 builds on glam 0.33 (via `glamx`); ours
  is 0.34. The engine lives in `grid4d-sim` and every value crosses its edge
  as an array (`convert.rs`), converting centimetres to metres on the way; a
  test checks the round trip.
- **Less glue than expected.** Rapier 0.36 ships a `PhysicsWorld` holding the
  pipeline, sets and solvers, so the crate's `World` is a thin layer: bodies
  from a `Body` (dynamic or kinematic, restitution, friction, density or mass,
  start velocity), a frame of sub-steps, poses back in centimetres.
- **Presets of one tag kind.** Rigid Body and Collider Body are one Dynamics
  tag (an object has at most one of each kind), so `EditorCommand::AddTag`
  now carries the tag to add rather than its kind; the Object Manager's
  right-click menu passes menu entries by index, since its actions are `Copy`.
- **Clippy:** a helper placed after the test module was moved above it.
- **On-screen testing:** a sphere with a Rigid Body tag above a floor with a
  Collider Body tag: the tag icons in the Object Manager, the tag's settings
  in the Attribute Manager, the Project settings. (Scrolling to the Simulation
  section does not reach the app through posted messages; it is covered by
  the undo test.)

**11.2 — Simulation core**

*Problems and solutions:*
- **Scale.** Engine bodies have a position and a rotation only, so each
  object's world matrix is split: the scale goes into the collider points
  (every part mapped into the body's unscaled space), and the cache keeps the
  start scale to rebuild full matrices.
- **Custom Mass and Rotational Mass.** The engine takes a density or a mass
  per collider, but a body can have several parts and Rotational Mass has no
  setting. Each collider's mass properties are computed here instead: a
  Custom Mass is spread over the parts by their volume, and Rotational Mass
  divides the inverse inertia. A test checks a 100 cm cube at density 1
  weighs 1 kg and a custom 7 kg holds whatever the size.
- **Combining surfaces.** Bounce takes the higher of two bodies' values and
  friction their product (as the spec says), set as the engine's combine
  rules on every collider.
- **Animated colliders** are kinematic: before every sub-step they are moved
  to where their keys put them at that sub-step's end, so a paddle sweeping
  through a box pushes it along.
- **Speed:** the dev profile optimises dependencies, so the physics tests run
  in milliseconds.
- **On-screen testing:** nothing to see yet — the cache reaches the viewport
  in 11.3; the step is covered by its physics tests.

**11.3 — Playback, cache and renders**

*Problems and solutions:*
- **Showing poses without evaluation knowing about physics.** `grid4d-eval`
  got a general `Overrides` (world matrices by node) applied after animation
  and Target tags; an overridden node's change is carried to its subtree as a
  delta matrix, so a simulated object's children follow it. Setting the same
  overrides again costs nothing (no re-evaluation), so idle frames stay free.
- **When is the cache stale?** Comparing whole documents would drop it on
  every selection. The key holds only what can change the motion: the
  settings, the frame rate and start, and for each body its start world
  matrix, its subtree's data (names left out) and what above it animates or
  hides it. It is rebuilt only when the scene revision changes.
- **Simulating without freezing the UI.** No worker thread: the engine is
  fast, so the viewport advances the simulation within 8 ms per refresh
  (40 ms while baking) and asks for another refresh while frames are missing.
- **Renders on their own thread** cannot borrow the editor's simulation. They
  take a `Replay`: a copy of the cache plus the document, which simulates any
  missing frame itself — the same motion, since a run is deterministic. A test
  renders frame 40 from a cache holding only the start and finds the cube
  fallen out of view.
- **Baking simulated poses.** Current State to Object and export now take the
  overrides too, turning a simulated world matrix back into a transform under
  the object's (evaluated) parent.
- **On-screen testing:** five cubes and a ball dropped on a floor: scrubbing to
  frame 61 showed them settled with the cache bar to 61; Simulate ▸ Bake
  Simulation filled the bar to frame 90; scrubbing back to 37 showed the
  cached frame at once. (Ctrl+R cannot be sent by the test driver; the render
  path is covered by the test above.)

**11.4 — Clones as bodies**

*Problems and solutions:*
- **A clone has no node.** Clones are instances, rebuilt with every
  evaluation, so they cannot be "placed" like objects. A simulated clone is
  given its **change since the start** (`world(f) × world(start)⁻¹`), applied
  to all its parts after the instances are built — a cloned group keeps its
  arrangement and the layout is not stored twice.
- **The Cloner's bounds** (selection box, framing) were computed from the
  layout; they are made again from the moved clones whenever a clone moves.
- **One body per clone with several parts:** the body is framed by the
  clone's first part, and every part's shown mesh becomes a collider in that
  frame, so a group of a cube and a sphere tumbles as one piece.
- **What else a clone's start depends on:** a tagged Cloner's effectors and
  Object mode object are not below it, so the inputs key now also holds their
  start place and data; moving an effector re-simulates.
- **On-screen testing:** a tagged 5 × 3 × 5 Grid Array of cubes and spheres
  dropped onto a tilted ramp: baked to frame 120, the clones slid off the ramp
  and scattered on the floor; scrubbing showed the cached frames.

**11.5 — Saved caches and limits**

*Problems and solutions:*
- **Where a saved cache lives.** It is not part of the document's undoable
  data, so it travels beside it: `grid4d_io::save_with_cache` /
  `load_with_cache` take and give a `SimulationCache` (plain scene data, so
  `grid4d-io` needs no physics), while `save` / `load` stay as they were.
- **Never trusting a file.** The poses' entry is read against the size the
  description claims (frames × bodies × 28 bytes), capped at 2 GiB; a file
  claiming one frame more than it holds is refused (a test edits the claim).
- **Is a saved cache still right?** The engine's state cannot be saved, and a
  file edited elsewhere could hold frames of another scene. On opening, a
  fresh simulation of the document is built (frame 0 only) and the saved
  frames are kept only if they agree with it: same bodies, scales and start
  poses. Past the saved frames the fresh simulation runs on — the same motion,
  being deterministic (a test compares frame 50 both ways).
- **The memory limit** is a setting that changes no motion, so it is left out
  of the inputs key: raising it keeps what is cached.
- **The benchmark** uses `criterion` without its default features (no plots,
  no thread pool): building 1 000 bodies takes ≈ 2.3 ms, a frame of 1 000
  cubes falling into a box ≈ 4.6 ms with 5 sub-steps.
- **Clippy** asked for `is_multiple_of`.
- **On-screen testing:** five cubes and a ball baked and saved with File ▸
  Save: the file held `simulation.bin` (17 836 bytes: 91 frames × 7 bodies);
  reopened, the cache bar was full at once.

Phase 11 is complete: rigid and collider bodies on objects and clones,
simulated with rapier3d, cached, baked, saved, rendered and exported.

---

## Phase 12 — Workspace (2026-10-30 – )

| Step | What was built | Tests |
|------|----------------|-------|
| — | Phase 12 planned in detail: docking layouts on `egui_tiles` with Standard / Modeling / Animate presets, Preferences (units, undo depth, autosave, viewport defaults), Dark / Darker / Light themes, Layers, Takes (the document holds the active take; switching is an undoable swap; Auto Take), the interface in Greek (`tr` with English keys) (`IMPLEMENTATION_PLAN.md`) | 683 |
| 12.1 | Docking layouts on `egui_tiles`: every manager and the viewport a pane that docks as a tab or a split, closes and reopens from the Window menu; Standard / Modeling / Animate layouts, Reset Layout; the arrangement remembered between runs | 689 |
| 12.2 | Preferences (Edit ▸ Preferences…): the length unit for every length field and the grid HUD (typed units converted), Undo Depth, Autosave with crash recovery, view defaults; saved between runs | 697 |
| 12.3 | Dark, Darker and Light themes (Preferences ▸ Interface ▸ Theme) for every panel, menu, window and icon; the 3D view keeps its own colours; a contrast test for every theme | 699 |
| 12.4 | Layers: a coloured, named table in the document with Solo, View, Render, Manager, Locked, Animation, Generators and Deformers switches; objects on a layer (children inherit it); the Layer Manager, layer chips in the Object Manager, a Layer field; saved and merged | 708 |
| 12.5 | Takes: variations of the document (transform, visibility, parameters, tags, display colour, a camera, render settings) recorded automatically in the active take, switched as undo steps, rendered by choice; the Take Manager; saved | 713 |
| 12.6 | The interface in Greek (Preferences ▸ Interface ▸ Language; English by default): 1 012 texts through `tr`, switched at once; a test that no shown text lacks Greek | 716 |

**Planning notes:**
- **The docking library.** The plan named `egui_dock`, to be checked against
  egui 0.37 first: its newest release (0.21.1) still needs egui 0.36. Rather
  than write a splitter tree, Phase 12 uses `egui_tiles` 0.18, released for
  egui 0.37 by egui's own authors, which also serialises its layout
  (decision D13).
- **Takes without take-aware panels.** Keeping the active take's state in the
  document and making a switch one undoable swap means the panels, the
  evaluation and the path tracer need no changes; only edits (recorded as
  overrides while a take is active) and renders (which apply a take to their
  copy) know about takes (decision D14).

**12.1 — Docking layouts**

*Problems and solutions:*
- **Panels that showed their own tabs.** The Object, Attribute and Coordinate
  Managers and the timeline, F-Curve editor and Material Manager each drew a
  tab strip and kept which tab was in front in editor state
  (`ManagerTab`, `BottomPanel`). Those strips are gone: each panel now draws
  only its contents and the tile tree owns the tabs; the F-Curve editor kept
  its header row (`widgets::header_row`).
- **Keys meant for a manager.** Delete, Ctrl+N, H, S and Tab asked which
  bottom tab or manager was in front. They now ask `EditorState::panes`: the
  panes drawn this frame (`is_shown`) and the one under the pointer
  (`hovered`), both marked by the tile `Behavior` while it draws them.
  Shortcuts that show a manager set a request (`panes.show`) that the
  workspace handles at the start of the next frame.
- **Where a reopened pane goes.** Each pane names the panes it usually sits
  with (`Pane::neighbours`); a reopened pane joins the first of their groups
  that is still open, or wraps the whole tree in a split when none is.
- **A saved tree from elsewhere.** A saved arrangement may hold a pane twice
  or nothing at all (another version); `Workspace::repaired` gives the preset
  back then, and adds the Viewport if it is missing. A first version also
  added every missing pane — which would have reopened closed managers on
  every start; closed panes now stay closed (a test saves and loads one).
- **Name clashes.** `egui::Layout` and the workspace's `Layout` meet in the
  menu bar (imported as `PaneLayout`); `Tree::new` needs a stable `Id`.
- **Clicks on tabs in the on-screen driver.** Posted mouse messages open menus
  but never reached the tabs (which sense both clicks and drags), so a test
  clicks them instead: it finds the Coordinates tab and its × through egui's
  accessibility tree and sends real pointer events — the tab comes to the
  front, the × closes it.
- **On-screen testing:** Standard, Modeling and Animate switched from the
  Layout menu; the Window menu highlights the panes that are showing; after a
  restart the Modeling arrangement came back as it was left.

**12.2 — Preferences**

*Problems and solutions:*
- **Units without threading them through every panel.** Lengths are edited in
  dozens of row helpers that never see the editor state. The unit is put on
  the egui context each frame (`preferences::set_units`) and read by
  `widgets::length`, a `DragValue` over the stored centimetres with a custom
  formatter and parser; tests that never set it see centimetres. Ranges and
  drag speeds stay in centimetres, so no caller changed beyond the widget.
- **Decimals.** A centimetre field with two decimals would show metres to the
  centimetre only; each unit adds the decimals it needs (metres and feet two,
  inches one) and trailing zeros are dropped, so what a field shows reads back
  as the same length (a round-trip test over every unit).
- **Whose recovery copy is it?** A copy found at startup could belong to
  another Grid4D still running. Each session holds a lock on its own
  `.lock` file (`File::try_lock`, in std since Rust 1.89); the system releases
  it when the process ends however it ends, so a copy whose lock can be taken
  is a crashed session's. A recovered copy becomes this session's own (renamed
  and locked), so it survives a second crash before the next autosave.
- **Writing without a hitch.** The copy is written by a thread from a clone of
  the document (cheap: meshes are shared handles), to a `.part` name renamed
  when complete. The app asks to be woken when the next copy is due, so an
  idle window still autosaves.
- **The undo depth** changes outside commands, so `commands::apply_pending`
  compares it with the history's limit each frame and trims at once, purging
  deleted objects only the dropped steps could restore.
- **Clippy** asked for a by-value `ShadingMode` in the serde helper, whose
  signature serde fixes (an `expect` with the reason).
- **On-screen testing:** Edit ▸ Preferences… opened the window; choosing
  Metres turned the Cube's P.Y into "1 m" and the HUD into "Grid Spacing : 1
  m"; the grid switched off with Apply to Views and stayed off after a restart.
  With Autosave at one minute, a new cube left a copy after a minute; the app
  was then ended abruptly, and on the next start *Recover Unsaved Work?* came
  up; Recover brought the document back with the extra cube and an asterisk
  in the title.

**12.3 — Themes**

*Problems and solutions:*
- **Constants everywhere.** About 270 uses of `theme::palette::NAME` in 26
  files read compile-time constants. They became fields of a `Palette` value
  returned by `theme::palette()`, which reads the theme from an atomic (no
  lock, no allocation, so hot paths are unaffected); the rewrite was a single
  mechanical substitution (`p::TEXT_DIM` → `p().text_dim`). One binding named
  `p` in the F-Curve editor shadowed the function and was renamed.
- **The view in a light theme.** The view's background stays a darker grey (as
  in the reference), so dark text or light-theme accents drawn over it would
  vanish. What is painted over the view — the HUD, the axis triad, the
  gizmo, spline and deformer handles, the Pen, snapping marks and the
  component colours — reads `theme::view_palette()`: the Dark palette in
  Light, the Darker palette in Darker.
- **Contrast.** A test computes WCAG contrast for every theme. It caught the
  Light theme's orange highlight at 2.9 : 1 against the headers; it was
  darkened to pass 3 : 1.
- **egui's own theme.** Light installs its style in egui's light slot
  (`dark_mode` off), so egui's built-in widgets (combo arrows, check marks,
  text cursors) pick light-theme defaults too.
- **On-screen testing:** Preferences ▸ Theme switched to Light at once
  (panels, menus, icons; the view unchanged), it was still Light after a
  restart, and Darker deepened every panel and the view.

**12.4 — Layers**

*Problems and solutions:*
- **Inheritance without walking up for every object.** The evaluation walk
  already keeps a stack of inherited materials; layers got the same stack, so
  each object's effective layer costs nothing extra. Code that looks at one
  object on its own (deformers, the gizmo, the Object Manager) asks
  `Scene::effective_layer`, which walks up the hierarchy — but returns at once
  while the document has no layers.
- **Inside generators.** A generator's inputs are read by their own walks,
  which only reach objects under a shown generator: there an object without
  a layer of its own shows like its parent, so only its own layer needs a
  check (`Layers::shows_own`).
- **Animation off everywhere.** World matrices are computed in two places —
  the evaluation's animation pass and `Scene::world_matrix_at`, which the
  gizmo, deformers and generator inputs use. Both read the document's own
  transform for objects on a layer whose Animation is off, so what is drawn
  and where the gizmo sits agree.
- **Locked.** Rather than teach every tool about locking, a locked layer's
  objects get no pick shape (so clicks and rectangle selection pass them by;
  they can still be snapped to), and `Scene::protection` reports every channel protected, so the
  gizmo and the Coordinate Manager leave them alone. The Object Manager still
  selects them.
- **Dragging between panels.** The Object Manager keeps its drag to itself;
  it now reports it as `EditorState::object_drag`, set at the start of each
  frame, so the Layer Manager sees it on the frame the button is released,
  whichever pane is drawn first.
- **Saving a broken colour.** The round-trip test found that a non-finite
  layer colour was written without complaint (only reading refused it); saving
  now refuses it too, as for materials.
- **Layout.** The first Layer Manager placed its columns with `add_sized`,
  which centred names and let the switch columns drift; rows now place the
  chip, the name and each switch at fixed positions from the right edge.
- **On-screen testing:** a scene with three layers (Set locked, Props, Proxies
  hidden): the Proxy Box was hidden and its chip shown; clicking V showed it;
  S on Props left only the crate and the ball; clicking the locked floor
  selected nothing, clicking the cone selected it. Dragging rows cannot be
  driven by posted mouse messages, so the drop onto a layer is tested in
  code.

**12.5 — Takes**

*Problems and solutions:*
- **Panels that know nothing about takes.** The document always holds the
  active take's values (decision D14); Main's values for what that take
  overrides wait beside it in `Takes`. Switching is one swap, so every panel,
  the evaluation, the simulation and the path tracer work unchanged.
- **Recording without touching every tool.** All undo steps pass through
  `History::record`, so Auto Take lives there: it compares each step's
  inverses (`SetData`, `Move`, `SetRender`) with the document and writes what
  changed into the active take, as one more edit of the same step. Edits of
  the takes themselves (clearing an override) use `record_as_is`, or clearing
  would be recorded as a new override.
- **An exact undo for switching.** Storing the document's values back into
  the take when leaving it (as first written) would let switching away and
  back change the take, so switching is a pure swap — the take already holds
  those values, since edits are recorded as they happen — and the random
  undo/redo test records in the take like the editor does.
- **Objects deleted while overridden.** A deleted object is only detached
  (undo can bring it back); the swap writes into detached objects too, so
  Main's values are not lost while it is away. The random test caught a
  second case: Main's parameters of an object whose kind had changed were
  skipped and so lost; Auto Take now drops parameter overrides when an
  object becomes another kind (Make Editable), and the swap always writes.
- **Rendering another take.** The render job gets a copy of the document
  switched to that take — the active take stays as it is; a take's camera is
  the one the render looks through.
- **On-screen testing:** New Take made a take active; unticking Visible in
  Editor in the Basic tab hid the cube and the Take Manager listed
  "Cube · Visibility"; clicking Main showed the cube, clicking the take hid it
  again.

**12.6 — Greek**

*Problems and solutions:*
- **Finding every text.** About a thousand texts lived in 40 files. Scripts
  wrapped the literals passed straight to egui widgets in `tr(…)`, enum labels
  where they are shown, and the `format!` texts became `trf` with numbered
  placeholders (Greek word order differs: "{0} Object" became
  "Αντικείμενο {0}"). The label-drawing helpers translate their argument
  themselves, which keeps the Attribute Manager's key buttons on their
  English row names. Texts in constants and `match` arms are marked with
  `key(…)` (a `const fn`) and translated where they are shown.
- **A test instead of a list.** A scanner reads the sources and gathers what
  the interface shows: `tr`/`trf`/`key` literals, the first literal of the
  label helpers, undo step and alert names, and every literal in the bodies
  of `fn label`, `fn hint`, `fn title` (in this crate and the crates whose
  labels it shows). It fails on any text without Greek and on any Greek no
  code uses. Two traps it met: files with an early `#[cfg(test)]` item (cut
  at the test module instead) and Rust's `\` line continuation inside a
  string (the key is the joined text).
- **A check that compared English text.** Saving a failed render looked for
  a note starting with "Not saved"; with Greek notes it never would. It now
  compares with the translated prefix.
- **Longer labels.** Greek labels are often longer than the 128-point label
  column; labels are now clipped to their column and show in full on hover,
  and the Preferences window is wider.
- **Tests never switch the language.** The language is one atomic for the
  whole process, so a test switching it could change another test's texts
  running at the same time; the tests check the table directly instead.
- **On-screen testing:** Preferences ▸ Language ▸ Ελληνικά switched the menus,
  managers, tabs, tooltips and status bar at once; Greek came back after a
  restart; the Basic tab's long labels were clipped neatly.

Phase 12 is complete: docking layouts, Preferences, themes, layers, takes and
the interface in Greek.

---

## Phase 13 — Extensibility (2026-10-30 – )

| Step | What was built | Tests |
|------|----------------|-------|
| — | Phase 13 planned in detail: plugins as Rust crates built in and registered from a list (`grid4d-plugin`: parameter schema, generator, deformer and command traits; `ObjectKind::Plugin`; missing plugins kept), scripting in Rhai through `SceneEdit`s (one undo step per run, an operations limit), the Script Manager, SDK documentation (`IMPLEMENTATION_PLAN.md`) | 716 |
| 13.1 | Plugins: the `grid4d-plugin` API (parameter descriptions, generator plugins, the registry), plugin objects in documents (kept when their plugin is missing), Create ▸ Plugins, parameters in the Attribute Manager, the example Gear | 726 |
| 13.2 | Deformer plugins (in the deformer chain, their box, a handle, Shift-create under the selection, Fit to Parent) and keyed plugin numbers (key buttons, F-Curves, files, takes); the example Wave | 732 |
| 13.3 | Command plugins: the Plugins menu, an options dialog from the plugin's description, changes applied as one undo step (or the plugin's message); the example Scatter | 737 |
| 13.4 | Scripting core: Rhai scripts on a copy of the document (objects, placement, parameters, hierarchy, materials and layers, plugin commands), one undo step, errors with line and column, an operations limit; Script ▸ Run Script File… | 743 |
| 13.5 | The Script Manager: the script library with five examples, the editor with line numbers, Run and Ctrl+Enter, the console; Script ▸ lists the library | 749 |
| 13.6 | Guides for plugin and script writers (`plugins.md`, `scripting.md`, its examples tested), the manual's Plugins and Scripts chapters, the Greek checked on screen, the phase's acceptance; Phase 13 complete | 751 |

**Planning notes:**
- **Why plugins are built in.** Loading plugin libraries at run time would
  need `unsafe` (forbidden in the workspace) and a stable ABI Rust does not
  have; plugins are crates the binary lists, and the plugin API is a crate of
  its own, so a plugin depends on nothing else of Grid4D (decision D16).
- **Why Rhai, not Python.** Rhai 1.27 (released 2026-10-21) is pure Rust,
  sandboxed and needs nothing installed; Python through `pyo3` 0.30 would
  need a Python runtime found or bundled on every platform and a more complex
  build (decision D17).
- **Plugins without plugin awareness.** Plugin objects are one more
  `ObjectKind` with a type id and values, so undo, takes, layers, animation,
  files and the path tracer handle them like any object; only evaluation and
  the Attribute Manager ask the registry.

**Fix (12.2's autosave, found by CI on Linux and macOS):** a recovery
session was named after the process and the millisecond it started; two
sessions made within one millisecond (as the recovery test does, quickly on
fast runners) got the same name, so discarding treated the other's copy as
its own and left it. Names now also count the sessions of the process; a
test makes fifty at once.

**13.1 — Plugin API and generator plugins**

*Problems and solutions:*
- **One more kind, no plugin awareness.** `ObjectKind::Plugin` holds only a
  type id and values by name, so undo, takes, layers, Merge and the file
  format needed no plugin knowledge; the exhaustive matches pointed at the
  ten places that had to say what a plugin object is (none, a group, or "a
  generator" for the view filter).
- **Where the registry lives.** Evaluation runs in library code that the
  path tracer, exports and tests call, so the plugins are installed once
  into a `OnceLock` by the binary (`src/plugins.rs`), and each test binary
  installs the Gear once through a `std::sync::Once`.
- **Values by name, checked when read.** Documents keep values as they
  were set; `Values` gives the plugin each one in range, or its default
  when it is missing or of another type, so plugins can change their
  parameters without breaking old files — and values a plugin does not know
  survive a save.
- **The gear's faces.** The outline is built counter-clockwise seen from +Y;
  a test checks with Newell's normals that every wall, cap and the hole face
  outwards, with and without a hole (a fan around a centre point without).
- **Translating plugins' texts.** Plugin names, descriptions, parameter
  names, hints and choices are English keys too; the translation scanner
  now reads the `plugins/` crates.
- **On screen:** a long "missing plugin" sentence widened the Attribute
  Manager's label column and pushed the values out of sight; it became a
  "Missing Plugin" row with the type id, the explanation in its tooltip.
- **On-screen testing:** Create ▸ Plugins ▸ Gear made a shaded gear with its
  hole and twelve teeth, titled "Gear Object", its parameters in centimetres
  and percent; a document naming a plugin Grid4D lacks opened with the
  object's values listed under "Missing Plugin".

**13.2 — Deformer plugins and animated plugin parameters**

*Problems and solutions:*
- **A deformer the scene cannot recognise.** `ObjectKind::is_deformer` lives
  in `grid4d-scene`, which knows nothing of plugins, so evaluation and the
  interface ask `grid4d_eval::is_deformer` (built-in or an installed
  deformer plugin) wherever deformers are treated apart: the box curve,
  bounds, cloned sources, Current State to Object and the handle.
- **One value lookup per point.** A plugin reads its values by name; doing
  that for every point would cost a search per point. `deform` takes the
  whole list of points, and the chain now runs link by link over all the
  points (the same result as point by point through every link).
- **Animating values the scene does not describe.** `ObjectParam::Plugin(i)`
  names a value by its place in the object's list, so the scene reads and
  writes it with no plugin knowledge; labels come from the value's name
  (`ObjectKind::param_label`) and units from the plugin's description where
  the interface shows them. Files keep tracks by name and find the place
  again when loading. `ParamSet` holds 128 bits: values past the first 64
  are not keyed, and its bit is now checked so that no index can overflow.
- **`impl dyn DeformerPlugin`** means `+ 'static`; the registry hands out
  borrowed plugins, so the helpers are on `dyn DeformerPlugin + '_`.
- **Translations:** the Wave's directions were in a constant the
  translation scanner does not read; they are written in the `Choice(&[…])`
  itself, as the scanner expects.
- **On screen:** the Object tab's title said "Plugin Object" for the Wave
  (the name was looked up among generators only); it now says "Wave Object".
- **On-screen testing:** a plane with a Wave under it (Radial, Phase keyed)
  showed the ripples, the wavy box and the handle; the Object tab showed
  key buttons on the numbers (Phase keyed), the F-Curve editor a "Phase"
  track; Create ▸ Plugins listed Gear, a line, Wave. Shift + click and the
  handle drag cannot be posted to the window; tests cover them.

**13.3 — Command plugins**

*Problems and solutions:*
- **New objects have no ids yet.** A command returns `Changes`, not
  `SceneEdit`s alone: an insert's parent can be the root, an existing object
  or a `NewObject` — one the command added earlier, by its order. Grid4D
  applies the list one change at a time (ids appear as it goes), takes all
  of it back if any change is refused, and records the inverses as one step
  (Auto Take included, through `History::record`).
- **Shapes without evaluation.** `grid4d-plugin` cannot depend on
  `grid4d-eval` (evaluation depends on it), so a command's `Context` carries
  a function from the interface that gives an object's shape — a polygon
  object's mesh, a generator's result, a primitive's or a plugin's mesh.
- **One set of fields.** The options dialog and the Object tab draw a
  plugin's parameters the same way: the fields moved into
  `widgets::plugin_value` (with `colour_button`, which both use).
- **Translating messages.** A command's message is shown translated; the
  scanner reads the strings inside `Err(…)` in plugin crates, so messages are
  written there rather than in constants.
- **On-screen testing:** a small cube and a sphere, Select All (the sphere
  active), Plugins ▸ Scatter… showed its options (Count, Seed, Align to
  Surface, Random Rotation 180°, Scale Variation); OK made a "Scatter" Null
  with 20 copies standing on the sphere, selected. With only the sphere
  selected it said "Select the objects to scatter, then the surface last"
  and the document stayed unchanged. Typing into the dialog's fields cannot
  be posted to the window; tests set options directly.

**13.4 — Scripting core**

*Problems and solutions:*
- **All or nothing without recording forward edits.** A script works on a
  clone of the document (meshes are shared handles, so the clone is cheap)
  through `Document::apply`, keeping the inverses. When it finishes, the
  clone *is* the new document and the inverses, reversed, are the undo
  step; on an error the clone is dropped and the user's document was never
  touched. The ids the script made are the clone's, so nothing has to be
  mapped back.
- **Rhai's functions must own what they use.** The registered closures are
  `'static`, so the copy lives in an `Rc<RefCell<State>>` each closure
  holds, and the shapes for plugin commands are a plain function pointer
  (`commands::shape_of`) evaluated on the copy's scene.
- **One way to apply a command's changes.** Scripts run plugin commands too,
  so the all-or-nothing application of `Changes` moved from the interface
  into `grid4d_plugin::Changes::apply` (with `Applied`: inverses, new
  objects, top-level ones).
- **Errors where they happened.** Rhai fills in the call's position for
  errors raised by our functions; `take_position` separates it from the
  message, shown as "Line 3, column 9: …".
- **An endless loop** costs 100 million operations before it is stopped:
  about three seconds in a dev build; the document is unchanged.
- **On-screen testing:** Script ▸ Run Script File… opened the system's file
  dialog (filled in by typing); a 5 × 5 grid script on an empty document
  made the Grid Null with 25 coloured cubes, selected, and said "made 25
  cubes"; one Undo removed all of it and the title lost its unsaved mark.

**13.5 — The Script Manager**

*Problems and solutions:*
- **Panels never touch files.** The window edits the script in memory and
  pushes commands (New, Open, Save, Run); `ui/src/scripts.rs` is the only
  code that reads and writes the library, and the menu lists the names it
  keeps (refreshed when the library is made or saved to), so no frame reads
  the folder.
- **Ctrl+Enter, not a new line.** The editor would take the Enter first, so
  the window consumes Ctrl+Enter before drawing the editor, and only while
  the editor has the keyboard.
- **Where output goes.** With the window open, output and errors of every
  run (also Script ▸ a script) go to its console; a message would hide the
  window. With it closed, a message as in 13.4.
- **A Rhai rule met in testing:** a property cannot be set on a call's
  result (`find("Cube").name = …` is a syntax error); the manual says to
  keep the object in a variable first.
- **Examples are tested:** every built-in example runs on the demo scene
  (the Sphere and the Cube selected) without error, changes something and
  prints one line.
- **A backslash in the Greek table** (the characters a name may not
  contain) has to be escaped in the Rust literal; written by hand.
- **On-screen testing:** the first start made the library with the five
  examples, listed in Script ▸; the Script Manager opened with the hint
  script and line numbers; Open ▸ Grid of Cubes, Run made the grid
  ("made 25 cubes" in the console); Random Colours and Spread Selected
  Objects ran on the selected Grid and wrote their lines below.

**13.6 — SDK documentation and finish**

*Problems and solutions:*
- **Guides that stay true.** Every ```` ```rhai ```` block of `scripting.md`
  is run by a test on the demo scene (the Sphere and the Cube selected), and
  the test checks that the five built-in examples appear in the guide word
  for word — an API change that breaks an example fails the build. Blocks
  meant to show errors are marked `text`.
- **Bugs found by testing in Greek:** on a deformer's Object tab (built-in or
  plugin) the key buttons were missing in Greek: the Fit to Parent hint, much
  longer in Greek, sat in the grid's middle column and widened it until the
  key column fell off the panel. The hint now wraps at about a field and a
  half.
- **The phase's acceptance** is a test for the Gear (edit and undo, keys on
  its Radius, a take's Thickness, save and reload with the keys found by
  name, saved back to the same bytes) beside the earlier tests for the Wave,
  Scatter and scripts; the missing-plugin case was checked on screen in 13.1
  and the file layer never asks for plugins.
- **On-screen testing in Greek:** the Wave's Object tab (Πλάτος κύματος,
  Μήκος κύματος, Φάση, Κατεύθυνση: Ακτινικά, key buttons), Script ▸ with
  Διαχείριση scripts… and Εκτέλεση αρχείου script…, and the Script Manager
  (Νέο, Άνοιγμα, Αποθήκευση, Εκτέλεση, Κονσόλα) all read in Greek.

**Phase 13 complete.** Plugins are Rust crates built into Grid4D behind the
`grid4d-plugin` API: generators, deformers (in the deformer chain, with a
box and a handle) and commands (changes as one undo step), their numbers
animated and overridden in takes, documents with missing plugins kept
intact; the Gear, Wave and Scatter examples. Scripts in Rhai run on a copy
of the document as one undo step, with errors at their line, a limit for
endless loops, a library with examples and the Script Manager. Guides:
`plugins.md`, `scripting.md`.

---

## Phase 14 — Stabilisation (2026-11-01 – )

| Step | What was built | Tests |
|------|----------------|-------|
| — | Phase 14 planned in detail: CI on Node 24 with `cargo deny` and third-party licences; the `.g4d` format reviewed, frozen and specified, property-tested and fuzzed; crash reports and a random-command test; the performance budget benchmarked and measured; `egui_kittest` UI tests and a walkthrough in English and Greek; packages for Windows, macOS and Linux from a tagged release; the 1.0 documentation (`IMPLEMENTATION_PLAN.md`) | 751 |
| 14.1 | CI on Node 24; `cargo deny` (licences, advisories, duplicates, sources) and the third-party licence list checked in CI; Help ▸ About shows the version, commit and date and a Licences page; version 0.13.0; dependencies updated | 751 |
| 14.2 | `.g4d` v3 reviewed and frozen, specified in `docs/FILE_FORMAT.md`; a golden fixture with everything v3 holds; property-tested round trips; damaged and hostile files; a fuzz target with a CI job and a weekly run; saved rotations kept exact and never NaN | 756 |
| 14.3 | Text outlines read with `skrifa` (egui's own font reader) instead of the unmaintained `ttf-parser`, identical to the byte; local crash reports (a panic hook, the log's last lines, pointed to on the next start, Show Report); the random-command test (long `EditorCommand` sequences with undo / redo); start-up without graphics or a data folder told in words; autosave's first failure an alert; failed saves keep the old file; files too large for memory refused | 767 |
| 14.4 | The performance budget measured and met (`docs/performance.md`): `criterion` benches for geometry, evaluation, extraction, files and the path tracer (built in CI); a counting-allocator test of steady frames; the stress scenes (`examples/stress.rs`); View ▸ Frame Time; topology 10× and the BVH build a fifth faster | 770 |
| 14.5 | The interface pass: the whole interface as one `Shell` driven headless by `egui_kittest` (Shift / Ctrl selection, drag to re-parent, rename, a typed value, a gizmo drag, a Bend handle drag, a menu command, each with undo); one shortcut list for Help and the manual, kept equal by a test; accessible names for fields and Object Manager rows; the interface rendered in English and Greek, Dark and Light, and fixed where it read wrong | 778 |
| 14.7 | Documentation and 1.0: a new README (download table, what Grid4D does, fresh pictures of the workspace and a path-traced render, made headless from a showcase scene), `CHANGELOG.md` with 1.0.0 and the road there by version, CONTRIBUTING (where things go, how releases are made), the names-and-wording review (R2), the manual's last stale notes; version 1.0.0 | 778 |
| 14.6 | Packaging and the release workflow: version 1.0.0-rc.1; icons rendered from the SVG masters (PNG 16 – 1024, `.icns`), the macOS `Info.plist`, the Linux `.desktop` and MIME type, the WiX installer source; `release.yml` (three builds; `.zip`, `.msi`, universal `.dmg`, `.tar.gz`, AppImage; SHA-256 sums; a draft release with the `CHANGELOG.md` section; itch.io with butler when its key exists; a dry run by hand); the Windows `.zip` built and started here | 778 |
| — | The itch.io page prepared ([`assets/itch/page.md`](../assets/itch/page.md): its fields, description and pictures); step 14.6 also pushes the packages there with `butler` | — |

**Planning notes:**
- **What "stable" means here.** No new features: the format frozen and
  written down, inputs that cannot crash the program, work that a crash
  cannot lose, the budget measured rather than assumed, licences in order,
  and packages anyone can install.
- **Why the format is reviewed before it is frozen.** Version 3 grew one
  step at a time for eleven phases; once 1.0 ships, its shape is a promise.
  The review may produce a v4 with a migration — cheaper now than a
  compatibility burden later.
- **Why our own release workflow.** The app bundle, the installer's file
  association and the AppImage need hand-made pieces anyway; a plain
  workflow keeps every step visible (D19).
- **Unsigned for 1.0** (R4): signing needs paid certificates and an Apple
  developer account; the README will explain the warnings.

**After 1.0, planned (2026-11-02):** the plan's new section *After 1.0*
compares Grid4D 1.0 with what artists expect from the established suites and
lays out ten phases, v1.1 – v2.0: materials, UVs and textures (Phase 15,
planned in detail, with `.g4d` v4); render quality; modelling 2; fields and
motion graphics 2; sculpting and painting; characters and animation 2;
simulation 2; nodes; GPU rendering; the pipeline. Decisions D24 – D30 and
risks R5 – R8 go with it.

**14.1 — CI and supply chain**

*Problems and solutions:*
- **Licences outside the list:** the font egui embeds is `Ubuntu-font-1.0`
  (with `OFL-1.1`), allowed; Rhai's `smartstring` is MPL-2.0, a file-level
  copyleft that is fine for a crate used unmodified — allowed for that crate
  only.
- **An unmaintained dependency:** `ttf-parser`, which grid4d-text uses for
  Text outlines, was declared unmaintained (RUSTSEC-2026-0192; no
  vulnerability). Ignored with a reason in `deny.toml`; moving to `skrifa`
  is the follow-up. A yanked `clipboard-win` went with `cargo update`.
- **`cargo-about` without a command:** its binary needs the `cli` feature
  (`cargo install cargo-about --features cli`); CI installs a prebuilt one.
  Its template escaped text as HTML (`&quot;`) until the fields were written
  with triple braces, and some crates' licence files have CR line ends — the
  generation pipes through `tr -d '\r'`, locally and in CI alike, so the
  committed file can be compared byte for byte.
- **A deterministic build identity:** the commit's date, not the build's,
  so two builds of one commit are identical; without git both are empty and
  About shows the version alone.
- **The Licences page:** 365 KB of text shown a row at a time
  (`ScrollArea::show_rows`); wrapped lines broke the row height and widened
  the window, so lines are not wrapped and scroll sideways; the row height
  given to `show_rows` excludes the spacing it adds itself.
- **On-screen testing:** Help ▸ Keyboard Shortcuts & About showed "Version
  0.13.0 (f030cc8, 2026-11-01)", the two pages, and the licence list
  scrolling inside the window.

**14.2 — File format freeze**

*Problems and solutions:*
- **Freeze v3 or write v4?** The review went through every DTO: later
  additions are all optional with defaults, references are indices, units
  are consistent (radians; the two degree fields say `_deg`). Nothing had to
  change, so v3 is frozen as it is — no migration to write.
- **The golden file covered little.** `demo_v3.g4d` is the demo scene with
  one mesh; most of what eleven phases added (splines, deformers, Cloners,
  effectors, plugins, tracks, layers, takes, a saved simulation) was only
  tested by round trips that would accept a silent format change. A second
  fixture, `full_v3.g4d`, pins all of it byte for byte.
- **Bugs found by the new tests:** re-saving a loaded file changed
  `simulation.bin` — loading normalised every quaternion, which moves the
  last bit of ones that were already unit length; and a zero quaternion in a
  file became NaNs. Rotations are now kept exactly when they are unit length,
  normalised when they are off, and the identity when they have no length.
- **The property test's own mistake:** the generator could give a plugin
  object two tracks for one value — a document the application cannot make
  (loading keeps one); the generator was fixed, not the loader.
- **Fuzzing needs nightly** and runs best on Linux, so `cargo fuzz` runs in
  CI (two minutes on every push, an hour weekly or as long as asked by hand)
  and a stable test does the same kind of damage locally on every platform.
- **The disk filled up** while building the release tests: `target/` had
  grown to about 95 GB (57 GB of incremental cache, 36 GB of old test
  binaries). The incremental cache and the half-built release folder were
  deleted (rebuilt as needed); `cargo clean` now and then keeps it in check.

**14.3 — Robustness**

*Problems and solutions:*
- **Replacing `ttf-parser`.** `skrifa` 0.44 was already in the build (egui
  reads its fonts with it), so the change adds no crate. Its metrics follow
  the same rule as before (OS/2 typo metrics when the font asks for them,
  else `hhea`) and its pen gets the same moves, lines and curves: the
  outlines of a set of test strings (Latin, Greek, accents, symbols, several
  lines) were dumped with both libraries and compared — identical byte for
  byte. `ttf-parser` remains in `Cargo.lock` only through winit's Wayland
  decorations on Linux, which `cargo deny` does not count as ours, so the
  advisory needs no exception any more.
- **A panic hook that must not fail.** It runs while the program is already
  in trouble: it never waits for a lock (the panicking thread may hold the
  log's), never panics itself, and writes the report before the default
  hook prints the message. It is tested in an integration test of its own,
  since a hook is installed for the whole process.
- **The log's last lines** need the logger to keep them: `env_logger` is
  built rather than installed and wrapped by a logger that keeps 200 lines.
- **A UTC time without a date crate** — the report's name and header use
  the civil-from-days conversion, checked against known dates (a leap day,
  2100).
- **Messages before the language is known.** The data folder is checked
  before the window opens, when the preferences (and the language) are not
  loaded yet; the check returns the problem, and the app turns it into a
  translated alert on its first frame.
- **No GPU.** eframe's wgpu start-up already falls back to a software
  adapter when it is the only one (WARP on Windows); with none at all
  `run_native` returns an error and the window never opens — now a system
  message box says why. Checked by starting with `WGPU_BACKEND` set to a
  backend the machine lacks (see below).
- **Found on screen: the message box closed itself.** Started with
  `WGPU_BACKEND=metal` (a backend Windows lacks), Grid4D logged the message
  but no box appeared: the failed event loop leaves a quit message in the
  main thread's queue, and `MessageBoxW` returns at once when it sees one.
  The box is now shown from a thread of its own, and stays until OK.
- **The fuzz job failed in CI** (the first run after 14.2): cargo-fuzz
  built for `x86_64-unknown-linux-musl`, which has no standard library on
  the runner and does not allow the address sanitizer. The prebuilt
  cargo-fuzz that the install action fetches is a static musl binary, and
  cargo-fuzz defaults to the target it was itself built for. Both fuzz
  workflows now pass `--target x86_64-unknown-linux-gnu`.
- **The random-command test** found no bug in 120 000 commands (300 seeds
  of 400, frame changes and component modes included) — so it was checked
  against a deliberate one: a command that renamed the document without an
  undo step was caught within 32 commands. Its first runs found two
  mistakes of its own: a selected tag may outlive its object by design
  (undo brings both back; it is only read through `selected_tag`), and a
  sequence ending in undos has its end further on, after redoing.
  Coverage was checked by counting the undo labels it produced: almost
  every command that changes the document, including Extrude, Bevel,
  Bridge, Weld, Loop and Line Cut.

**14.4 — Performance budget**

*Problems and solutions:*
- **A counting allocator without `unsafe`.** Counting allocations needs a
  global allocator, which Rust only lets one write with `unsafe` — forbidden
  in the workspace. `allocation-counter` (MIT / Apache-2.0, no dependencies)
  brings its own; as a dev-dependency used by one integration test, it is
  linked into that test binary alone.
- **A test that passed for the wrong reason.** The first scenes for the
  allocation test and the benches set "animated" values with
  `NodeData::set_value_at`, which only writes a key on a parameter that
  already has a track — so nothing was animated, and "evaluating the next
  frame" measured 12 ns. Noticed from that number; the scenes now push
  tracks, and the test checks that the objects really move between frames.
  With real animation it still allocates nothing.
- **Topology was the slowest thing found** (580 ms for a million triangles,
  on entering a component mode): two hash maps over every corner. Rewritten
  with a counting sort by each edge's lower point: 57 ms, and a test
  compares it with the hashing version on every test shape and on random
  non-manifold meshes (same edges, same order, same orientation verdict).
- **Measuring on screen without a mouse.** Neither posted nor real mouse
  clicks reached the window from the test driver this time (keys did), and
  posted Ctrl combinations arrive without Ctrl. The measurements used keys
  only (F8 to play) and a local build — not committed — with the readout on
  from the start and an environment variable choosing a component mode;
  Select All was timed headless instead, in an ignored test that draws
  every panel.
- **The readout's average hid hitches**, so it also shows the worst frame of
  the last 128 (a fixed ring, nothing allocated).
- **Where the time to open a big file goes:** 0.4 – 0.5 s reading and
  replacing the document, the rest (to 1.3 – 1.7 s) the first frame
  triangulating and uploading about 4.5 million triangles.

**14.5 — The interface pass**

*Problems and solutions:*
- **Driving the interface without a window.** The layout the binary drew
  each frame moved into `grid4d_ui::shell::Shell`, so tests run the very
  same interface; `egui_kittest` feeds it clicks, drags, modifier keys and
  typing through egui's own input, and finds widgets by their accessible
  names.
- **Tests without fonts.** egui 0.37 no longer bundles its fonts by
  default — the application gets them through eframe — so every headless
  test so far had laid text out with no font at all (silently, as empty
  boxes). The interface tests turn egui's `default_fonts` on.
- **Found by the accessibility check:** six Coordinates fields and the
  frame-rate menu had no name, and Object Manager rows were not in the
  accessibility tree at all. Fields are now named by their labels, rows by
  their objects (as tree items, with their selection).
- **Found by laying out every text:** the menu-path arrow "▸" and the
  remove button's "✕" are not in the built-in font; Windows drew them from
  a system font, a system without one would not. Replaced by "›" and "×".
- **Found by reading the pictures:** the Coordinate Manager's column titles
  were not translated (written without `tr`, so the completeness test could
  not see them), and the Optimize dialog was too narrow for its Greek.
- **A false alarm:** in the first Light-theme pictures menus were invisible
  and fields black — the test harness re-applies its own (dark) theme after
  setup; the walkthrough now names the theme to the harness too.
- **The two shortcut lists had drifted** (each had rows the other lacked);
  they are one table now, and the manual's table is checked against it.
- **On-screen driving:** posted (and real) mouse clicks from the test driver
  no longer reach the window, which is why the walkthrough is rendered
  headless rather than clicked through on screen.

**14.6 — Packaging and the release workflow**

*Problems and solutions:*
- **Icons at every size without image tools.** Only a 256-pixel PNG and
  the Windows `.ico` existed. A small throw-away program (the `resvg` crate,
  outside the repository) rendered the SVG masters at 16 – 1024 pixels (the
  small master below 48); the `.icns` is written directly (its PNG entries
  behind a short header). The results are committed, so the workflow needs
  no image tools.
- **Nothing here to build an `.msi`, `.dmg` or AppImage** (no WiX, no Mac,
  no Linux). The workflow therefore has a dry run by hand that builds every
  package as an artifact without publishing anything; the Windows `.zip` was
  built and started here (its file properties say Grid4D 1.0.0-rc.1).
- **Apple silicon refuses an unsigned bundle** whose parts disagree, so
  the universal app gets an ad-hoc signature (not a certificate: R4 holds).
- **An MSI version must be numeric**: the installer takes `1.0.0` from
  `1.0.0-rc.1` and allows a same-version upgrade, so 1.0.0 replaces the
  release candidate.
- **macOS document opening:** the bundle declares `.g4d` documents, but the
  "open document" event macOS sends does not reach the app through eframe
  yet; double-clicking starts Grid4D without the file (written in the manual).
- **Pinned tools:** WiX 5.0.2, appimagetool 1.9.1, and action versions
  checked to exist before use.
- **CI ran out of disk linking the interface tests** (Linux: the linker died
  with a bus error, the usual sign of a full disk): since 14.5 the
  `grid4d-ui` test binary links egui, wgpu and `egui_kittest`. The test job
  now builds with line-table debug information only and, on Linux, first
  removes preinstalled tools Grid4D never uses (Android SDK, .NET, GHC,
  CodeQL).
- **The first dry run** built the macOS and Linux packages at once; the
  `.msi` failed: WiX gives a component a generated GUID only when it holds
  one unversioned file (or a versioned key file), and the licence files
  shared one component. Each has its own now. (A "Re-run jobs" of a failed
  run builds the same old commit again; a new run is needed after a fix.)
  The second dry run built every package: Windows in 18 minutes, macOS in 5,
  Linux in 8.

**14.7 — Documentation and 1.0**

*Problems and solutions:*
- **Pictures for the README without a window.** The old screenshot showed
  a toolbar from months ago. A showcase scene (clones moved by a Random
  effector, extruded text, a Subdivision Surface, a bent column, a light
  with soft shadows) is built in code and rendered headless twice — the
  whole workspace with shadows and ambient occlusion on, and the same view
  through the path tracer (`readme_pictures`, beside the walkthrough).
- **The names-and-wording review (R2):** the interface names no other
  product (only About's "not affiliated" note); the documentation mentions
  Cinema 4D only to describe the workflow it follows, always with the
  trademark note; "MoGraph" (a Maxon product name) was left in the
  changelog and a code comment and is gone. The icons are original.
- **The `.msi` tried by hand:** installed, started from the Start menu and
  uninstalled from Settings ▸ Apps on the reference laptop.

---

## Phase 15 — Materials, UVs and textures (2026-11-06 – )

| Step | What was built | Tests |
|------|----------------|-------|
| 15.1 | Five specs: the format version 4, material channels, textures and the Texture Manager, the Material tag's projections and layers, the UV editor; `.g4d` version 4 written (version 3's module moved to `v4.rs`; `v3.rs` upgrades a version 3 `document.json` as JSON), `FILE_FORMAT.md` for version 4, golden `demo_v4.g4d` / `full_v4.g4d`, the version 3 fixtures frozen and checked to read exactly as before, a property test of documents labelled version 3, the fuzz corpus with both versions; the manual and changelog say what 1.1 files mean for 1.0 | 783 |

**15.1 — Specs and the format version 4**

*Problems and solutions:*
- **A new version without a second copy of the format.** Version 3's
  module was the whole reader and writer (4,000 lines besides its tests).
  A frozen copy beside a new one would have doubled it and let the two
  drift apart. Version 4 only adds to version 3, so the module became
  `v4.rs`, and version 3 is now an upgrade of the parsed JSON before
  version 4's types read it: it removes every field version 4 adds
  (Grid4D 1.0 ignored unknown fields) and will convert any value whose form
  changes. What a version 3 file means is pinned by its fixtures: the
  frozen `full_v3.g4d`, loaded and saved, must give exactly what the same
  document gives when written now.
- **Errors without a position.** Read from JSON already parsed, serde knows
  no line or column; such errors (only in upgraded files) show the message
  alone.
- **The plan's "Texture tag".** Grid4D already has a Material tag that
  links a material; a second tag doing the same with a projection would
  raise the question of which wins. The spec grows the Material tag instead
  (projections, several per object layered in order), as the reference's
  own tag does.
- **Version 4 holds no new data yet.** The plan puts the format first; the
  data arrives with the steps that make it (textures 15.3, channels and
  projections 15.4, seams 15.7), each adding to version 4 while it is open
  and listing the addition in `v3.rs` and `FILE_FORMAT.md`. Version 4 is
  frozen when 1.1 ships.

---

## How this file is kept

After every step (and every fix worth noting) this file gets the new step in its
phase's table, its problems and solutions, and an updated Status block. The manual is
updated in the same change.
