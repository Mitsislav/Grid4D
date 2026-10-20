# Viewport display modes: shading and lines

> Sources (read 2026-10-21): Cinema 4D manual —
> [Display menu](https://help.maxon.net/c4d/r21/us/html/5860.html) (shading modes:
> Gouraud Shading, Quick Shading, Constant Shading, each also "(Lines)", Hidden
> Line, Lines; line modes Wireframe and Isoparms, which only show in a mode that
> draws lines; Box draws each object as a box of its dimensions; Skeleton connects
> the objects' origins by hierarchy; the settings can differ per view) and the
> [keyboard shortcuts](https://help.maxon.net/c4d/2026/en-us/Content/html/Shortcuts.html)
> (viewport: N~A Gouraud Shading, N~B Gouraud Shading (Lines), N~C Quick Shading,
> N~D Quick Shading (Lines), N~E Constant Shading, N~F Hidden Line, N~G Lines,
> N~H Wireframe, N~I Isoparms, N~K Box, N~L Skeleton). Written in our own words.
>
> Covers implementation-plan step **7.3**. Scene lights in the viewport are 7.4.

## Purpose

Choosing per view how objects are drawn: smooth shading to judge the form,
lines to see the mesh's structure, boxes to keep heavy scenes quick.

## Behaviour

Each view has its own display mode, picked in its **Display** menu (or with the
N~ shortcuts, which act on the active view). A mode has two parts:

**Shading** (one of):

| Mode | Surfaces | Lines |
|------|----------|-------|
| Gouraud Shading (N~A) | smooth, lit (by the scene's lights once 7.4 is done; until then by the default light) | — |
| Gouraud Shading (Lines) (N~B) | as above | on top |
| Quick Shading (N~C) | lit by the default light only — always the light from over the viewer's shoulder, never the scene's lights | — |
| Quick Shading (Lines) (N~D) | as above | on top |
| Constant Shading (N~E) | flat, in the object's colour, no lighting | — |
| Constant Shading (Lines) | as above | on top |
| Hidden Line (N~F) | not drawn, but they hide what is behind them | the visible ones |
| Lines (N~G) | not drawn | all, hidden ones included |

**Line style** (one of):

- **Wireframe** (N~H): the lines are the edges of each object's polygons
  (n-gons show their outline, not their triangles).
- **Box** (N~K): each object is drawn as its bounding box, in the object's own
  orientation — shaded like any surface in a shaded mode, its twelve edges in a
  line mode. (The reference does not say how Box mixes with shading; this is our
  choice.)
- *Isoparms* (N~I) and *Skeleton* (N~L) are listed but planned.

**Line colours**: over shaded surfaces the lines are a dark shade of the
object's colour; in Hidden Line and Lines they take the object's colour (the
reference: lines are coloured by the object's material). Lines of selected
objects are orange.

The component overlay of the component modes, the gizmo, the grid and the
camera pyramids are drawn as before in every mode. New views start in Gouraud
Shading with Wireframe lines.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `DisplayMode { shading: ShadingMode, style: DisplayStyle }` | `grid4d-render::display` |
| Per view, in `FrameData::display` | `ViewState`, `ViewportPanel` |
| Line geometry (edges per mesh key, uploaded once), box geometry per key | `ViewportRenderer` (`gpu.rs`) |
| Quick / Constant / depth-only fills, line pass | `shaders/mesh.wgsl` (`vs_line`, `fs_line`) |
| Display menu, N~ chords | `viewport/mod.rs`, `shortcuts.rs` (`EditorCommand::SetShading`, `SetDisplayStyle`) |

Deliberate differences: Isoparms, Skeleton, the separate display settings for
active objects, Backface Culling, X-Ray and textures in the viewport are later;
picking still uses the real geometry in Box mode; lines are one pixel wide.

## Acceptance criteria

- [x] Every shading mode draws as described; the "(Lines)" modes, Hidden Line and
      Lines draw the polygon edges (no triangulation lines on n-gons).
- [x] Box draws each object as its bounding box, shaded or as lines.
- [x] Each view keeps its own mode; the Display menu and N~A … N~H, N~K change the
      active view's.
- [x] Hidden Line hides lines behind surfaces; Lines shows them all.
- [x] Line geometry is built once per mesh key; drawing stays allocation-free in
      steady state.
