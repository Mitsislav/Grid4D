# Spline Boolean generator

> Sources: the reference's spline-combining object was not read for this
> spec; the four modes are the plain set of 2D set operations. Method from
> public work, our own code: Martínez, Rueda and Feito, *A new algorithm for
> computing Boolean operations on polygons* (2009) — a sweep over the
> segments' endpoints with exact orientation tests (the same predicates as
> the mesh Boolean, 17.2). Written 2026-11-09 in our own words.
>
> Covers implementation-plan step **17.5**. Decisions D40, D45. Name (R8):
> **Spline Boolean**, plain and descriptive.

## Purpose

Combines closed splines as flat shapes — united, cut, intersected or
exclusive — producing a **spline**, live. The result feeds an Extrude, Lathe,
Sweep or Loft like any spline: a logo outline made from circles and
rectangles, a gasket with holes, a cookie-cutter profile.

## Parameters

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Mode | Union · Subtract · Intersect · Exclusive | Union | Union: inside any. Subtract: the first child minus all the others. Intersect: inside all. Exclusive: inside an odd number of them. |
| Plane | XY · ZY · XZ (the generator's own axes) | XY | The plane the shapes are combined in. |

## Behaviour

- **Inputs**: every visible child spline (spline primitives, editable
  splines, Text, and — D45 — another generator that produces a curve, such as
  a nested Spline Boolean), in order. Each is sampled as it is drawn (its
  interpolation), placed in the generator's space and **flattened** onto the
  plane (the off-plane coordinate dropped). Only **closed** segments count;
  open ones are ignored. A child's own nested segments (a letter's hole)
  count by even-odd, as the Extrude reads them.
- **Result**: a spline in the plane — closed segments, outlines
  counter-clockwise and holes clockwise when seen from the plane's front —
  of type Linear (its points are the combined outlines' points). Points
  closer than 0.001 cm are merged; segments smaller than that vanish.
- **The first generator whose result is a curve**: the viewport draws it as
  a spline (picked like one), and Extrude, Lathe, Sweep and Loft read it as a
  child spline. The children are hidden while it is enabled.
- Overlapping, touching and coincident edges are handled exactly; a degenerate
  input (fewer than three distinct points) is ignored.
- Make Editable bakes it into an editable spline (Linear); Current State to
  Object copies it; export leaves it out like every spline.

## Generators reading a generator's curve (D45)

- A spline generator's spline input may be **any child that evaluates to a
  curve**: a spline object as before, or a generator whose result is a curve
  (in 1.3 only the Spline Boolean). Evaluation builds the inner generator
  first and hands its cached curve to the outer one, keyed by its curve id.
- Points mode on such a result is not available (it is not an editable
  spline); Make Editable makes it one.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `ObjectKind::SplineBoolean(SplineBoolean)`, `ObjectKindTag::SplineBoolean` in `GENERATORS` and `SPLINES` (it is drawn as one), label, icon, page | `scene/node.rs`, `ui/icons.rs`, `ui/panels/attributes.rs` |
| `spline_boolean(outlines, mode) -> Vec<closed polylines>` | `grid4d-geom::spline` (pure; shares the exact predicates) |
| Curve results of generators (`CurveHandle` per generator), spline generators reading them | `grid4d-eval::{generator, curve}` |
| File | `.g4d` v6 kind `spline_boolean` (17.5) |

## Decisions made while implementing (2026-11-24)

1. **The method.** Not the endpoint sweep: every edge is cut where it
   crosses or touches another (crossings decided by exact orientation
   tests, the crossing point computed in `f64`), points within 0.001 cm
   merged and equal edges made one, owned by the shapes that have it an odd
   number of times. Each edge is then classified by a ray from its middle
   (the shapes it crosses an odd number of times hold that side; the other
   side differs by the edge's own shapes); edges where the mode's rule holds
   on one side only are kept, turned so the result is on their left, and
   chained into loops turning as sharply right as possible, so shapes that
   touch at a corner stay two outlines. Simpler than the sweep, with the
   same exact predicates; a hundred overlapping circles of 128 points take
   71 ms.
2. An outline with no area (fewer than three points, or all in a line)
   counts as no shape: it never empties an Intersect.
3. Subtract's "first child" is the first child that has a closed outline.
4. It is listed with the generators, not the spline primitives, and is not
   a spline itself (Points mode needs Make Editable).
5. A spline generator above it rebuilds its curve from the inner inputs it
   gathers (as a Boolean does with a generator inside its input), so one
   comparison of inputs still decides every rebuild.
6. Its children, splines included, are hidden while it is enabled; disabled
   it is a group and they show as they are.
7. Export writes it as the editable spline it shows, which glTF and OBJ
   leave out like every spline.

## Acceptance criteria (all met in 17.5)

- [x] Two overlapping circles: Union gives one outline, Subtract a crescent,
      Intersect a lens, Exclusive two crescents; areas within 1e-3 of the
      analytic values at fine sampling.
- [x] A rectangle minus a circle inside it gives an outline and a hole; an
      Extrude of the generator gives a plate with a round hole and caps.
- [x] Coincident edges and touching corners give clean outlines (no zero-area
      slivers), and the fuzzer finds no panic (`fuzz/fuzz_targets/spline_boolean.rs`,
      in CI).
- [x] Open child segments are ignored; a child out of the plane is flattened.
- [x] Moving a child updates the Extrude above in the same frame.
- [x] Make Editable gives a Linear editable spline with the same points.
- [x] Saved as v6 and reopened the same; a v5 file naming it is refused.
