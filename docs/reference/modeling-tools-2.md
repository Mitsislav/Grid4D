# Modelling tools 2: Slide, Plane Cut, Polygon Pen, Extrude along Spline, Mirror, Array, Bevel profiles

> Sources: the reference's pages for these tools were not read for this
> spec; each tool is described by what it does to a mesh, in the plain terms
> any polygon modeller uses, and its options are ours. Written 2026-11-09 in
> our own words.
>
> Covers implementation-plan steps **17.6** (Slide) and **17.7** (the rest).
> Names (R8): all plain and descriptive (*Slide*, *Plane Cut*, *Polygon Pen*,
> *Extrude along Spline*, *Mirror*, *Array*); shortcuts stay unbound until
> verified against the reference's list.

## Common rules

- Each tool works on the active polygon object's selection (all visible
  components when none, where that makes sense), through `modeling.rs` like
  Extrude: a **viewport drag** that rebuilds from the press every frame (Esc
  restores) and an **Apply** button in the Attribute Manager's Tool page;
  **one undo step** either way.
- Every operation returns a `Rebuilt` mesh recording each point's and face's
  origin, so selections, hidden flags, seams, creases and hard edges carry
  across (`Rebuilt::carry`); what it creates is selected (`modeling::Fresh`).
  A property test per tool: random meshes and selections, the carried
  selection names the same geometry.
- The options are editor state (`ModelingSettings`), never saved in a document.

## Slide (points, edges)

| Option | Range | Default | Effect |
|--------|-------|---------|--------|
| Offset | −100 – 100 % | 0 | How far along the neighbouring edges (a drag sets it). |
| Clamp | bool | on | Stop at the neighbouring points. |
| Keep Curvature | bool | off | Move along the curve through the neighbours, not the straight edge. |
| Duplicate | bool | off | Leave the original loop and slide a new one (an edge loop inserted). |

- **Points**: a selected point moves along the edge nearest the cursor's
  direction. **Edges**: a selected loop moves as a whole along the edges that
  cross it (each point on its own ring edge), towards the side the drag goes.
- Never changes topology (unless Duplicate), so it is a point edit.
- *Decided in 17.6 (2026-11-25):* a drag's left/right sets Offset as a
  share of the rails' mean length; single points slide towards the
  neighbour along the view's right (the direction is kept for Apply). A
  loop's two sides come from grouping the faces around it (faces across a
  loop edge are opposite, faces sharing another edge at a loop point are on
  one side); positive offsets go to the side of the first face of the
  loop's lowest edge, a drag to the side that lies the way it goes. Keep
  Curvature follows the parabola through the two neighbours. Duplicate cuts
  the ring of the first rail (Loop/Path Cut's own cut), so on a partly
  selected loop the new loop runs the whole ring.
- **Slide is done (17.6)**: Apply and a drag; one undo step; a test slides
  a loop halfway on every rail and inserts one with Duplicate.

## Plane Cut

| Option | Range | Default | Effect |
|--------|-------|---------|--------|
| Plane | Object XY / ZY / XZ · World XY / ZY / XZ (Free: Line Cut's Infinite option) | Object XZ | Free: drawn by a line in the view (the plane through it along the view); the others: an axis plane placed by the handle. |
| Cuts | 1 – 64 | 1 | Parallel cuts. |
| Spacing | length | 10 cm | Between parallel cuts. |
| Selected Only | bool | off | Cut only the selected polygons. |
| Split | bool | off | Separate the pieces at the cut (the cut points doubled). |

- Every polygon the plane crosses is split along it; new points lie exactly
  on the plane; new edges are selected. A plane that touches a point or runs
  along an edge adds nothing there.

## Polygon Pen

| Option | Range | Default | Effect |
|--------|-------|---------|--------|
| Snap Distance | pixels | 10 | Clicks this near a point or edge use it. |
| N-Gons | bool | on | Off: a polygon of more than four points is split into quads and triangles. |

- **Clicking** places points on the object's surface or on existing points
  and edges (snapping as Snapping does); clicking the first point closes a
  new polygon, welded to what it snapped to. Backspace removes the last
  point; Esc cancels the polygon.
- **Ctrl-dragging an open border edge** pulls out a new quad row from it.
- Each closed polygon is one undo step (as the spline Pen's points are).

## Extrude along Spline

| Option | Range | Default | Effect |
|--------|-------|---------|--------|
| Spline | a link to a spline | none | The path. |
| Steps | 1 – 1 000 | 10 | Segments along it. |
| Twist | angle | 0° | Rotation over the whole path. |
| End Scale | 0 – 1 000 % | 100 % | The profile's size at the end (linear along it). |
| Follow Path | bool | on | The profile turns with the path (parallel transport, as the Sweep); off: it only moves. |

- Each selected region of polygons is extruded along the spline's shape
  (the path taken from its first point, placed at the region's centre, its
  start along the region's normal); the sides are quads, the end cap is the
  region moved. A missing or degenerate spline does nothing.

## Mirror

| Option | Range | Default | Effect |
|--------|-------|---------|--------|
| Plane | Object XY / ZY / XZ · World XY / ZY / XZ | Object ZY | The mirror plane (through the object's or world origin; the handle moves it). |
| Duplicate | bool | on | Keep the original and add the mirror; off: flip the selection over. |
| Weld | bool | on | Points meeting at the plane within the tolerance are made one. |
| Tolerance | length | 0.01 cm | — |

- Works on the selected polygons (the whole object when none); faces of the
  mirror are turned so both halves face out.

## Array

| Option | Range | Default | Effect |
|--------|-------|---------|--------|
| Copies | 1 – 1 000 | 3 | — |
| Offset | vector | (100, 0, 0) cm | Added per copy. |
| Rotation | H, P, B | 0° | Added per copy, about the selection's centre. |
| Scale | % | 100 % | Multiplied per copy. |
| Weld | bool | off | Points of neighbouring copies within 0.01 cm are welded. |

- Copies the selected polygons (the whole object when none) as new pieces;
  the copies are selected.

## Bevel: profile and mitres

| Option | Range | Default | Effect |
|--------|-------|---------|--------|
| Profile | Round · Custom | Round | Round: the existing depth bulge. Custom: a curve from the edge to the offset, edited as points in the Tool page (presets: Chamfer, Step, Cove). |
| Mitres | Default · Patch | Default | Default: as 1.2 (points slid, limited). Patch: where three or more bevelled edges meet, a grid patch fills the corner, so the profile turns the corner. |

- A Custom profile with *n* points makes *n − 1* rows across the bevel
  (Subdivision is then ignored).

## Decisions made while implementing (2026-11-25)

1. **Plane Cut's Free plane is Line Cut's Infinite option** (a line drawn
   in the view already cuts along the plane through it); Plane Cut keeps
   the six axis planes, moved along their normal by Offset — what a drag
   sets. It shares Line Cut's cutting code (`cut_by_plane`); several cuts
   run one after another, their origins composed.
2. **Mirror, Array and Extrude along Spline run from Apply only** (they
   have no single amount for a drag to set).
3. **Mirror and Array work on the selected polygons in Polygons mode**,
   the whole object otherwise; Extrude along Spline only on selected
   polygons. Mirror's image reads its UVs with U turned round, as the
   Symmetry's does.
4. **Extrude along Spline keeps every point and face number and appends**
   the rings (like Extrude), so the selected region is the moved end; the
   path is the spline's first segment resampled at equal lengths, the
   profile carried by parallel transport.
5. **The Polygon Pen's N-Gons option is left out**: a closed polygon is
   always one polygon (Connect or Triangulate can split it). Its corners
   snap to points, then edges, then the surface, then the plane through the
   object facing the view.
6. **Bevel's Custom profile** is kept as up to 16 points in the corner's
   axes ((1, 0) on the first face's side, (0, 1) on the second's), presets
   Chamfer, Step and Cove; edited as rows of numbers in the Tool page.
   **Patch mitres** fill a corner of three or more bevelled edges with
   triangles to a point pulled from the hole's middle towards the original
   corner (by half the Depth, Round; by 0.3, Custom).

## Acceptance criteria (all met in 17.6 – 17.7)

- [x] Every tool: Apply and a drag give the same mesh for the same options;
      one undo step; selections carried (a randomized test over shapes and
      planes for Plane Cut, Mirror and Array; the random-command test runs
      every tool).
- [x] Slide clamps at neighbours; a loop slid 50 % lies halfway on every
      ring edge; Duplicate inserts a loop.
- [x] Plane Cut on a cube through its centre gives 4 new quads' worth of
      points, all on the plane; 3 cuts give 3 loops.
- [x] Polygon Pen closes a quad welded to four existing points; a dragged
      border edge makes one new quad.
- [x] Extrude along Spline of one quad along a quarter circle, 10 steps:
      40 side quads, the end quad turned 90°.
- [x] Mirror of half a cube welded gives a closed cube; Array of 3 gives 3
      pieces at the offsets.
- [x] A Custom bevel profile's rows follow its points; Patch mitres leave no
      gap at a cube's corner.
