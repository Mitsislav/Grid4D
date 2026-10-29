# Loft generator

> Sources (read 2026-10-29): manual —
> [Loft Object](https://help.maxon.net/c4d/en-us/Content/html/OLOFT.html),
> [its Object properties](https://help.maxon.net/c4d/en-us/Content/html/OLOFT-ID_OBJECTPROPERTIES.html),
> [Caps (R23)](https://help.maxon.net/c4d/r23/en-us/Content/html/OLOFT-ID_CAPSANDBEVELS.html).
> Interpolation across sections: Catmull-Rom and uniform cubic B-spline curves
> (public, textbook). Written in our own words — no copied text or images.
>
> Covers implementation-plan step **9.7**. Builds on
> [`generator-extrude.md`](generator-extrude.md) (caps).

## Purpose

Stretches a skin over a series of cross-sections: circles of different sizes
become a vase or a bottle, a circle and a square a transition piece, a row of
outlines a boat hull or an aircraft fuselage.

## Generator behaviour

- **Inputs:** every child of the generator that is a visible, enabled spline,
  in Object Manager order, read in the generator's space (move a section to
  move that part of the skin). Each spline's first segment is its section.
  Children that are not splines are ignored.
- **Around (U):** each section is resampled into the same number of points by
  length along it, starting at its first point (use Set First Point to line the
  sections up). Open sections give `Subdivision U` points; closed ones one
  fewer, since the last would coincide with the first. Closed sections are
  turned to run the same way round, so the skin does not twist into a bow tie.
- **Along (V):** the skin passes exactly through every section and curves
  smoothly between them (Catmull-Rom); **Linear Interpolation** joins them with
  straight lines instead. **Organic Form** uses the sections as control shapes
  of a looser, smoother surface that no longer passes exactly through them
  (except the first and the last). `Subdivision V` rings are spread over the
  whole length by the distance between sections, or evenly per section pair
  with **Per Segment**; every section gets a ring.
- **Loop** joins the last section back to the first: a closed ring of skin
  (no caps).
- **One section** alone is closed and covered with a flat cap.
- **Caps:** closed sections get a start and an end cap (N-Gons or Triangles)
  unless looped. Surfaces face outwards when the sections run
  counter-clockwise around the direction from the first section to the last
  (they are turned that way); **Reverse Normals** flips the skin only.
- Q, Alt + create (wraps the selected objects: every selected object, in
  selection order, becomes a section), Make Editable (one polygon object, all
  sections removed) and rendering work as for every generator; the sections
  stay drawn.

## Parameters

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Subdivision U | 2 – 4000 | 30 | Points around each section (closed: one fewer) |
| Subdivision V | 2 – 4000 | 10 | Rings along the skin (at least one per section) |
| Organic Form | bool | off | A looser surface that does not pass through the inner sections |
| Per Segment | bool | off | Rings spread evenly per section pair, not by distance |
| Loop | bool | off | Joins the last section to the first |
| Linear Interpolation | bool | off | Straight lines between sections instead of curves |
| Reverse Normals | bool | off | Turns the skin inside out (caps untouched) |
| Start Cap / End Cap, Caps Type | as the Extrude | on, on, N-Gons | |

Not in 9.7 (later): Adapt UV and UVs, Isoparm display, sections with several
segments (holes), bevels.

## Edge cases

- No spline child or only empty ones: nothing.
- Sections with fewer than two points are skipped; open and closed sections
  mixed: the first section decides, the others are read as it is.
- Sections of zero length (a point) are allowed: every resampled point is that
  point (a cone's tip).

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `Loft` options, `loft(&[Polyline], &Loft) -> PolyMesh` (resampling, alignment, interpolation, caps shared) | `grid4d-geom::surface` |
| `ObjectKind::Loft(Loft)`, tag `Loft`, `GENERATORS`; `generator_inputs` = every child | `scene/node.rs` |
| `Options::Loft`: every spline child, in order, in the generator's space | `grid4d-eval::generator` |
| Alt + create wraps every selected object; Make Editable removes all children | `ui/src/commands.rs` |
| `.g4d` v3 `loft` kind (additive; defaults omitted; counts clamped) | `grid4d-io::v3` |
| Icon, Create ▸ Generator, toolbar, Attribute Manager rows | `grid4d-ui` |

## Decisions (2026-10-29)

1. **Defaults** U 30, V 10 (the long-standing values; the manual states none);
   caps on.
2. **Organic Form** is read as "the skin approximates the inner sections"
   (a uniform cubic B-spline with clamped ends); off, the skin interpolates
   them (Catmull-Rom).
3. **Winding**: closed sections are turned to one direction around the loft;
   start points are kept as drawn.

## Acceptance criteria

- [x] Two circles of radius 100 and 50, 200 apart, give a closed, capped
      frustum facing outwards, whichever way each circle runs.
- [x] The skin passes through every section; with Organic Form it does not
      pass through an inner one; Linear gives straight segments.
- [x] Subdivision U and V decide the counts (closed: U − 1 around); Per Segment
      spreads the rings evenly.
- [x] Loop closes the skin without caps; a single section is a flat cap.
- [x] Saved and loaded; rebuilt only when inputs change; Alt + create wraps
      several selected splines; Make Editable removes every section in one
      undo step; rendered.
