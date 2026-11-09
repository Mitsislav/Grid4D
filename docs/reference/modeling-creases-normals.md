# Edge creases, hard edges and weighted normals

> Sources: the reference's Subdivision Surface page (read for step 4.7)
> describes weights on points, edges and polygons that pull the smooth
> surface toward the cage; its Phong tag page (step 6.1) the smoothing angle.
> Methods from public papers, our own code: DeRose, Kass and Truong,
> *Subdivision surfaces in character animation* (1998) — semi-sharp creases;
> Thürrner and Wüthrich, *Computing vertex normals from polygonal facets*
> (1998) — angle-weighted normals, here multiplied by area. Written
> 2026-11-09 in our own words.
>
> Covers implementation-plan step **17.8**. Decisions D42, D43. Names (R8):
> **creases** and **hard edges** (the reference's *weights* and *breaks*),
> **weighted normals** — all plain terms. Replaces the planned
> `modeling-sds-weights.md`.

## Purpose

Sharp corners on smooth subdivided models without extra edge loops
(creases), and crisp, clean shading on low-poly hard-surface models (hard
edges, weighted normals).

## Edge creases

- Each edge may carry a **crease** from 0 % (smooth, as today) to 100 %
  (sharp at every level). Stored on the polygon object next to the mesh, by
  corner like UV seams (an edge's crease is the largest of its corners'),
  carried across rebuilding operations by `Rebuilt::carry` (a split edge's
  pieces keep it; an edge made of merged ones takes the largest).
- **The Subdivision Surface honours them** with the semi-sharp rules: an edge
  of crease *c* has sharpness *s* = 6 · *c* (6 being the deepest level); at
  each level an edge with *s* ≥ 1 is subdivided with the crease rules (its
  edge point the midpoint, its points moved along the crease only), an edge
  with 0 < *s* < 1 blends the smooth and crease rules by *s*, and the child
  edges get *s* − 1. A point on two or more creased edges follows the
  crease rule; on three or more it stays as a corner. 100 % stays sharp at
  every level.
- **Editing**: Mesh ▸ Crease… sets the selected edges' crease to a value
  (one undo step); the **Crease tool** sets it by dragging left and right in
  the view (live, one undo step per drag). Edges mode shows creased edges
  tinted by their value.
- Creases have no effect without a Subdivision Surface (and on export, which
  bakes the subdivided mesh).

## Hard edges

- Each edge may be **hard**: shading is split along it even where the Phong
  angle would smooth across. Stored by corner next to the mesh like seams,
  carried like them.
- **Editing**: Mesh ▸ Normals ▸ Make Hard / Make Soft (selected edges),
  Hard from Angle (every edge whose faces meet at more than the Phong
  angle), Clear Hard Edges. Edges mode shows them in their own colour.
- The viewport, renders and **glTF / OBJ export** (normals written per
  corner) all use the same split, through the triangulator's smoothing.
- A Boolean marks its cut edges hard (its option).

## Weighted normals

- The **Phong tag** gains **Weighted Normals** (off by default): a smooth
  point's normal is the sum of its faces' normals weighted by each face's
  **area × the corner's angle**, so large flat faces stay flat and the
  shading bends only on the narrow bevels between them. Hard edges and the
  Phong angle still split.
- Off, normals are as in 1.2 (angle-weighted only).

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `ComponentState::{creases: CreaseSet (corner → percent, sparse), hard: BitSet}`, carried by `Rebuilt::carry` | `scene/mesh.rs`, `grid4d-geom::ops` |
| Semi-sharp Catmull-Clark; `SubdivisionOptions` takes per-edge sharpness | `grid4d-geom::subdivision` |
| `Smoothing` with hard edges and weighting; the triangulator's normals | `grid4d-geom::{normals, triangulate}`, `Tag::Phong { weighted }` |
| `EditorCommand::{SetCrease, SetHardEdges, HardFromAngle}`, the Crease tool, the edge colours | `ui/src/commands.rs`, `ui/src/modeling.rs`, `grid4d-render::components` |
| File | `.g4d` v6 `polygon` kind `creases` (groups of edge ranges with a percent), `hard` (edge ranges); `phong` tag `weighted` (17.8) |

## Acceptance criteria

- [ ] A cube cage with all edges at 100 % subdivided twice keeps the cube's
      corners and edges exactly; at 0 % it is today's rounded box; at 50 %
      in between (monotonic in the crease).
- [ ] Creases and hard edges survive extrude, bevel, cut, delete and
      Optimize on their edges (property tests).
- [ ] A hard edge splits normals within the Phong angle; Hard from Angle
      marks a cube's 12 edges and none of a smooth sphere's.
- [ ] Weighted normals on a bevelled box: the big faces' normals equal their
      face normal within 1°.
- [ ] glTF and OBJ export write the split normals; reimported, the same.
- [ ] Each command is one undo step; saved as v6 and reopened the same; a v5
      file carrying `creases`, `hard` or `weighted` reads without them.
