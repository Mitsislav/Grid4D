# Lathe generator

> Sources (read 2026-10-29): manual —
> [Lathe Object](https://help.maxon.net/c4d/en-us/Content/html/OLATHE.html),
> [its Object properties](https://help.maxon.net/c4d/en-us/Content/html/OLATHE-ID_OBJECTPROPERTIES.html),
> [Caps (S22)](https://help.maxon.net/c4d/s22/us/html/OLATHE-ID_CAPSANDBEVELS.html).
> Written in our own words — no copied text or images.
>
> Covers implementation-plan step **9.5**. Builds on
> [`generator-extrude.md`](generator-extrude.md) (spline inputs, caps with holes).

## Purpose

Turns a profile into a body of revolution: the spline under the generator is
spun around the generator's Y axis, like wood on a lathe. Half the outline of a
glass, a vase or a bottle becomes the whole object; with a movement along the
axis the spin becomes a helix (screw threads, springs).

## Generator behaviour

- **Input:** the generator's first child spline (a primitive or an editable
  spline), in the generator's space. Draw the profile in the XY plane, to one
  side of the Y axis (Grid4D accepts any plane through Y: it measures each
  point's distance from the axis).
- **The axis** is the generator's own Y axis through its origin: move or turn
  the spline under the Lathe to change where the profile sits.
- **A full turn** (Angle ±360°, no Movement, Scaling 100 %) closes on itself: the
  last ring is the first, and points on the axis are shared, so a profile that
  starts and ends on the axis gives a closed, welded body.
- **A partial turn, a Movement or a Scaling** leaves the two ends open: closed
  profiles get a **start cap** and an **end cap** (with holes, like the
  Extrude's); open profiles stay open.
- **Orientation:** surfaces face away from the axis for an outline drawn either
  way round, and for an open profile as if the area between it and the axis
  were solid (a vase's wall faces out and its bottom down). **Reverse Normals**
  flips the shell, never the caps. A negative angle turns the other way and
  still faces outwards.
- **The input stays visible** as a line; Q, Alt + create, Make Editable and
  rendering work as for every generator.

## Parameters

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Angle | degrees, any | 360° | How far the profile turns; more than 360° winds round again (useful with Movement) |
| Subdivision | 3 – 4000 | 24 | Steps along the turn |
| Movement | cm, any | 0 cm | Distance along Y from start to end: a helix instead of a circle |
| Scaling | 0 % – any | 100 % | Size of the profile at the end, relative to the generator's origin (grows or shrinks along the turn) |
| Reverse Normals | bool | off | Turns the shell inside out (caps untouched) |
| Start Cap / End Cap | bool | on / on | Lids where an open turn starts and ends (closed profiles only) |
| Caps Type | N-Gons · Triangles | N-Gons | As for the Extrude |

Not in 9.5 (later): Isoparm display, bevels, Delaunay / grid caps, UVs.

## Edge cases

- No spline, an empty spline: nothing.
- Angle 0 with no Movement and Scaling 100 %: nothing (the body would be flat).
- Points on the axis are welded only while they stay put (no Movement, Scaling
  100 %); faces that collapse there become triangles or are left out.
- A profile crossing the axis folds over itself: the surface is still made.
- Angles beyond 360° without Movement overlap themselves: made as asked.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `Lathe` (angle in radians, subdivision, movement, scaling, Reverse Normals, caps, cap type), `lathe(&Polyline, &Lathe) -> PolyMesh`; shared outline nesting and caps with the Extrude | `grid4d-geom::surface` |
| `ObjectKind::Lathe(Lathe)`, tag `Lathe`, `GENERATORS` | `scene/node.rs` |
| `Options::Lathe`, the first child spline as input | `grid4d-eval::generator` |
| `.g4d` v3 `lathe` kind (additive; defaults omitted; subdivision clamped, scaling ≥ 0) | `grid4d-io::v3` |
| Icon, Create ▸ Generator, toolbar, Attribute Manager rows | `grid4d-ui` |

## Decisions (2026-10-29)

1. **Defaults**: 360°, 24 subdivisions, no movement, 100 % — the long-standing
   convention; the manual states none.
2. **Caps on by default**: they only appear where a closed profile's turn stays
   open, so a full turn is unaffected.
3. **Scaling** scales the whole profile uniformly about the generator's origin,
   growing linearly along the turn.
4. **Hierarchical** is not offered (the reference has none for the Lathe): only
   the first child is turned.

## Acceptance criteria

- [x] A rectangle beside the axis turned 360° gives a closed, welded ring
      (a pipe section) of `24 × 4` quads facing outwards (positive volume),
      whichever way the rectangle was drawn.
- [x] An open profile from the axis out and up (a cup) gives a closed bottom
      (triangles at the axis, one shared centre point) and an outward wall.
- [x] 180° closes a closed profile with two caps; a negative angle still faces
      outwards; Reverse Normals flips the shell only.
- [x] Movement raises the end ring by that much; Scaling 50 % halves its
      distance from the axis.
- [x] Saved and loaded; the result rebuilt only when inputs change; Make
      Editable and Alt + create are one undo step each; rendered.
