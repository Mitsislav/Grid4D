# The UV editor

> Sources: the established workflow of UV editors in 3D suites (the
> reference's UV tools included, by their general behaviour only: a 2D view
> of the UV square over the texture, the three component modes, unwrapping
> from seams, relaxing and packing). LSCM is a published algorithm (Lévy et
> al., 2002). The reference's introduction to its UV Editor
> ([UV_Editor_Overview](https://help.maxon.net/c4d/en-us/Content/html/UV_Editor_Overview.html),
> read 2026-11-07) was read before step 15.7; the corrections are under
> *Decisions* below. Written in our own words, 2026-11-06 / 2026-11-07.
>
> Covers implementation-plan step **15.7**, built in three parts: **15.7a**
> the algorithms in `grid4d-geom`, **15.7b** seams in the document and the UV
> commands, **15.7c** the pane.

## Purpose

Seeing and arranging how a polygon object's surface lies on its pictures:
cutting it into flat pieces (islands) along seams, flattening them with
little stretching, and laying them out in the UV square.

## The pane

- A **pane** of the workspace (Window ▸ UV Editor; a tab beside the
  viewport in the Modeling layout), showing the **active polygon object**'s
  UVs as a 2D mesh over the 0 – 1 square, with the material's Base Color
  texture behind it (a menu chooses another channel or none) and the tiling
  grid around it.
- Navigation: middle-drag or Alt+drag pans, the wheel zooms about the
  cursor, **H** frames all, **O** frames the selection.
- **Points / Edges / Polygons** modes and the selection are **the same as
  the viewport's** (one component selection per object): selecting in either
  shows in both. Click, Shift (add), Ctrl (remove), rectangle drag; Select
  All / Deselect All / Invert / Grow / Shrink / Connected work here too, and
  **Select Island** (double-click) selects all connected UV polygons.
- UV corners are per corner: a point shared by several faces may sit in
  different places in UV space (a seam). Points mode moves the UV corners at
  one place together.

## Tools

| Tool | What it does |
|------|--------------|
| Move, Scale, Rotate | the selection in UV space (a 2D gizmo; values in the Attribute Manager); one undo step per drag |
| Flip U / Flip V | mirrors the selection about its centre |
| Unwrap | flattens the selected polygons (all, if none) with **LSCM**, cutting along **seams**: the selected edges, else edges sharper than an angle (60°), else the mesh's open borders; each island scaled to its 3D area; then packed |
| Project | from a projection (Box, Spherical, Cylindrical, Flat, Camera; as in [`tag-material.md`](tag-material.md)) or from the view |
| Relax | moves inner UV points to even out stretching, borders pinned (an option frees them); iterations 1 – 100 (20) |
| Pack Islands | scales and places every island (or the selected ones) into the square without overlap; margin in pixels of a chosen texture size (4 px of 1024); rotation in 90° steps allowed (on) |
| Mark Seams / Clear Seams | keeps the selected edges as seams for later Unwraps (stored with the object) |
| Stretch display | colours polygons from blue (compressed) to red (stretched) |

## Behaviour

- Every tool is one undo step and changes the object's mesh like any mesh
  edit (a new mesh handle; selections kept).
- A polygon object without UVs gets them on the first Unwrap or Project.
- Objects that are not editable (primitives, generators) show their UVs
  read only, with a note to make them editable.

## Decisions (2026-11-07, after reading the reference)

What the reference does, in short: its UV points are kept apart per island
(a point on a border exists once on each side, and selecting a UV polygon
selects only its own UV points); seams are a selection of their own, kept in
a tag; a double-click selects an island (polygons) or a loop (edges); one key
frames the selection; pins hold UV points while relaxing; coordinates past
0 – 1 lie in neighbouring tiles.

What Grid4D does:

- **One selection.** Polygons and edges selected in the UV editor are the
  object's own selection, shared with the viewport and saved (no second set
  to keep in step). Points mode selects the object's points; a selected
  point shows, and moves, at **every** UV place it has (each side of a
  seam). A separate selection of UV points may come later.
- **Islands** are found from the UVs themselves: two polygons sharing an
  edge are in one island when their corners agree in UV at both ends of it.
  A **UV vertex** is the corners of one point that are joined this way.
- **Seams** are an edge set of their own, stored with the object's
  components (15.7b), shown in both views; Unwrap cuts along the seams,
  the borders of what it unwraps and the mesh's open borders; with no seams
  marked and no edges selected it cuts at edges sharper than 60°.
- A part that would still be closed after cutting (a sphere with no seams)
  is split by the direction its polygons face (like a Box projection), so
  every part flattens.
- **Framing**: H frames everything, S the selection (as in the F-Curve
  editor), with the pointer over the pane.
- Not in 15.7: pins, soft selection, UV tiles past the first (coordinates
  outside 0 – 1 simply repeat the picture), Align and Straighten tools.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| Algorithms: LSCM (sparse least squares, conjugate gradients), islands, seams, relax, pack, projections | `grid4d-geom` `uv.rs`, pure functions with tests |
| Mesh edits | `ui/src/modeling.rs`, `EditorCommand::UvCommand(UvOp)` through `commit` |
| Pane | `panels/uv_editor.rs`, a `Pane`; drawn with egui shapes (no GPU view needed) |
| Seams | stored with the polygon object's components (file: v4) |

## As built (15.7a, the algorithms)

All in `grid4d_geom::uv`, pure functions that return new corner UVs:

- `layout`: UV vertices (union-find over corners joined across edges whose
  two sides agree within 10⁻⁵) and islands; `sharp_edges(angle)`.
- `unwrap(mesh, topology, faces, cuts, options)`: parts are the chosen
  faces joined across edges that are not cut; a part whose Euler
  characteristic is not 1 (closed, or with handles) is split by the axis
  its faces face. Each part is triangulated (the viewport's triangulation)
  and flattened by LSCM: two Cauchy-Riemann rows per triangle weighted by
  the square root of its area, two far-apart points pinned, solved by CGLS
  (conjugate gradients on the normal equations, columns scaled to unit
  length, stopped at 10⁻⁶ of the problem's own gradient) from a planar first
  guess. A part is turned the right way round if mirrored, scaled to its
  surface area, and the parts packed. Faces not chosen keep their UVs.
- `relax(mesh, topology, faces, options)`: springs along each polygon edge
  with the surface length at the island's scale; each round moves every free
  UV vertex half its mean pull, halving the step (down to an eighth) rather
  than turning a face over; borders pinned by default; 1 – 100 rounds.
- `pack(uvs, islands, options)`: boxes (turned a quarter when taller than
  wide, which never mirrors) in rows, tallest first, the row width tried at
  several sizes, the gap found in a few rounds so it is the margin after the
  final scale; all scaled into 0 – 1.
- `flip(uvs, corners, axis)`, `face_stretch` (log₂ of a face's UV area per
  unit of surface over its island's, for the stretch display) and
  `stretch_measure` (mean squared relative difference of edge lengths).
- Unwrapping one part of 16 384 quads over hills takes 1.8 s on the
  reference machine (`cargo bench -p grid4d-geom`, "UV"); a flat one of
  65 536 quads 0.4 s.

## As built (15.7b, seams and commands)

- `ComponentState::seams` (scene): a `BitSet` of corners, edges by corner
  like an edge selection; `.g4d` v4 writes it as `seams` (a component set
  with `edges` only) on polygon kinds; a version 3 file's `seams` are
  dropped. Rebuilding operations carry seams like the edge selection
  (`Rebuilt::carry`).
- `EditorCommand::UvCommand(UvOp)` (`ui/src/uv_tools.rs`): Unwrap, Relax,
  Pack, Project (Box, Spherical, Cylindrical, Flat, fitted to the mesh's
  bounds), Flip U / V, Mark Seams, Clear Seams on the active polygon
  object's selected visible polygons (all visible ones when none); one undo
  step, none when nothing changes. UV changes make a new mesh handle and
  keep the components; seam changes keep the mesh.
- Unwrap's cuts: the seams, plus the selected edges in Edges mode; with
  neither, edges sharper than `UvSettings::seam_angle` (60°). Pack's margin:
  `margin_pixels` of a `texture_size` picture (4 of 1024).
- Mesh ▸ UV holds the commands until the pane (15.7c) has its own menus;
  the random-command test applies them.

## Acceptance criteria

- [x] Unwrap of a cube cut along seven edges gives one island with no
      flipped faces; a cylinder cut along one edge flattens with area
      distortion under 5 %.
- [x] Pack Islands never overlaps islands; everything lies in 0 – 1.
- [x] Relax lowers the stretch measure and never flips a face.
- [ ] Selecting in the UV editor selects the same components in the
      viewport, and back.
- [ ] Every tool is one undo step; save and load keep UVs and seams.
