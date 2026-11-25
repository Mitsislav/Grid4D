# Soft selection

> Sources: the reference's soft-selection page was not read for this spec;
> the idea (components near a selection follow it less the farther they are,
> by a radius and a falloff, measured through space or along the surface) is
> common to every modelling program and is described here as Grid4D's own.
> Surface distances by Dijkstra's shortest paths along edges (public). Written
> 2026-11-09 in our own words.
>
> Covers implementation-plan step **17.6**. Decision D41. Name (R8): *soft
> selection* is the plain, general term.

## Purpose

Moves, rotates and scales a selection with a smooth fall-off into the points
around it, so dents, bulges and gentle reshaping need no exact selection.

## Parameters (tool options, editor state)

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Soft Selection | bool | off | Turns it on for every component transform and Tweak. |
| Radius | length > 0 | 50 cm | How far from the selection points still follow. |
| Falloff | Linear · Smooth · Dome · Sharp | Smooth | The weight's curve from 1 (at the selection) to 0 (at the radius): straight, eased at both ends, round near the selection, or quickly dropping. |
| Distance | Volume · Surface | Volume | Volume: straight-line distance to the nearest selected point. Surface: distance along the mesh's edges, so the other side of a thin part does not follow. |

## Behaviour

- Every point of the active polygon object (or editable spline, or FFD
  lattice) gets a **weight** 1 when selected (or when it belongs to a
  selected edge or polygon), else from the falloff of its distance to the
  nearest selected point, 0 beyond the radius. Hidden points get 0.
- **The gizmo's moves, rotations and scales** and **Tweak** multiply each
  point's change by its weight (a rotation turns by angle × weight, a scale
  scales by 1 + (s − 1) × weight). One undo step per drag, as before.
- **Shown** while on, in component modes: points within the radius tinted
  by weight (a ramp from the selection colour to the mesh colour), so the
  reach is visible before dragging; the ramp is drawn with the component
  overlay and rebuilt only when it changes.
- With nothing selected, Tweak's point under the cursor is the selection.
- Surface distance with no path (a separate piece) is beyond the radius.
- The settings are **never saved in the document**; they are preferences of
  the tool (kept with the other tool options for the session).
- Shortcut: none until one is verified against the reference's list.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `SoftSelection { enabled, radius, falloff, distance }` in `EditorState` (the Attribute Manager's Tool mode shows it with the Move tool's options) | `ui/src/editor.rs` |
| `soft_weights(mesh, topology, selected, options, &mut Vec<f32>)` — volume by a grid of cells, surface by Dijkstra with a reused heap | `grid4d-geom::selection` (pure) |
| Weights cached with the overlay (mesh id, selection, options) in reused buffers; the gizmo's `PointEdit` and Tweak apply them | `ui/panels/viewport/gizmo.rs`, `grid4d-render::components` (the ramp) |

## Decisions made while implementing (2026-11-25)

1. The weights live in their own module, `grid4d_geom::soft`
   (`SoftSelection`, `Falloff`, `SoftDistance`, `soft_weights`,
   `SoftScratch`), not in `selection`.
2. Distances are measured in **world space** (the object's points placed by
   its matrix), so the radius means the same on a scaled object.
3. The settings sit with the transform tools' (`TransformSettings::soft`)
   and show on the Move, Scale and Rotate tool pages; the ramp shows in
   every component mode while it is on.
4. The ramp is the overlay's point colour: a weight (0 – 255) in the point's
   flags, mixed in the shader from the wire colour to the selection colour;
   in Edges and Polygons modes only the points it reaches are drawn. The
   viewport keeps the weights and computes them again only when the mesh,
   its selection, its placement, the mode or the settings change.
5. Falloffs: Linear 1 − t, Smooth 1 − (3t² − 2t³), Dome √(1 − t²), Sharp
   (1 − t)².
6. An editable spline's or FFD lattice's points use Volume distance (they
   have no edges to walk).

## Acceptance criteria (all met in 17.6)

- [x] On a plane, a moved centre point drags its neighbours by the falloff's
      weights; points beyond the radius stay.
- [x] Surface distance: on a folded strip the far layer does not follow;
      with Volume it does.
- [x] Rotation and scale apply by weight; weight 1 points move exactly as
      without soft selection.
- [x] Each falloff gives 1 at distance 0 and 0 at the radius, monotonic.
- [x] The steady frame allocates nothing (the allocation test with soft
      selection on and the overlay shown).
- [x] One drag is one undo step; undo restores every moved point.
