# The UV editor

> Sources: the established workflow of UV editors in 3D suites (the
> reference's UV tools included, by their general behaviour only: a 2D view
> of the UV square over the texture, the three component modes, unwrapping
> from seams, relaxing and packing). LSCM is a published algorithm (Lévy et
> al., 2002). The reference's UV manual pages are to be read in detail before
> step 15.7 and this spec corrected where needed. Written in our own words,
> 2026-11-06.
>
> Covers implementation-plan step **15.7**.

## Purpose

Seeing and arranging how a polygon object's surface lies on its pictures:
cutting it into flat pieces (islands) along seams, flattening them with
little stretching, and laying them out in the UV square.

## The pane

- A **pane** of the workspace (Window ▸ UV Editor; a tab beside the
  viewport in the Modeling layout), showing the **active polygon object**'s
  UVs as a 2D mesh over the 0 – 1 square, with the material's Base Color
  texture behind it (a menu chooses another channel or none) and the tiling
  grid around it.
- Navigation: middle-drag or Alt+drag pans, the wheel zooms about the
  cursor, **H** frames all, **O** frames the selection.
- **Points / Edges / Polygons** modes and the selection are **the same as
  the viewport's** (one component selection per object): selecting in either
  shows in both. Click, Shift (add), Ctrl (remove), rectangle drag; Select
  All / Deselect All / Invert / Grow / Shrink / Connected work here too, and
  **Select Island** (double-click) selects all connected UV polygons.
- UV corners are per corner: a point shared by several faces may sit in
  different places in UV space (a seam). Points mode moves the UV corners at
  one place together.

## Tools

| Tool | What it does |
|------|--------------|
| Move, Scale, Rotate | the selection in UV space (a 2D gizmo; values in the Attribute Manager); one undo step per drag |
| Flip U / Flip V | mirrors the selection about its centre |
| Unwrap | flattens the selected polygons (all, if none) with **LSCM**, cutting along **seams**: the selected edges, else edges sharper than an angle (60°), else the mesh's open borders; each island scaled to its 3D area; then packed |
| Project | from a projection (Box, Spherical, Cylindrical, Flat, Camera; as in [`tag-material.md`](tag-material.md)) or from the view |
| Relax | moves inner UV points to even out stretching, borders pinned (an option frees them); iterations 1 – 100 (20) |
| Pack Islands | scales and places every island (or the selected ones) into the square without overlap; margin in pixels of a chosen texture size (4 px of 1024); rotation in 90° steps allowed (on) |
| Mark Seams / Clear Seams | keeps the selected edges as seams for later Unwraps (stored with the object) |
| Stretch display | colours polygons from blue (compressed) to red (stretched) |

## Behaviour

- Every tool is one undo step and changes the object's mesh like any mesh
  edit (a new mesh handle; selections kept).
- A polygon object without UVs gets them on the first Unwrap or Project.
- Objects that are not editable (primitives, generators) show their UVs
  read only, with a note to make them editable.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| Algorithms: LSCM (sparse least squares, conjugate gradients), islands, seams, relax, pack, projections | `grid4d-geom` `uv.rs`, pure functions with tests |
| Mesh edits | `ui/src/modeling.rs`, `EditorCommand::UvCommand(UvOp)` through `commit` |
| Pane | `panels/uv_editor.rs`, a `Pane`; drawn with egui shapes (no GPU view needed) |
| Seams | stored with the polygon object's components (file: v4) |

## Acceptance criteria

- [ ] Unwrap of a cube cut along seven edges gives one island with no
      flipped faces; a cylinder cut along one edge flattens with area
      distortion under 5 %.
- [ ] Pack Islands never overlaps islands; everything lies in 0 – 1.
- [ ] Relax lowers the stretch measure and never flips a face.
- [ ] Selecting in the UV editor selects the same components in the
      viewport, and back.
- [ ] Every tool is one undo step; save and load keep UVs and seams.
