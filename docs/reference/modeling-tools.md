# Modelling tools — Extrude, Inset (more to follow)

> Sources (read 2026-10-19): Cinema 4D manual —
> [Extrude](https://help.maxon.net/c4d/en-us/Content/html/TOOLEXTRUDE.html) and its
> options ([S22 page](https://help.maxon.net/c4d/s22/us/html/TOOLEXTRUDE-MDATA_MAINGROUP.html)),
> [Extrude Inner / Inset](https://help.maxon.net/c4d/s22/us/html/TOOLEXTRUDEINNER.html) and its
> [options](https://help.maxon.net/c4d/s22/us/html/TOOLEXTRUDEINNER-MDATA_MAINGROUP.html),
> [Quick Start](https://help.maxon.net/c4d/en-us/Content/html/CINEMA_4D_QUICKSTART.html)
> (shortcuts D and I), [Hide Selected](https://help.maxon.net/c4d/s22/us/html/5651.html) /
> Mesh menu (hidden elements are left alone); parameter IDs from the
> [Python SDK resource pages](https://developers.maxon.net/docs/py/2026_3_0/cinema_resource/tool/toolextrude.html)
> (`MDATA_EXTRUDE_*`, `MDATA_EXTRUDEINNER_*`). Written in our own words — no copied
> text or images.
>
> Covers implementation-plan step **4.4**. Bevel, cuts, Bridge, Weld (4.5–4.6) will be
> added here.

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

## Grid4D mapping

| Piece | Where |
|-------|-------|
| Algorithms | `grid4d-geom::ops` — pure functions `extrude_polygons`, `extrude_edges`, `inset` over `PolyMesh` + `Topology` (+ `pieces`), each with topology-invariant tests on `shapes.rs` meshes |
| Tools | `Tool::Extrude`, `Tool::Inset`; options `ModelingSettings` (editor state) shown in the Attribute Manager's Tool mode with *Apply* |
| Shared step | `ui/src/modeling.rs`: a tool on a polygon object's selection (all visible components when none), new mesh + updated components |
| Drag | viewport `mesh_tools.rs`: rebuilds from the mesh at the press every frame (like a component transform), one `CommitLive` on release, Esc restores |
| Apply | `EditorCommand::ApplyMeshTool` → `commit` with one `SetData` |
| Menu / keys | Mesh menu *Extrude* (D), *Inset* (I); `shortcuts.rs`, Help ▸ About |

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
