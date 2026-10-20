# Modelling tools — Extrude, Inset (more to follow)

> Sources (read 2026-10-19): Cinema 4D manual —
> [Extrude](https://help.maxon.net/c4d/en-us/Content/html/TOOLEXTRUDE.html) and its
> options ([S22 page](https://help.maxon.net/c4d/s22/us/html/TOOLEXTRUDE-MDATA_MAINGROUP.html)),
> [Extrude Inner / Inset](https://help.maxon.net/c4d/s22/us/html/TOOLEXTRUDEINNER.html) and its
> [options](https://help.maxon.net/c4d/s22/us/html/TOOLEXTRUDEINNER-MDATA_MAINGROUP.html),
> [Quick Start](https://help.maxon.net/c4d/en-us/Content/html/CINEMA_4D_QUICKSTART.html)
> (shortcuts D and I), [Keyboard Shortcuts](https://help.maxon.net/c4d/2025/en-us/Content/html/Shortcuts.html)
> (2025: chords such as K L, M M), [Loop/Path Cut](https://help.maxon.net/c4d/s22/us/html/TOOLKNIFEPATH-MDATA_MAINGROUP.html)
> and its [interaction](https://help.maxon.net/c4d/s22/us/html/TOOLKNIFEPATH-MDATA_KNIFEPATH_INTERACTION_GROUP.html),
> [Line Cut](https://help.maxon.net/c4d/s22/us/html/TOOLKNIFELINE-MDATA_MAINGROUP.html),
> [Dissolve](https://help.maxon.net/c4d/r23/en-us/Content/html/15945.html),
> [Connect Points/Edges](https://help.maxon.net/c4d/r23/en-us/Content/html/15943.html),
> [Edges](https://help.maxon.net/c4d/s22/us/html/5492.html) (Delete in Edges mode), [Bevel](https://help.maxon.net/c4d/s22/us/html/XBEVELTOOL.html)
> and its [options](https://help.maxon.net/c4d/s22/us/html/XBEVELTOOL-MDATA_BEVEL_GROUP_OPTION.html),
> [Bridge](https://help.maxon.net/c4d/r23/en-us/Content/html/TOOLBRIDGE.html) and its
> [options](https://help.maxon.net/c4d/s22/us/html/TOOLBRIDGE-MDATA_MAINGROUP.html),
> [Weld](https://help.maxon.net/c4d/s22/us/html/TOOLPOINTWELD.html),
> [Optimize](https://help.maxon.net/c4d/2025/en-us/Content/html/5674.html), [Hide Selected](https://help.maxon.net/c4d/s22/us/html/5651.html) /
> Mesh menu (hidden elements are left alone); parameter IDs from the
> [Python SDK resource pages](https://developers.maxon.net/docs/py/2026_3_0/cinema_resource/tool/toolextrude.html)
> (`MDATA_EXTRUDE_*`, `MDATA_EXTRUDEINNER_*`). Written in our own words — no copied
> text or images.
>
> Covers implementation-plan steps **4.4** (Extrude, Inset), **4.5** (Delete, Melt,
> Dissolve, Connect, Loop/Path Cut, Line Cut) and **4.6** (Bevel, Bridge, Weld, Optimize).

## Purpose

The two tools a box-modelling session starts with: **Extrude** pushes polygons out
of (or into) the surface and builds side walls; **Inset** (called *Extrude Inner* in
older releases) shrinks polygons inside their own outline and fills the gap with a
ring of new polygons. Both work on the active polygon object.

## Common behaviour

- **Invocation**: Mesh menu, or the shortcuts **D** (Extrude) and **I** (Inset). Both
  are tools: options in the Attribute Manager's Tool mode, used in the viewport.
- **Interactive**: press and drag left/right anywhere in the viewport (Alt+drag still
  navigates). Dragging right is a positive Offset; the distance is measured in screen
  pixels at the selection's depth, so the result follows the cursor at any zoom. The
  model updates live; release = one undo step (*Extrude*, *Inset*); Esc during the drag
  cancels it. The dragged Offset is written back to the tool's options.
- **Every new drag is a new operation** on the current selection: dragging again
  extrudes the result again.
- **Numeric**: *Apply* in the Attribute Manager runs the tool once with the shown
  options on the current selection (one undo step).
- **Scope**: the selected components of the current mode; with **nothing selected,
  every visible component** (manual: "all of the object's elements"). **Hidden
  polygons are never touched**, and hidden edges are not extruded.
- **Selection afterwards**: the moved polygons stay selected (they are the same
  polygons, now at the top of the extrusion / inside the inset), so a second drag works
  on them. After an edge extrusion the new outer edges are selected instead of the old.
- **Indices**: existing points and polygons keep their indices; new ones are appended.
  So selections and hidden flags of untouched components survive.
- A click without drag, or Offset 0 on release, changes nothing and records nothing.

## Extrude

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Offset | length, any sign | 10 cm ¹ | Extrusion distance along the normals; negative goes inwards. |
| Subdivision | integer ≥ 0 | 0 | Extra rows of points along the side walls. |
| Create N-gons | bool | off | Each side wall becomes one n-gon; the subdivision points stay on its sides, without edges across. |
| Create Caps | bool | off | Also keeps a copy of the extruded polygons at the start, facing the other way, so the extrusion encloses a volume. |
| Preserve Groups | bool | on | Connected selected polygons move as one piece (walls only around the piece's outline). Off: every polygon is extruded on its own, with walls on all its edges. |
| Maximum Angle | 0°–180° | 89° ¹ | With Preserve Groups: neighbours whose normals differ by more than this are split into separate pieces. |
| Rotation (edges) | angle | 0° | Edge mode: tilts the new edge polygons around the edge, towards the polygon's normal. |
| Offset Variation | % | — | Random length per polygon: **later** (not in 4.4). |
| Bevel, Bevel Variation (points) | length, % | — | Point mode: **later** (not in 4.4). |

¹ The manual states no defaults; these are our choice.

**Polygons** —

- *Pieces*: with Preserve Groups the selected polygons are grouped by shared edges,
  but only across edges where the two normals are within Maximum Angle; without it,
  every polygon is its own piece.
- *Points*: every point a piece uses gets a moved copy — on the outline one per
  time the outline passes the point, so a piece touching itself at a point stays
  manifold; a point only the piece uses moves itself. The direction is the
  normalised sum of the normals of that piece's polygons around the point. The length
  is scaled so that each of those polygons moves by the Offset along its own normal (a
  90° corner moves √2 × Offset diagonally); the scale is capped at 4.
- *Rewiring*: the piece's polygons switch to the moved copies.
- *Walls*: every outline edge of a piece gets a wall (Subdivision + 1 quads, or one
  n-gon) connecting the original edge to the moved one. An outline edge is a mesh
  border, or the side of a polygon that is not in the piece. Walls face the same way as
  the piece.
- *Closed meshes*: extruding every polygon of a closed mesh with Preserve Groups (and
  Maximum Angle 180°) has no outline, so the mesh only inflates.

**Edges** — Only **border edges** (edges with exactly one polygon) can grow a flap;
other selected edges are ignored (our reading — an inner edge has no free side).

- *Direction*: each border edge moves perpendicular to itself, in its polygon's plane
  and away from the polygon, tilted by Rotation.
- *Chains*: where two extruded edges meet, they share the moved point, mitred so the
  strip keeps its width.
- *Faces*: each edge gets Subdivision + 1 quads (or one n-gon) that continue the
  polygon's orientation.

**Points** — not in 4.4: Extrude in Points mode shows a hint and does nothing.

## Inset (Extrude Inner)

Polygons mode only (the manual: Inset needs Polygons mode).

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Offset | length, any sign | 10 cm ¹ | Distance between the original outline and the inset one; negative grows outwards. |
| Subdivision | integer ≥ 1 | 1 | Rings of polygons between the outlines. |
| Create N-gons | bool | off | Each edge's strip of rings becomes one n-gon (no edges between the rings). |
| Preserve Groups | bool | on | Connected selected polygons are inset as one piece (only the piece's outline moves inwards). Off: every polygon is inset on its own. |
| Maximum Angle | 0°–180° | 89° ¹ | As for Extrude. |
| Offset Variation | % | — | **later**. |

**Behaviour** —

- *Pieces*: grouped as for Extrude.
- *Outline points*: every point on a piece's outline gets an inner copy, moved within
  the surface. The direction is the average of the inward perpendiculars of its two
  outline edges, each in its own polygon's plane. The distance is mitred, so both
  edges end up the Offset inside, and is capped at 4 × Offset.
- *Shared points*: a point where the outline touches itself (two corners of the piece
  meeting at one point) gets one copy per corner.
- *Rewiring*: the piece's polygons switch to the inner copies (inner points stay).
- *Rings*: every outline edge gets Subdivision quads (or one n-gon) between the
  outlines.
- Too large an Offset makes the outlines cross, as in the reference: lower the Offset.

## Shortcuts

Single keys: **0** Rectangle Selection, **Backspace / Delete** Delete, **Ctrl+Backspace**
Dissolve, **Alt+Backspace** Melt. **Chords**: press the first key, then the second
(the status bar shows the pending prefix; any other key or Esc cancels it).

| Chord | Command | Chord | Command |
|-------|---------|-------|---------|
| U L | Loop Selection | K L / M L | Loop/Path Cut |
| U B | Ring Selection | K K / M K | Line Cut |
| U W | Select Connected | M M | Connect Points/Edges |
| U Y | Grow Selection | M N | Dissolve |
| U K | Shrink Selection | M T | Extrude |
| U I | Invert | M W | Inset |
| U Z | Melt | M S | Bevel |
| U O | Optimize | M B (or B) | Bridge |

## Delete, Melt, Dissolve (components)

All three work on the selection of the current mode of the active polygon object
(nothing selected: nothing happens), as one undo step. Points that no polygon uses
any more are removed and the remaining indices close up; selections and hidden flags
follow their components.

| Mode | Delete | Melt | Dissolve |
|------|--------|------|----------|
| Points | the points and every polygon using one | each point's polygons merge into one, without the point | as Melt |
| Edges | the polygons using the edges (the manual: edges go *with* their polygons) | the two polygons of each edge merge into one; the edge's points stay | as Melt, then points left between just two edges of the merged polygon are removed |
| Polygons | the polygons | each connected group merges into one n-gon | as Melt |

- Merging needs the merged region to have a single outline (a disc): a group with a
  hole, or one touching itself at a point, is left as it is.
- A border edge (one polygon) cannot melt: there is nothing to merge with.
- Dissolve with Shift (respecting the Phong angle) is **later**.

## Connect Points/Edges

- **Points**: in every polygon with two or more selected points, the selected points
  are joined in their order around the polygon (pairs that are already neighbours need
  no new edge). Two points split the polygon in two; three or more cut out the polygon
  of the selected points plus the pieces around it.
- **Edges**: every selected edge gets a point in its middle (shared by both of its
  polygons); in every polygon those new points are then joined as for points. A polygon
  with one selected edge only gains the point. On an edge ring this cuts a loop through
  the middle.
- The new edges are selected (Edges mode); new polygons inherit the selection of the
  polygon they were cut from.

## Loop/Path Cut

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Offset | 0-100 % | 50 % (1) | Where the cut crosses the edge, from its reference end; the click sets it. |
| Number of Cuts | 1 ... | 1 | Several cuts, evenly spaced across the edges (the click position is then ignored). |
| Select Cuts | bool | on (1) | The new edges become the edge selection. |
| Mode (Path), Offset Mode (Edge Distance), Restrict to Selection, Quantize, Bidirectional, Range | - | - | **later** |

(1) Our defaults: the manual states none.

- Hover an edge: a preview shows the loop a click would cut. It runs through the
  ring of quads across the edge, both ways, until it meets a border or a non-quad
  polygon, or closes. Click: cut (one undo step).
- The reference end is carried along the ring, so the cut stays on the same side of
  every quad; on the hovered edge it is the edge's first point.
- Polygons at the ends of an open loop that are not quads gain the new points on their
  edge, so the mesh stays closed.

## Line Cut (Knife)

| Name | Type | Default | Effect |
|------|------|---------|--------|
| Visible Only | bool | on | Cut only where the surface is visible from the camera (not back sides or covered parts). |
| Infinite Cut | bool | off | The line extends beyond its ends across the whole view. |
| Connect Cut Edges | bool | on | Also split the polygons; off: only points are added on the cut edges. |
| Select Cuts | bool | on (1) | The new edges become the edge selection. |
| Slice Mode (Split, Remove Part), multi-point paths, Auto Snap, Angle Constrain | - | - | **later** |

- Drag a line across the object in the viewport (the reference's Single Line); the
  release cuts, as one undo step; Esc cancels.
- The cut is the plane through the eye and the line, limited to the wedge between the
  two ends (unless Infinite). Every edge crossing it gets a new point (a crossing at a
  point uses that point). Every polygon crossed twice is split between the two
  crossings; with more crossings they are paired along the line.

## Bevel

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Offset | length | 10 cm (1) | Width of the bevel: how far the new points move from the original edge or point along the surface. A drag in the viewport sets it. |
| Subdivision | integer ≥ 0 | 0 | 0: a flat chamfer. More: rows across the bevel, along a rounded profile. |
| Depth | -100 … 100 % | 100 % | The profile's bulge with Subdivision: 100 % round, 0 % flat, negative hollow. |
| Limit | bool | on | Clamps the Offset so that new points never pass each other (no overlapping polygons). |
| Bevel Mode (Solid), Offset Mode (Proportional, Radial), Shaping, Topology options, Polygons mode | - | - | **later** |

(1) Our default: the manual states none.

- **Interaction**: like Extrude. Pick Edges or Points mode, drag left/right anywhere
  in the viewport (the model follows live, release = one undo step, Esc cancels), or
  *Apply* with the shown options. With nothing selected, nothing happens (a bevel of
  every edge is rarely wanted).
- **Edges**: every selected edge between two polygons becomes a strip of
  Subdivision + 1 quads.
  - *Endpoints*: each endpoint of a bevelled edge is replaced by new points. Where a
    polygon's corner has one bevelled edge, its point slides the Offset along the
    other edge; where both of its edges are bevelled, the point moves into the
    polygon, the Offset away from both.
  - *Corners*: the hole left at each replaced point is filled by one polygon (a
    triangle where three bevelled edges meet, as on a cube's corner).
  - *Border edges* (one polygon) are not bevelled.
- **Points**: every selected point is cut off. Its polygons lose the corner, and a new
  polygon joins the points placed the Offset along each of its edges.
- **Selection afterwards**: the long edges of the new strips are selected (Edges
  mode).
- The rounded profile runs from one side of the strip to the other through a
  control point pulled towards the original edge (Depth). Corner polygons stay flat
  n-gons.

## Bridge

| Name | Type | Default | Effect |
|------|------|---------|--------|
| Subdivision | integer ≥ 0 | 0 | Rows of polygons along the bridge. |
| Spin Steps | integer | 0 | Turns which point of the second element each point of the first joins (fixes or makes a twist). |
| Delete Original Polygons | bool | on | Polygons mode: the two bridged polygons are removed, so the bridge opens through them. |
| Points mode (polygons point by point), chaining | - | - | **later** |

- **Polygons**: drag from one polygon to another (or select two and *Apply*). Their
  outlines are joined by a band of quads: the points pair up in opposite order, and
  the pairing with the shortest total distance is chosen (Spin Steps turns it). They
  need the same number of points and no shared point.
- **Edges**: drag from one border edge to another (or select two and *Apply*): one
  new polygon (a quad, or a triangle when the edges share a point) continues both
  neighbours' winding.
- One undo step; the new polygons are selected in Polygons mode.

## Weld

- Select points, then with the Weld tool **click one of them** to weld all selected
  points onto it, or **click anywhere else** to weld them at the centre of the
  selection. In Edges and Polygons modes the selection's points are welded.
- Polygons that collapse (fewer than three distinct points) are deleted; a point
  repeated elsewhere in a polygon removes that polygon. One undo step.
- Shift / Ctrl clicks on unselected points: **later**.

## Optimize

| Name | Type | Default | Effect |
|------|------|---------|--------|
| Points | bool | on | Merges points closer than Tolerance. |
| Tolerance | length | 0.01 cm | The merge distance. |
| Unused Points | bool | on | Removes points no polygon uses. |

- Works on the selected points; on the points of the selected edges or polygons;
  with nothing selected, on the whole object. It opens a small dialog with the options
  (Mesh menu, U~O); *OK* runs it as one undo step.
- Polygons that collapse while merging are deleted (one- and two-point polygons cannot
  exist in Grid4D, so the reference's *Polygons* option is always on).
- Merging closes a cylinder's separate caps into a rounded rim (the manual's caution).

## Grid4D mapping

| Piece | Where |
|-------|-------|
| Algorithms | `grid4d-geom::ops` — pure functions `extrude_polygons`, `extrude_edges`, `inset` over `PolyMesh` + `Topology` (+ `pieces`), each with topology-invariant tests on `shapes.rs` meshes |
| Tools | `Tool::Extrude`, `Tool::Inset`; options `ModelingSettings` (editor state) shown in the Attribute Manager's Tool mode with *Apply* |
| Shared step | `ui/src/modeling.rs`: a tool on a polygon object's selection (all visible components when none), new mesh + updated components |
| Drag | viewport `mesh_tools.rs`: rebuilds from the mesh at the press every frame (like a component transform), one `CommitLive` on release, Esc restores |
| Apply | `EditorCommand::ApplyMeshTool` → `commit` with one `SetData` |
| Menu / keys | Mesh menu *Extrude* (D), *Inset* (I), *Loop/Path Cut*, *Line Cut*, *Connect*, *Melt*, *Dissolve*; `shortcuts.rs` (single keys and chords), Help ▸ About |
| 4.5 algorithms | `grid4d_geom::ops` — `delete`, `melt`, `dissolve`, `connect`, `loop_cut`, `line_cut`, returning a `Rebuilt` mesh with the origin of every point and polygon; `selection::remap` carries selections and hidden flags across |
| 4.6 | `ops::{bevel, bridge_polygons, bridge_edges, weld, optimize}`; tools `Tool::Bevel` (drag + Apply, like Extrude), `Tool::Bridge` (drag between components + Apply), `Tool::Weld` (click); Optimize dialog (`panels/optimize.rs`) → `EditorCommand::Optimize` |
| 4.5 commands | `EditorCommand::{DeleteSelected (component modes), MeltSelected, DissolveSelected, ConnectSelected, LoopCut, LineCut}`; tools `Tool::LoopCut`, `Tool::LineCut` |

## Acceptance criteria

- [ ] Extruding the top quad of a cube (Offset 50) gives 12 points, 10 polygons, still
      closed and manifold; the top polygon is 50 higher and still selected.
- [ ] With Preserve Groups, two adjacent coplanar quads extrude as one piece (3 walls
      each side, no wall between them); without it, 8 walls and the inner edge doubled.
- [ ] Two cube faces at 90°: with Maximum Angle 89° they extrude as two pieces, with
      180° as one; in one piece each face still moves exactly the Offset.
- [ ] Subdivision 2 gives 3 rows of wall quads; Create N-gons gives one wall per edge
      with the same points.
- [ ] Create Caps on a plane's quad leaves a closed box (manifold, no border).
- [ ] Extruding every polygon of a closed mesh with Preserve Groups and 180° adds no
      polygons.
- [ ] Edge extrude of a plane's border edge adds a quad in the plane's direction; two
      neighbouring border edges share the new point; the new outer edges are selected.
- [ ] Inset of a quad (Offset 0.25 on a unit quad) gives a centred 0.5 × 0.5 quad and 4
      ring quads; Subdivision 2 gives 8 ring quads; Preserve Groups on two adjacent quads
      gives one ring of 6 quads around both (their shared edge stays inside).
- [ ] Hidden polygons are never extruded or inset, also with nothing selected.
- [ ] A drag is one undo step that restores the exact mesh and selection; Esc
      restores the mesh; a click without drag records nothing.
- [ ] Apply with the shown options gives the same result as a drag to that Offset.

- [ ] Delete in Points mode on a cube's corner removes the 3 polygons using it and the
      point; no point is left unused and the mesh stays valid.
- [ ] Delete in Edges mode removes the edge's two polygons; Melt merges them into one,
      keeping all points; Dissolve on a middle edge of a 3 × 1 plane also removes the
      points left between two edges.
- [ ] Melt of a 2 × 2 patch of quads gives one 8-point polygon without the middle
      point; Melt of a ring of quads around a hole changes nothing.
- [ ] Connect of two opposite points of a quad splits it into two triangles; Connect of
      an edge ring around a cylinder's side cuts a loop around it.
- [ ] A loop cut at 50 % on a cube's side edge adds a loop of 4 points and 4 polygons,
      all quads, still closed; 3 cuts add 12 points and 12 polygons.
- [ ] A line cut across a plane splits every crossed quad in two; with Visible Only a
      cut across a closed cube leaves its back side whole.
- [ ] Chords: U then L picks Loop Selection; U then an unbound key does nothing and
      clears the chord.
- [ ] Bevelling one edge of a cube (Offset 20 on a 200 cube) gives 8 + 2 points, 7
      polygons, still closed; with Subdivision 2 the strip has 3 quads and its middle
      points lie outside the flat chamfer.
- [ ] Bevelling the three edges at a cube corner leaves a triangle at the corner.
- [ ] Bevelling a cube's corner point gives a triangle there and 3 cut polygons.
- [ ] Bridging two opposite faces of a cube with Delete Original Polygons makes a hole
      through it: 8 points, 4 + 4 polygons, closed (a torus-like genus 1).
- [ ] Bridging two border edges of two separate quads adds one quad.
- [ ] Welding the four points of a quad's top edge pair into their centre removes
      the collapsed polygon; Weld onto a chosen point keeps that point's position.
- [ ] Optimize on a cylinder with separate caps merges the rims (16 points left of
      32) and leaves a closed mesh; points farther apart than Tolerance stay.
