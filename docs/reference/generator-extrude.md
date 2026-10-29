# Extrude generator

> Sources (read 2026-10-29): manual —
> [Extrude Object](https://help.maxon.net/c4d/en-us/Content/html/OEXTRUDE.html),
> [its Object properties](https://help.maxon.net/c4d/en-us/Content/html/OEXTRUDE-ID_OBJECTPROPERTIES.html),
> [Caps and bevels (R23)](https://help.maxon.net/c4d/r23/en-us/Content/html/OEXTRUDE-ID_CAPSANDBEVELS.html).
> Hole bridging for the caps: D. Eberly, *Triangulation by Ear Clipping* (2002,
> public). Written in our own words — no copied text or images.
>
> Covers implementation-plan step **9.4**. Depends on
> [`object-spline.md`](object-spline.md) (splines) and
> [`generator-subdivision-surface.md`](generator-subdivision-surface.md) (the
> generator framework).

## Purpose

Turns a flat outline into a solid: the spline under the generator is pushed
along a direction, its path becomes the wall (the *shell*), and closed outlines
get flat lids (the *caps*). A Circle becomes a coin or a tube, a Rectangle a
slab, a rectangle with a hole a frame, and later (9.8) text becomes 3D letters.

## Generator behaviour

- **Input:** the generator's **first child**, which must be a spline (a spline
  primitive or an editable spline). With **Hierarchical** on, every visible,
  enabled spline in the first child's subtree is extruded, each on its own (so
  a Null holding several splines works, and letters never cut holes into each
  other). Anything that is not a spline gives nothing.
- **Segments and holes:** one spline may have several segments. Its closed
  segments are sorted by nesting (even-odd): an outline inside one other outline
  is a **hole**, one inside a hole is an island again. Each outline's caps have
  its direct holes cut out. Open segments get a shell and never a cap.
- **The input stays visible:** splines are curves, so the input spline is still
  drawn and picked as a line (click it to select it and edit its points); the
  generator draws the solid. (The Subdivision Surface hides its cage; splines
  under any generator stay drawn.)
- **Orientation:** whatever way the outline was drawn, the shell and caps face
  outwards (outlines run counter-clockwise and holes clockwise around the
  direction of the extrusion). Open segments face the side their drawing
  direction gives; **Reverse Normals** flips them (and the shell of closed ones),
  never the caps.
- **One mesh:** the shell's first and last rings are the caps' points, so a
  closed outline gives a closed, welded solid. Hard edges between caps and shell
  come from the Phong angle (80° by default, a Phong tag on the generator — or
  on the spline — changes it).
- **Enabled / `Q` / Alt + create / Make Editable** work as for every generator:
  a disabled Extrude shows nothing but its children; Alt while creating wraps
  the selected spline; Make Editable bakes the solid into one polygon object and
  removes the input spline.
- **Rendering:** the path tracer renders the solid like any polygon object.

## Parameters

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Direction | Auto · X · Y · Z · Custom · Absolute | Auto | How the movement is found. *Auto*: along the generator's axis (X, Y or Z) closest to the perpendicular of the spline's plane, so a spline in the XY plane goes along +Z. *X / Y / Z*: along that axis. *Custom*: along Movement (only its direction counts). *Absolute*: exactly Movement. All in the generator's own space. |
| Offset | cm, any (negative reverses) | 20 cm | Length of the movement for Auto, X, Y, Z and Custom |
| Movement | vector, cm | (0, 0, 20) | Custom: the direction; Absolute: the movement itself |
| Subdivision | 1 – 4000 | 1 | Rings of polygons along the movement |
| Reverse Normals | bool | off | Turns the shell inside out (for open splines); caps untouched |
| Hierarchical | bool | off | Extrude every spline in the first child's subtree, each on its own |
| Start Cap / End Cap | bool | on / on | Lids at the start (facing back) and at the end (facing forward) |
| Caps Type | N-Gons · Triangles | N-Gons | *N-Gons*: one polygon per outline without holes, triangles where holes need them. *Triangles*: every cap triangulated |

Not in 9.4 (later): bevels and their shapes, Delaunay / Regular Grid caps,
Isoparm display, Separate Bevel Controls, Break Phong Rounding, selection tags
for the caps.

## Edge cases

- No first child, a first child that is not a spline, an empty spline: the
  generator shows nothing (and framing it frames nothing).
- A movement of zero length: nothing (the solid would be flat).
- Segments with fewer than two points: skipped. Closed segments need three
  points for a cap.
- Outlines that cross themselves or each other: the shell is still made; caps
  are triangulated as well as they can be, never failing (ear clipping falls back
  to a fan).
- Non-planar outlines (a Helix): the shell follows the curve; caps are only
  made for closed segments and triangulated in the plane across the movement.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `Extrusion` (direction, offset, movement, subdivision, reverse normals, caps, cap type), `ExtrudeDirection`, `CapType`; `extrude(&Polyline, &Extrusion) -> PolyMesh` | `grid4d-geom::surface` (pure, tests on hand-built curves) |
| Outlines with holes: nesting (`outline_groups`) and triangulation (`triangulate_outlines`: hole bridging + ear clipping) | `grid4d-geom::surface`, `grid4d-geom::triangulate` |
| `ObjectKind::Extrude { extrusion, hierarchical }`, tag `Extrude`, `GENERATORS` | `scene/node.rs` |
| Spline inputs (primitive parameters or editable spline id + interpolation, matrix into the generator), cache, the input spline left drawn | `grid4d-eval::generator`, `evaluated.rs` |
| `.g4d` v3 `extrude` kind (additive; defaults omitted) | `grid4d-io::v3` |
| Icon, Create ▸ Generator ▸ Extrude, toolbar, Attribute Manager (Object and Caps groups), filter entry *Generator* | `grid4d-ui` |

## Decisions (2026-10-29)

1. **Defaults** follow the manual's ranges and the long-standing convention:
   Auto, 20 cm, one subdivision, both caps, N-Gons.
2. **Auto** picks an axis of the generator (not the exact plane normal), and
   always its positive direction, so the result does not flip when the outline
   is drawn the other way round.
3. **Without Hierarchical** only the first child itself is extruded (its own
   children are ignored); with it, each spline of the subtree separately.
4. **Make Editable** gives one polygon object (shell and caps welded), not a
   group of caps and shell.

## Acceptance criteria

- [x] A Circle under an Extrude gives a closed, welded cylinder: shell rings ×
      subdivision, two n-gon caps, every face outward (positive volume).
- [x] A spline with an outline and a hole gives caps with the hole cut out
      (triangulated) and a shell on both outlines facing outwards; drawing the
      outlines either way round gives the same faces' directions.
- [x] Directions: Auto on XY / ZY / XZ splines moves along Z / X / Y; X, Y, Z,
      Custom (normalised × offset) and Absolute; a negative offset reverses
      and still faces outwards.
- [x] Open splines get only a shell; Reverse Normals flips it.
- [x] Caps Type Triangles makes every cap face a triangle; caps off leave the
      solid open.
- [x] Hierarchical extrudes every spline of the subtree; without it only the
      first child.
- [x] The result is rebuilt only when the spline, its placement under the
      generator or the options change; idle frames do nothing.
- [x] Make Editable bakes the solid and removes the input; Alt + create wraps
      the selected spline; both one undo step.
- [x] Saved and loaded with every parameter; older files still load.
- [x] The path tracer renders it.
