# Viewport Filter menu

> Sources (read 2026-10-21): Cinema 4D manual —
> [Filter menu](https://help.maxon.net/c4d/s22/us/html/11082.html) (each entry shows
> or hides one kind of object or one viewport element: Null, Polygon, Spline,
> Generator, Subdivision Surface, Deformer, Camera, Light, Joint, Particle, Field,
> Hair, Environment, Other; Workplane, World Grid, Axis, World Axis, Multi-Select
> Axes, Handles, highlighting, SDS Mesh / Cage, N-gon Lines, Guides, Animation
> Path, Ghosting, HUD, Horizon; everything is shown by default; choosing an entry
> toggles it; Ctrl/Cmd + choosing keeps only that entry; All, None and Default
> buttons; Geometry Only; selecting a hidden object in the Object Manager still
> shows its axis, and it can be moved, scaled and rotated) and
> [Filter, older text](https://help.maxon.net/c4d/r21/us/html/11082.html).
> Written in our own words.
>
> Covers implementation-plan step **7.5**.

## Purpose

Uncluttering one view: hiding the cameras and lights while modelling, showing
only polygon objects, or hiding the grid and HUD for a clean picture — in that
view only.

## Behaviour

Each view has its own **Filter** menu; everything is shown in a new view.

**Object types** (entries Grid4D has objects for):

| Entry | Objects |
|-------|---------|
| Polygon | polygon objects |
| Generator | parametric objects: the primitives (Cube, Sphere, …) and the spline generators (Extrude, Lathe, Sweep, Loft) |
| Subdivision Surface | Subdivision Surface generators (their smoothed result) |
| Camera | cameras (their marker and pyramid) |
| Light | lights (their marker, cone or arrow) — they still light the view |

A hidden type is **neither drawn nor picked** in that view: clicks, rectangle
selection and snapping pass through it. Only objects of that type are hidden,
not their children. A selected hidden object (picked in the Object Manager)
still shows its gizmo and can be moved, scaled and rotated; in a component mode
its points, edges and polygons still show and can be edited.

**Elements**:

| Entry | What |
|-------|------|
| World Grid | the grid (in the view's plane) |
| World Axis | the axis triad in the lower left corner |
| HUD | the text in the lower right corner (grid spacing) |

- Choosing an entry toggles it. **Ctrl + choosing** shows only that entry and
  hides every other one.
- **All** shows everything again, **None** hides everything.
- The grid, axis triad and HUD toggles move from the Options menu to Filter.

Deliberate differences: the types and elements Grid4D has no objects or
drawing for yet (Null axes, Spline, Deformer, Joint, Particle, Field, Hair,
Environment, Other, Workplane, Axis, Handles, highlighting, SDS cage, N-gon
lines, guides, animation paths, ghosting, horizon), Default (the same as All
here), Geometry Only and filter presets are later; the filter is not saved with
the document.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `ViewFilter` (`FilterItem`s shown / hidden), `FilterItem::of(ObjectKindTag)` | `grid4d-render` |
| `EvaluatedNode::kind` | `grid4d-eval` |
| Drawing (`FrameData::filter`), picking (`pick`), snapping (`SnapQuery::shown`) | `grid4d-render` |
| Filter menu, rectangle selection, overlays (pyramids, cones, brackets, triad, HUD) | `viewport/mod.rs`, `select_tools.rs`, `overlay.rs` |

## Acceptance criteria

- [x] Each view hides and shows its own types and elements; All, None and
      Ctrl + choosing work.
- [x] Hidden types are neither drawn nor picked, selected by a rectangle or
      snapped to in that view; lights still light it.
- [x] Filtering keeps steady-state drawing allocation-free and leaves the
      document untouched.
