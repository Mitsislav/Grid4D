# FFD deformer

> Sources (read 2026-10-29): manual — the FFD deformer's description and Object
> properties (grid points, editing them in Points mode, Reset). Free-form
> deformation: T. Sederberg and S. Parry, *Free-Form Deformation of Solid
> Geometric Models* (SIGGRAPH 1986 — public), Bernstein polynomials. Written in
> our own words — no copied text or images.
>
> Covers implementation-plan step **10.4**; the framework (box, Mode, who is
> deformed, Fit to Parent) is in [`deformer-basics.md`](deformer-basics.md).

## Purpose

Sculpts an object by moving the points of a lattice around it, like pushing a
box of soft clay: pinch a waist, lift a corner, swell a face — with a few
points instead of many.

## Parameters

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Size, Mode, Fit to Parent | as every deformer | 250 cm × 3, Limited | Changing Size (or Fit) scales the lattice with the box |
| Grid Points X / Y / Z | 2 – 20 each | 3 × 3 × 3 | How many lattice points along each axis; changing them resets the lattice |
| Reset Lattice | button | — | Puts every point back on its regular grid (one undo step) |

## Behaviour

- **The lattice** is a grid of points spread evenly over the box. Unmoved, it
  changes nothing.
- **Editing:** select the FFD and switch to **Points** mode: its lattice points
  are shown and are selected and moved exactly like a spline's points (click,
  Shift / Ctrl, rectangle and Live Selection, Select All / Invert, the Move,
  Scale and Rotate gizmo, Tweak). Each change is one undo step; the selection is
  saved. Lattice points cannot be deleted.
- **Deformation:** a point inside the box moves by the lattice points' offsets
  from their grid places, each weighted by how close it is (smooth Bernstein
  blending along X, Y and Z): moving one lattice point pulls the region around
  it, most strongly at that point.
- **Modes:** Limited — points outside the box move with the nearest part of the
  box's surface; Within Box — only points inside move; Unlimited — the blending
  continues outside the box.
- **Display:** the lattice is drawn as lines joining neighbouring points (the
  box's own outline included), in the deformer's colour.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `Ffd` (size, grid, mode), `Ffd::grid_points`, `Ffd::apply(lattice, point)`, `lattice_lines` | `grid4d-geom::deform` |
| `ObjectKind::Ffd { ffd, lattice: SplineHandle, components }` — the lattice kept as a linear spline's points so Points mode, the gizmo and selections work as for splines; `ObjectKind::{points, points_mut}` for both | `scene/node.rs` |
| `Deformer::Ffd` carries the lattice; the lattice registered with evaluation's editable points | `grid4d-eval` |
| Grid / Size changes resetting or scaling the lattice, Reset Lattice, Delete skipping lattice points | `grid4d-ui` |
| `.g4d` v3 `ffd` kind: grid, size, mode, the lattice points (omitted while regular), selected / hidden ranges | `grid4d-io::v3` |

## Decisions (2026-10-29)

1. **The lattice is stored as a spline's points** (linear, no tangents): one
   point-editing path for splines and lattices.
2. **Size scales the lattice**; **grid counts reset it** (its points no longer
   correspond).
3. **Not animatable** in 10.4: the lattice points are edited, not keyed (point
   animation is later).

## Acceptance criteria

- [x] An unmoved lattice changes nothing; moving one corner point moves that
      corner of a cube the same amount and the opposite corner not at all; the
      middle of a 3 × 3 × 3 lattice pulls the centre most.
- [x] Modes as for the other deformers.
- [x] Points mode on an FFD selects and moves lattice points with the gizmo in
      one undo step; Delete leaves them; Reset Lattice puts them back.
- [x] Changing the grid resets the lattice; changing Size scales it.
- [x] Saved and loaded with moved points and their selection; rendered.
