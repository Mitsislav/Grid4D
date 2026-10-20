# Viewport layouts: several views, orthographic projections

> Sources (read 2026-10-21): Cinema 4D manual —
> [Panel menu](https://help.maxon.net/c4d/r21/us/html/5861.html) (a view panel holds
> up to four views, each with its own camera, projection and display mode; layouts
> from Single View to 4 Views and stacked / side-by-side variants; *Toggle Active
> View* makes the active view fill the panel and back — a middle click does the
> same; View 1–4 (F1–F4) and All Views (F5)),
> [Cameras menu](https://help.maxon.net/c4d/r21/us/html/5858.html) (projections:
> Perspective, Parallel, Left, Right, Front, Back, Top, Bottom, …) and the
> [keyboard shortcuts](https://help.maxon.net/c4d/en-us/Content/html/Shortcuts.html)
> (viewport: "Changing the views F1–F5", "All views F5", "Viewport large/small: MMB
> on view"). Written in our own words.
>
> Covers implementation-plan step **7.2**. Display modes per view are 7.3.

## Purpose

Seeing a model from several sides at once while modelling: a perspective view
next to exact top, front and side views without perspective distortion.

## Behaviour

- **Four views**, each with its own camera and settings: View 1 **Perspective**
  (top left), View 2 **Top** (top right), View 3 **Right** (bottom left), View 4
  **Front** (bottom right), the reference's default arrangement.
- **Panel menu** (each view's header): *Single View*, *2 Views Stacked*,
  *2 Views Side-by-Side*, *4 Views*; *View 1* … *View 4* (F1–F4) show one view
  alone, *All Views* (F5) shows the four. The panel starts as a single
  perspective view, as before.
- **Middle click** on a view (without dragging) toggles it between filling the
  panel and its place in the arrangement.
- **The active view**: the one last clicked or scrolled in; it has a thin
  highlighted frame when several views show. Tools, picking and the gizmo work in
  it; a press in another view makes that one active first, so every view can be
  used directly. H, O and Reset View act on the active view.
- **Projection** (each view's Cameras menu): Perspective, Top, Bottom, Front, Back,
  Left, Right. The orthographic ones look straight along a world axis without
  perspective:
  - Front looks along −Z (X right, Y up), Back along +Z; Right along −X (−Z right),
    Left along +X; Top down −Y (X right, −Z up), Bottom up +Y.
  - They cannot orbit: Alt + left drag pans there; panning and zooming work as
    usual (zoom changes the visible area).
  - The grid lies in the view's plane (XY in Front/Back, ZY in Left/Right, XZ in
    Top/Bottom and the perspective view), with the axis lines in their colours.
- Each view has its own Options (grid, axis triad, HUD) and title (its projection,
  or the name of the camera it looks through).
- **Looking through a camera** (7.1) is offered in View 1 only.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| Orthographic camera (`Projection::Parallel`: zoom = the visible height; rays parallel) | `grid4d-render::OrbitCamera` |
| Screen picking and snapping along parallel rays | `ScreenView::parallel` |
| Grid plane and parallel view direction for the shaders | `FrameData` (`grid_plane`, `parallel`) |
| Views, arrangement, active view, maximised view | `ViewportPanel` (`ViewKind`, `Arrangement`) |
| One renderer, one offscreen target per view | `viewport/gpu_view.rs` |
| F1–F5 | `EditorCommand::ShowView` |

Deliberate differences: the reference's 3-view and other 4-view arrangements, the
Parallel and axonometric projections, and saving the layout with the document are
later; camera links only in View 1.

## Acceptance criteria

- [x] The orthographic views show the scene exactly along their axes (a cube is a
      square); picking, the gizmo, component selection and snapping work in them.
- [x] Panel arrangements, F1–F5 and the middle click switch views; each view keeps
      its own camera and settings.
- [x] A press in an inactive view activates it and the same press already acts.
- [x] The grid lies in each view's plane (grid snapping uses that plane too).
- [x] Idle frames leave the document untouched with four views.
