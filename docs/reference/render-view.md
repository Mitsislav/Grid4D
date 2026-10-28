# The Render View

> Sources (read 2026-10-28): Cinema 4D manual — the Render menu's *Render
> View* (Ctrl+R: renders the active view inside the viewport; the result is not
> saved and goes away as soon as the view or the scene changes) and the
> [Picture Viewer](https://help.maxon.net/c4d/r21/us/html/5878.html) page for
> what is kept instead. Written in our own words.
>
> Covers implementation-plan step **8.4**. The renderer is
> [`render-engine.md`](render-engine.md), its settings
> [`render-settings.md`](render-settings.md), the background rendering
> [`render-picture-viewer.md`](render-picture-viewer.md).

## Purpose

A quick look at the final picture right where you work, without opening a
window or writing a file: render the active view, look, carry on editing.

## Behaviour

- **Starting**: Render ▸ Render View, **Ctrl+R** or the toolbar button. The
  active view is rendered in place, at its own size in screen pixels, through
  its camera (the editor camera or the camera object it looks through), at the
  current frame, with the Render Settings' samples, bounces and Auto Light (not
  their size, file or alpha).
- **Progressive**: the picture covers the view after the first pass and
  sharpens; a line in the view's corner shows the progress, then the samples
  and the time. The editor stays usable.
- **Goes away** when what it shows is no longer true: the view's camera moves,
  the view changes size or another view becomes active, the scene changes
  (any edit, undo, a new or opened document), the Render Settings change, or
  the current frame changes. A render that runs is then stopped. **Esc** (with
  no tool busy) clears it too. Selecting objects does not.
- **Again**: Ctrl+R while a Render View shows starts it afresh.
- Overlays (the view's name, the gizmo, camera and light markers) stay on top,
  so the view can still be worked in; the background is black.
- Not saved, not in the Picture Viewer's history (our choice, like the
  reference: use Shift+R for that). Only one Render View at a time.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `ViewKey` (what the picture depends on), `Renders::{start_view, clear_view, view_render}` — a second job beside the Picture Viewer's | `grid4d-ui` (`rendering.rs`) |
| Drawing the picture over the active view, clearing it when the key changes, `ViewportPanel::view_key` | `grid4d-ui` (`panels/viewport/mod.rs`) |
| `EditorCommand::RenderView`, Ctrl+R, Esc, Render menu, toolbar | `grid4d-ui` |

Deliberate differences: no Render Region (Alt+R) and no interactive
re-rendering while editing (the picture goes away instead).

## Acceptance criteria

- [x] Ctrl+R, the menu and the toolbar render the active view at its size.
- [x] The picture sharpens in passes and equals a full render of the view.
- [x] Moving the camera, editing the scene, changing frame or Render Settings,
      resizing or switching views, and Esc remove it and stop its render.
- [x] Nothing is saved; the Picture Viewer's history is unchanged.
