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
> Covers implementation-plan steps **9.1** (spline primitives, drawing),
> **9.2** (editable splines, point editing) and **9.3** (spline commands, the
> Pen).

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

## Editing a spline (step 9.2)

- **Make Editable** (C) turns a spline primitive into an editable **Spline**
  object with the same control points, tangents, type, segments and
  intermediate points (the plane and Reverse are baked in); one undo step.
- **Points mode** on the active editable spline shows its control points; a
  **Bezier** spline also shows each selected point's two **tangent handles**.
  - A click on a point selects it (Shift adds, Ctrl removes, empty space
    clears); the Rectangle and Live Selection tools select the points inside
    the frame or under the brush; Select All, Deselect All and Invert work on
    the points.
  - The **gizmo** sits at the centre of the selected points; Move, Scale and
    Rotate act on them, turning and scaling their tangents with them.
  - **Dragging a tangent handle** sets that tangent in the view's plane; the
    opposite one turns to stay in line, keeping its length, so the curve stays
    smooth. With **Shift** only the dragged one moves (a corner). Esc cancels.
  - **Delete** removes the selected points; a segment left without points
    disappears.
- **Object tab** of an editable spline: **Type** (changing to Bezier gives
  points without tangents smooth ones, leaving others; changing away keeps the
  tangents for a later return), **Close Spline** (every segment), the
  Intermediate Points, and the number of points and segments.
- Every edit is one undo step; the selected points are saved with the document.

## Spline commands and the Pen (step 9.3)

- **Mesh ▸ Spline** (on the active editable spline; the point commands work on
  its selected points):
  - **Hard Interpolation**: the points lose their tangents — sharp corners.
  - **Soft Interpolation**: the points get smooth tangents again (from their
    neighbours), whatever they had.
  - **Reverse Sequence**: the segments with selected points (all segments
    when none is selected) run the other way; the curve looks the same.
  - **Set First Point**: in a closed segment, the selected point becomes its
    first (the curve is unchanged; generators start there).
  - The selection follows the points; each command is one undo step.
- **The Pen** (Create ▸ Spline ▸ Pen; a tool with a **Type** option, Bezier
  by default):
  - With an editable spline active it adds points to its end; otherwise the
    first click makes a new Spline object (selected) at the origin.
  - A **click** adds a point (for Bezier: without tangents, a corner); a
    **click and drag** adds a point and pulls its tangents (smooth, the dragged
    one outgoing).
  - Points go on the **grid's plane**: the floor (XZ) in perspective views,
    the view's own plane through the origin in front, side and top views.
  - A click on the first point of the open segment being drawn (with at least
    three points) **closes** it and finishes; **Esc** or **Enter** finishes;
    so does choosing another tool or object. A line from the last point to the
    cursor shows where the next segment goes.
  - Each added point is one undo step.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `Spline`, `SplineType`, segments; `Interpolation`, `sample` → `Polyline`; `SplinePrimitive` (`SplineShape`, `SplinePlane`, Reverse) → `Spline` | `grid4d-geom` (`spline.rs`) |
| `ObjectKind::SplinePrimitive` (one variant; tags per shape) | `grid4d-scene` |
| `CurveKey`, cached polylines, `EvaluatedNode` curves; picking by screen distance | `grid4d-eval`, `grid4d-render` |
| Line lists per curve drawn with the instanced line pipeline; `FilterItem::Spline` | `grid4d-render` |
| `KindDto` spline primitives (additive) | `grid4d-io` (`v3.rs`) |
| Create ▸ Spline, toolbar, icons, Attribute Manager | `grid4d-ui` |
| `SplineHandle` (shared, unique id) and `ObjectKind::Spline { spline, interpolation, components }`; Make Editable | `grid4d-scene`, `grid4d-ui` (`commands.rs`) |
| Spline points in Points mode: picking and the overlay of points and handles (2D), selection commands, gizmo point edits, tangent drags, Delete | `grid4d-ui` (`panels/viewport/spline_tools.rs`, `gizmo.rs`, `commands.rs`) |
| `editable_spline` kind (points, tangents, segments, selection) | `grid4d-io` (`v3.rs`) |
| `Spline::{hard, soft, reverse_segments, set_first_points}` (pure, with index maps) | `grid4d-geom` (`spline.rs`) |
| `EditorCommand::{SplineCommand, PenPoint, PenClose, PenFinish}`, `SplineOp`, `Tool::Pen`, `PenSettings` | `grid4d-ui` (`commands.rs`, `editor.rs`, `panels/viewport/pen_tool.rs`) |

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
- [x] Make Editable keeps the curve exactly (same samples) and is one undo step.
- [x] Points are selected by click, rectangle and brush; the gizmo moves, scales
      and rotates them (tangents along); handles drag mirrored or broken.
- [x] Delete removes points; Type and Close Spline change the curve; all
      undoable and saved.
- [x] Hard, Soft, Reverse Sequence and Set First Point change the spline as
      described, keep the selection on the same points and are undoable.
- [x] The Pen makes a spline from clicks and drags on the grid's plane, adds
      to an active spline, closes on its first point and finishes with Esc.
