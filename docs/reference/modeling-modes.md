# Editable meshes — Make Editable, component modes, component selection

> Sources (Cinema 4D 2027.0 manual, read 2026-10-19):
> [Make Editable](https://help.maxon.net/c4d/en-us/Content/html/5664.html),
> [Polygon Object](https://help.maxon.net/c4d/en-us/Content/html/OPOLYGON.html),
> [Component Modes](https://help.maxon.net/c4d/en-us/Content/html/68782.html),
> [Points](https://help.maxon.net/c4d/en-us/Content/html/5490.html) /
> [Edges](https://help.maxon.net/c4d/en-us/Content/html/5492.html) /
> [Polygons](https://help.maxon.net/c4d/en-us/Content/html/5491.html) /
> [Model](https://help.maxon.net/c4d/en-us/Content/html/5494.html),
> [Tweak](https://help.maxon.net/c4d/en-us/Content/html/54037.html),
> [Select menu](https://help.maxon.net/c4d/en-us/Content/html/5605.html) and its tool pages
> (Brush/Live, Rectangle, Loop, Ring, Select All, Deselect All, Invert, Select Connected,
> Grow, Shrink, Convert Selection, Hide Selected),
> [Optimize](https://help.maxon.net/c4d/en-us/Content/html/5674.html).
> Written in our own words — no copied text or images.
>
> Covers implementation-plan steps **3.6** (Make Editable), **4.1** (modes, display,
> picking), **4.2** (selection tools) and **4.3** (component transforms). Modelling
> tools (Extrude, Bevel, …) get their own spec, `modeling-tools.md`.

## Purpose

Primitives are parametric: they have no points to grab. **Make Editable** turns them
into a *polygon object* whose points, edges and polygons the user selects and
transforms in the matching **component mode**. This is the base of every modelling
tool and of the Subdivision Surface workflow (see
[`generator-subdivision-surface.md`](generator-subdivision-surface.md)).

## 1. Polygon object

- Holds points and faces in object space. Faces are **triangles, quads or n-gons**
  (more than four corners); all three are first-class.
- A face whose points are not coplanar is *non-planar*; it is drawn (and later
  rendered) as triangles. A face whose points are collinear is *degenerate* (no
  normal). Neither is an error — tools must not crash on them, and normals must not
  produce NaN.
- Each face has its own frame, used by tools such as Matrix Extrude: origin at the
  face centre, X along the first edge (corner 0 → 1), Z = face normal, Y completing a
  right-handed frame. (Our `PolyMesh::face_center` / `face_normal` already provide the
  pieces.)
- An empty polygon object (no points) is valid and can group children like a Null.
- Quads are the preferred topology: triangles and n-gons subdivide less cleanly.

## 2. Make Editable

| | |
|---|---|
| Invoked by | `C`, Mesh ▸ Conversion ▸ Make Editable, toolbar button |
| Applies to | every selected object that is parametric |
| Result | the object becomes a polygon object built from its current parameters |
| Reversible | only through Undo — there is no way back to the parameters |

Behaviour:

- Name, transform, colour, flags, children and position in the hierarchy stay the
  same. Grid4D keeps the **same `NodeId`** (the kind changes in place), so the
  selection and references survive.
- Several selected objects convert in **one undo step**.
- Objects that are not parametric (Null, Camera, Light, Polygon) are left alone; if
  nothing convertible is selected the command does nothing and adds no undo step.
- Geometry is the primitive's mesh at its current segment counts, sized in object
  space (the size parameters are baked into the points, scale stays 1).
- Seams: the manual notes that converted primitives may contain **duplicate
  points** on purpose — e.g. a cylinder's caps are separate from its side so the
  rim stays hard under Subdivision Surface; **Optimize** merges them when unwanted.
  **Decided:** Grid4D matches this — Cylinder and Cone caps get their own ring of
  points, not shared with the side (the generator currently welds them; change it
  together with Make Editable in step 3.6).
- After conversion the active mode does not change.
- *Current State To Object* (later, with generators): makes a polygon **copy** of the
  object's evaluated result (deformers/generators applied), ignoring children, and
  leaves the original untouched.

## 3. Modes

| Mode | Acts on | Gizmo / Delete / Select All act on |
|------|---------|------------------------------------|
| Model | whole objects (geometry is moved, axes stay unscaled) | objects |
| Points | the active polygon object's points | selected points |
| Edges | its edges | selected edges |
| Polygons | its faces | selected polygons |

- C4D also has an *Object* mode (scales the object's axes, used for animation).
  Grid4D's Model mode currently scales the transform; revisit when animation lands
  (Phase 6) — the manual's rule of thumb is "Model for modelling, Object for
  animating".
- Component modes work on **polygon objects**; on a primitive they show nothing
  selectable (the user must Make Editable first). With several polygon objects
  selected, each shows and edits its own components in the reference application.
  **Decided:** Grid4D v0.4 shows and edits components of the **active object only**
  (the last selected); multi-object component editing comes later.
- Tools that make no sense in the current mode are disabled (greyed out), not hidden.
- **Converting a selection by switching mode**: Shift+click on another mode
  converts the selection *tolerantly* (e.g. a point selects every edge/face that uses
  it); Ctrl+click converts *strictly* (an edge/face is selected only if all its points
  were). A plain click just switches mode and keeps each mode's own selection.
- **Shortcuts:** to be verified against the manual's shortcut list before binding
  (plan § Phase 4). Mode switching is mouse-driven in the palette until then.

## 4. Display in component modes

- Points: small screen-space squares at every point of the active polygon object;
  selected points highlighted in the selection colour.
- Edges: all edges drawn as lines; selected edges in the selection colour.
- Polygons: selected faces tinted; edges stay visible so faces can be told apart.
- Depth-tested against the shaded surface so hidden components do not clutter the
  view, while staying readable on top of their own surface (small depth bias).
- Hovering highlights the component that a click would pick (pre-selection).
- Hidden components (see §6) are neither drawn nor pickable.
- Colours come from `theme::palette`; sizes from `theme::metrics`.

## 5. Selecting components

### Modifiers (all selection tools)

| Input | Effect |
|-------|--------|
| click / drag | replace the selection |
| Shift + click / drag | add to it |
| Ctrl + click / drag | remove from it |
| Esc while dragging | cancel the drag (selection returns to how it was) |
| click or drag on empty space | deselect all (in the current mode) |

The Move/Scale/Rotate tools have built-in selection: clicking a component selects it
(with the same modifiers), dragging the gizmo transforms the selection.

### Tools

| Tool | Parameters (default) | Behaviour |
|------|----------------------|-----------|
| **Live Selection** (brush, `9`) | Radius px, 1..1000 (default to verify, ~10); *Visible Only* (on); *Tolerant* (off) | Paint over components to select them. Middle-drag changes the radius. *Visible Only*: components behind other faces are not selected, even when visible through wireframe. *Tolerant*: an edge/face is taken as soon as the brush touches it; otherwise only once most of it (≈ 75 %) has been painted. |
| **Rectangle Selection** | *Visible Only* (on); *Tolerant* (off) | Drag a marquee. *Tolerant*: an edge/face is taken if any part of it is inside; otherwise only if all of its points are inside. |
| **Loop Selection** | *Stop at Selections*, *Stop at Non-Quads*, *Stop at Poles* (off) | Hover highlights the loop under the cursor; click selects it. Works in all three modes. Always stops at borders and non-manifold geometry. A *pole* (for Loop) is a point whose face count is neither 2 (border) nor 4 (regular grid). Double-clicking an edge with Move/Scale/Rotate also selects its loop; double-clicking a border edge selects the whole border. |
| **Ring Selection** | same three options | Selects the parallel elements around a ring (e.g. every vertical edge of a cylinder). A *pole* for Ring is a point surrounded only by triangles (sphere poles). |

Lasso, Polygon (freehand), Path, Outline/Fill and Phong-break selections come later.

### Commands (Select menu)

| Command | Behaviour |
|---------|-----------|
| Select All | every **visible** component of the current mode |
| Deselect All | clears the current mode's selection |
| Invert | flips the selection of every visible component; hidden ones are untouched |
| Select Connected | grows to every component reachable through shared geometry, staying in the current mode; double-click a face with Move/Scale/Rotate does the same in Polygons mode |
| Grow Selection | adds every neighbour; option *Grow Using Edges Only*: faces must share an **edge** (on) or merely a **point** (off) |
| Shrink Selection | removes the components on the border of the selection; a selection with no border (all faces of a closed mesh) does not shrink |
| Convert Selection | dialog converting between point/edge/polygon selections, with *Tolerant* as in §3; switches to the target mode |
| Hide Selected / Hide Unselected / Unhide All | hiding faces also hides their points; hiding points hides only the points |

### Where selections live

C4D stores component selections **in the object** (they are saved with the file and
undoable). Grid4D follows that:

- Point and polygon selections: bitsets over point / face indices.
- Edge selection: bitset over **corners** — corner *c* stands for the edge from that
  corner to the next one in its face. An edge shared by two faces is selected when
  either of its corners is; `Topology::corner_edge` maps corners to unique edges.
  Corners, unlike unique edge indices, do not depend on rebuilding the topology.
- Hidden flags: the same three bitsets.
- They are stored **next to** the `MeshHandle`, not inside it, so selecting does not
  create a new `MeshId` or re-upload the GPU mesh.
- Selection changes are document edits (`SceneEdit` → undoable), one undo step per
  click or drag. They are saved in `.g4d` v3.
- Mesh operations that renumber points or faces must remap or clear the selections
  they invalidate.

## 6. Transforming components

- Move / Scale / Rotate with the existing gizmo act on the selected components of
  the current mode (edges and faces move their points).
- Pivot = centre of the selection's bounding box; axes = object space (world and
  normal axes later, with the Coordinate Manager).
- One drag = one undo step (`LiveEdit` + `EditorCommand::CommitLive`, as for objects).
- **Tweak** (option, off by default): with nothing selected, pressing on a component
  and dragging transforms just that component without changing the selection.
- Double-clicking a point with a transform tool opens numeric entry (later, with the
  Coordinate Manager).

## 7. Deleting

`Delete` / `Backspace` acts on the current mode:

| Mode | Removes |
|------|---------|
| Model | the selected objects (existing behaviour) |
| Points | the selected points and every face that uses one of them |
| Edges | the selected edges together with the faces that use them (confirmed in the manual; to keep the faces use *Melt* or *Dissolve*, see `modeling-tools.md`) |
| Polygons | the selected faces; points no other face uses are removed too |

Afterwards unused points are dropped and indices compacted (the same clean-up as
*Optimize ▸ Unused Points*), selections cleared or remapped. One undo step.

## 8. Optimize (needed early, small)

Options: *Polygons* (drop faces with fewer than 3 distinct points), *Unused Points*,
*Points* (merge points closer than *Tolerance*). Applies to the selection, or to the
whole mesh when nothing is selected. Faces that collapse while merging are deleted.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| Polygon kind, `MeshHandle` | `ObjectKind::Polygon` (`scene/node.rs`), `scene/mesh.rs` (in the working tree) |
| Component selection + hidden bitsets | new `ComponentSelection` next to the mesh handle in `ObjectKind::Polygon`; `SceneEdit` variant for selection changes |
| Make Editable | `EditorCommand::MakeEditable` → `commit` with one `SceneEdit` per converted node |
| Modes | `EditMode` (`ui/editor.rs`) — already has Model/Points/Edges/Polygons |
| Display | `grid4d-render`: point/edge/face-tint passes in the offscreen `ViewportTarget` |
| Component picking | `grid4d-eval` picking + mesh BVH (`geom/bvh.rs`); screen-space distance for points/edges |
| Selection algorithms (loop, ring, grow, shrink, connected, convert) | pure functions in `grid4d-geom` over `PolyMesh` + `Topology`, each with tests on `shapes.rs` meshes |
| Delete / Optimize | `grid4d-geom::ops`, returning a new `PolyMesh` + index remap |
| Tools | `Tool` gains `RectangleSelection`, `LoopSelection`, `RingSelection`; options shown in the Attribute Manager (tool mode) |

## Decisions (2026-10-19)

1. **Cylinder/Cone caps are separate** from the side (own ring of points), as in the
   reference application, so the rim stays hard under Subdivision Surface; *Optimize*
   welds them when a rounded rim is wanted.
2. **Active object only** for component display, picking, selection and transforms
   in v0.4; multi-object component editing later.

## Open questions

1. **Live Selection default radius** and the mode shortcuts: confirm from the manual's
   shortcut list / a running copy before binding.
2. ~~**Edge delete**~~: settled 2026-10-19 — Delete removes the edges' faces; Melt /
   Dissolve keep them (`modeling-tools.md`).

## Acceptance criteria

- [ ] Make Editable on a Cube (1 segment) gives 8 points, 6 quads, closed and manifold;
      same name, transform, children and `NodeId`; one undo step restores the Cube with
      its parameters.
- [ ] Make Editable on a Cylinder gives caps whose points are not shared with the
      side (each rim exists twice); Optimize merges them into a closed mesh.
- [ ] With two polygon objects selected, only the active one shows components.
- [ ] Make Editable on a mixed selection (Cube, Null, Sphere) converts only the
      primitives, in one undo step; with nothing convertible no undo step is added.
- [ ] In Points/Edges/Polygons mode a click picks the nearest visible component;
      Shift adds, Ctrl removes, click on empty space deselects; each is one undo step.
- [ ] Live Selection with *Visible Only* never selects back-facing occluded points of
      a sphere; with it off, it does.
- [ ] Rectangle *Tolerant* off selects a face only when all its points are inside.
- [ ] Loop selection on a quad sphere's equator edge selects the full ring of edges;
      it stops at the poles; on a plane it stops at the border.
- [ ] Grow then Shrink on an interior face patch of a plane returns the original set;
      Shrink on "all faces of a cube" changes nothing.
- [ ] Select All / Invert ignore hidden components.
- [ ] Shift+click on Polygons mode with one point selected selects every face using
      that point; Ctrl+click selects none.
- [ ] Moving selected points with the gizmo is one undo step; the mesh gets a new
      `MeshId`, the selection does not.
- [ ] Deleting a point of a cube removes the 3 faces using it and leaves a valid mesh
      (no unused points, no out-of-range indices).
- [ ] Component selections and hidden flags survive save and reload (`.g4d` v3).
- [ ] Idle frames in component modes do not touch the document
      (`idle_frames_do_not_touch_the_document`), and component display allocates
      nothing per frame once warmed up.
