# Snapping

> Sources (read 2026-10-20, Cinema 4D 2027.0 and R23 pages of the same chapter):
> manual — [Modeling Settings ▸ Snap](https://help.maxon.net/c4d/r23/en-us/Content/html/DMODELING-SNAP_SETTINGS.html)
> ([2027 page](https://help.maxon.net/c4d/2027/en-us/Content/html/DMODELING-SNAP_SETTINGS.html)),
> [Snapping overview](https://help.maxon.net/c4d/r23/en-us/Content/html/50805.html),
> [Keyboard Shortcuts](https://help.maxon.net/c4d/2025/en-us/Content/html/Shortcuts.html)
> (Modeling: *Snapping on/off* = Shift+S, *Quantize on/off* = Shift+Q);
> Python SDK [`c4d.modules.snap`](https://developers.maxon.net/docs/py/2027_0_0/modules/c4d.modules/snap/index.html)
> (the `SNAPMODE_*` targets and the `SNAP_SETTINGS_*` keys: enabled, radius, mode
> Auto/2D/3D).
> Written in our own words — no copied text or images.
>
> Covers implementation-plan step **4.8**. Depends on
> [`modeling-modes.md`](modeling-modes.md) §6 (component transforms, Tweak).

## Purpose

While moving an object or components, the moved element jumps onto nearby
geometry or grid positions, so things can be placed exactly — a point onto another
point, an object onto a grid point, a corner onto an edge.

## Parameters

Global settings (the reference also offers per-tool settings; see *Later*).

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Enable Snapping | bool, Shift+S | off | Snapping on or off (the palette button shows the state) |
| Type | Auto · 2D · 3D | Auto | **3D**: the element goes to the target's position in space, depth included. **2D**: it lines up with the target *on screen* and keeps its own depth. **Auto**: 3D in perspective views, 2D in orthographic ones (Grid4D has only the perspective view yet, so Auto acts as 3D) |
| Snap Radius | 0.1 … 100 px | 10 px | How close (on screen, in UI points) the cursor must be to a target. The reference does not state its default; 10 is our choice |
| Vertex | bool | on | Points of meshes (back sides too) |
| Edge | bool | on | The nearest point along an edge (back sides too) |
| Edge Mid Point | bool | on | The middle of an edge |
| Polygon Center | bool | on | The centre of a polygon |
| Polygon | bool | off | The surface under the element (best with 3D) |
| Axis | bool | on | Objects' origins |
| Grid Point | bool | on | Crossings of the floor grid at its current spacing (the "Grid Spacing" shown in the view) |
| Grid Line | bool | off | The nearest point on a floor grid line |

The reference does not document which targets are on by default; the defaults above
are ours.

## Behaviour

- **What snaps.** Dragging a Move handle moves the gizmo's centre: the object's axis
  in Model mode, the centre of the selection in a component mode. That centre snaps.
  With *Tweak*, the grabbed point, edge or polygon (its centre) snaps. The reference
  also lets the user drag an arbitrary element of the selection and snap it; our
  Move tool only drags handles, so the centre is the element.
- **Finding the target.** Targets are looked for around the **cursor** — the
  reference describes the snap radius as the cursor's detection area — so the user
  points at what to snap to. (A first version searched around the position the
  element would have had without snapping; on screen the highlight then appeared
  between the gizmo and the cursor, which was confusing.) Within the radius,
  point-like targets (vertex, edge mid point, polygon centre, axis, grid point) win
  over line-like ones (edge, grid line), which win over the surface (polygon);
  within a class the one nearest on screen wins. (The reference only says the
  nearest target is taken; the classes are our rule, so that a vertex can be caught
  next to the edges it ends.)
- **Constrained moves.** A handle drag moves along one axis. The snapped position is
  the point on that axis line closest to the target (3D), or closest to the view ray
  through the target (2D), so the element lines up with the target along the axis.
- **Free moves** (Tweak): 3D puts the element exactly on the target; 2D puts it on
  the view ray through the target, at its own depth.
- **What is not a target.** Whatever moves with the drag: the moved object and its
  children; in a component mode the moved points and the edges and polygons that use
  them (the object's other points stay targets, so a point can be put on top of
  another one); a generator whose cage is being changed (its result moves too).
  Cameras and lights (markers) are not targets.
- **Feedback.** While a drag is snapped, the target is highlighted in the view — a
  marker on a point, the edge or grid line drawn over — with its name next to it
  (*Vertex*, *Edge*, *Edge Mid Point*, *Polygon Center*, *Polygon*, *Axis*,
  *Grid Point*, *Grid Line*).
- **Other tools.** Scale and Rotate do not snap; stepping them is *Quantize* (later).
- **Undo.** Unchanged: a snapped drag is still one undo step.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `SnapSettings` (enabled, type, radius, targets) | `ui/src/editor.rs` (editor state, not saved in the document) |
| Target search: `find_snap(&EvaluatedScene, &ScreenView, &mut SnapCache, &SnapQuery) -> Option<SnapHit>` | `grid4d-render::snap` — screen-space like component picking; primitives' unit meshes cached per `MeshKey` |
| Grid targets | the floor (y = 0) at `OrbitCamera::grid_spacing()` |
| Applying it | `viewport/gizmo.rs` (handle drags keep the unsnapped total and snap it each frame), `viewport/select_tools.rs` (Tweak) |
| Highlight and label | viewport overlay (egui painter) |
| Shift+S, palette button (click toggles, right-click shows the settings), Tools ▸ Snap submenu, Attribute Manager Move tool ▸ Snap | `shortcuts.rs`, `panels/mode_palette.rs`, `panels/menu_bar.rs`, `panels/attributes.rs` |

## Later

Per-tool settings, Quantize (Shift+Q: steps for move, scale, rotate), workplane,
splines, guides and dynamic guides, intersections, perpendicular, the crosshair,
snapping for other tools (Line Cut points, Bridge).

## Acceptance criteria

- [x] With snapping off, drags behave exactly as before.
- [x] A cube moved along X with the cursor near another object's axis lines up
      with it along X (3D: the axis's X equals the target's X).
- [x] Tweaking a point onto another point of the same mesh (3D) puts it exactly
      there; 2D keeps its depth and lines it up on screen.
- [x] Vertices beat the edges that end in them; an edge is caught away from its ends.
- [x] Edge mid points, polygon centres, axes, grid points and grid lines are found
      at their exact positions; disabled targets are never used.
- [x] Nothing that moves with the drag is a target (the moved object's subtree, the
      moved points and their edges/polygons, the generator of a moved cage).
- [x] Targets farther than the radius are ignored; the radius is in screen units.
- [x] Shift+S toggles snapping; the palette button, the Tools menu and the Attribute
      Manager show and change the same settings.
- [x] A snapped drag is one undo step; undo restores the exact previous state.
