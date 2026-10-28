# Spline objects

> Sources (read 2026-10-29): Cinema 4D manual —
> [Spline object](https://help.maxon.net/c4d/r21/us/html/OSPLINE-ID_OBJECTPROPERTIES.html)
> (Type: Linear, Cubic, Akima, B-Spline, Bezier; Close Spline; Intermediate
> Points: None, Natural, Uniform, Adaptive — the default —, Subdivided; Number,
> Angle, Maximum Length; the point counts of Natural and Uniform),
> [Arc](https://help.maxon.net/c4d/r21/us/html/OSPLINEARC-ID_OBJECTPROPERTIES.html),
> [Helix](https://help.maxon.net/c4d/r21/us/html/OSPLINEHELIX-ID_OBJECTPROPERTIES.html),
> [Rectangle](https://help.maxon.net/c4d/r21/us/html/OSPLINERECTANGLE-ID_OBJECTPROPERTIES.html)
> and [n-Side](https://help.maxon.net/c4d/r21/us/html/OSPLINENSIDE-ID_OBJECTPROPERTIES.html)
> (each primitive's parameters; Plane: which of three planes it lies in;
> Reverse: the opposite point order). Written in our own words. The pages give
> no default values; ours are listed below.
>
> Covers implementation-plan step **9.1** (spline primitives, drawing); 9.2
> (editable splines, point editing) and 9.3 (spline commands, the Pen) extend it.

## Purpose

Curves as objects: outlines, paths and profiles that are seen and selected in
the viewport, and that spline generators (Extrude, Lathe, Sweep, Loft) turn
into surfaces.

## The spline model

- A spline has **control points**, joined by an interpolation **Type**:
  *Linear* (straight lines), *Cubic* (smooth through the points, may
  overshoot), *Akima* (smooth through the points without overshoot), *B-Spline*
  (very smooth, the points only pull the curve) or *Bezier* (through the points,
  shaped by two tangents per point).
- A spline is made of one or more **segments**, each open or **closed** (a ring
  is two closed segments; the letters of a text several).
- **Intermediate Points** decide how the curve is cut into straight pieces for
  display and for generators: *None* (the control points only), *Natural* and
  *Uniform* (Number points between each pair of control points — at the
  control points, or evenly spaced along the curve), *Adaptive* (a new point
  where the direction turns by more than Angle — the default) and *Subdivided*
  (Adaptive, then no piece longer than Maximum Length).

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Intermediate Points | None / Natural / Uniform / Adaptive / Subdivided | Adaptive | how the curve is cut into pieces |
| Number | 0…1000 | 8 | points between control points (Natural, Uniform) |
| Angle | 0.1…90° | 5° | turn that adds a point (Adaptive, Subdivided) |
| Maximum Length | 0.1 cm… | 5 cm | longest piece (Subdivided) |

## Primitives (Create ▸ Spline)

Every primitive has **Plane** (XY — the default, facing the front view —, ZY
or XZ), **Reverse** and the Intermediate Points above. Sizes in cm, angles in
degrees; defaults are ours.

| Primitive | Parameters (defaults) | Shape |
|-----------|-----------------------|-------|
| Circle | Radius 200; Ellipse (off) with a second radius 200; Ring (off) with Inner Radius 100 | four Bezier points; Ring adds an inner circle as a second segment |
| Rectangle | Width 400, Height 400; Rounding (off), Radius 50 | four linear corners; rounded corners are Bezier quarter circles |
| Arc | Type Arc / Sector / Segment / Ring; Radius 200, Inner Radius 100, Start Angle 0, End Angle 90 | Arc: the open curve; Sector: closed through the centre; Segment: closed by the chord; Ring: the band between two radii |
| n-Side | Radius 200, Sides 6; Rounding (off), Radius 50 | a regular polygon |
| Star | Inner Radius 100, Outer Radius 200, Twist 0, Points 8 | alternating outer and inner corners |
| Helix | Start Radius 200, Start Angle 0, End Radius 200, End Angle 720, Radial Bias 50 %, Height 200, Height Bias 50 %, Subdivision 100 | a coil along the plane's normal; the biases ease the radius and the rise |
| Flower | Inner Radius 100, Outer Radius 200, Petals 6 | round petals |

## Behaviour

- **Creating**: Create ▸ Spline ▸ … or the toolbar's spline buttons; a new
  spline appears at the origin like other objects (unique names: Circle,
  Circle.1, …).
- **Display**: splines are drawn as lines in their display colour, in every
  shading mode; a selected spline is drawn in the selection colour. Splines
  are not visible in the final render (they are guides; generators make
  surfaces from them).
- **Selecting**: a click near the line (a few pixels) selects the spline;
  rectangle selection takes splines inside the frame; framing (H, O) includes
  them; the Filter menu has a Spline entry.
- **Edge cases**: zero sizes give a point-sized spline (still selectable by its
  centre); radii, sides, points and subdivisions are clamped to sensible
  ranges; a Star's inner radius may exceed the outer one (spiky inside out).

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `Spline`, `SplineType`, segments; `Interpolation`, `sample` → `Polyline`; `SplinePrimitive` (`SplineShape`, `SplinePlane`, Reverse) → `Spline` | `grid4d-geom` (`spline.rs`) |
| `ObjectKind::SplinePrimitive` (one variant; tags per shape) | `grid4d-scene` |
| `CurveKey`, cached polylines, `EvaluatedNode` curves; picking by screen distance | `grid4d-eval`, `grid4d-render` |
| Line lists per curve drawn with the instanced line pipeline; `FilterItem::Spline` | `grid4d-render` |
| `KindDto` spline primitives (additive) | `grid4d-io` (`v3.rs`) |
| Create ▸ Spline, toolbar, icons, Attribute Manager | `grid4d-ui` |

Deliberate differences: no Cissoid, Cogwheel, Cycloid, Formula, Profile,
4-Side or Spiral primitives yet; splines are not exported to glTF or OBJ.

## Acceptance criteria

- [x] Each primitive gives the expected control points (a circle's points lie
      on the circle, a Bezier circle's samples within 0.1 % of the radius).
- [x] Natural/Uniform give the documented point counts; Adaptive adds points
      only where the curve turns; Subdivided bounds piece length.
- [x] Splines are created from the menu and toolbar, drawn, picked, framed,
      filtered, undone and saved.
- [x] Idle frames with splines change nothing; the path tracer ignores them.
