# The Picture Viewer

> Sources (read 2026-10-28): Cinema 4D manual —
> [Picture Viewer](https://help.maxon.net/c4d/r21/us/html/5878.html) (where
> renders appear with Render ▸ Render to Picture Viewer; the History keeps the
> pictures rendered in the session; render time per picture in the
> information area; Auto Zoom fits the picture to the window) and the Render
> menu (Render to Picture Viewer, Shift+R). Written in our own words.
>
> Covers implementation-plan step **8.3**. The settings it renders with are
> [`render-settings.md`](render-settings.md) (8.2); the Render View (rendering
> into the viewport) is 8.4, frame ranges 8.5.

## Purpose

Rendering the final picture without blocking the work: the picture appears and
sharpens while it renders, can be stopped, compared with earlier renders and
saved.

## Parameters

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Zoom | Fit, or 6 %…1600 % | Fit | Fit: the whole picture in the window; 100 %: one picture pixel per screen pixel |
| History | the session's pictures | — | click one to show it again; kept until the application closes or they exceed 512 MB together (our choice), oldest first |

## Behaviour

- **Starting**: Render ▸ Render to Picture Viewer, **Shift+R** or the toolbar
  button. The window opens (or comes back) and the render starts at once with
  the document's Render Settings: its size, samples, bounces, Auto Light.
- **What is rendered**: the scene at the current frame, through the active
  view's camera — the editor camera, or the camera object the view looks
  through — in the render's proportions (Render Settings ▸ Output; our choice:
  the camera's vertical field of view is kept).
- **Progressive**: the first picture appears after one sample per pixel and
  sharpens as more arrive; a bar shows the progress, the information line the
  samples and the time.
- **In the background**: the application stays usable; editing the scene does
  not change a render that runs (it renders the scene as it was when it
  started).
- **Stop**: the Stop button stops the render; the picture keeps what had been
  rendered (marked as stopped). Starting a new render while one runs stops the
  old one first.
- **When it finishes**: with Render Settings ▸ Save on and a File, the picture
  is saved there (PNG, with transparency if Alpha Channel is on), its folder made
  if needed; the information line says where, or why not (no file; a relative
  file and a document never saved; a write error, also shown as an alert).
  Stopped renders are not saved.
- **Save As…** saves the shown picture anywhere (PNG), with the alpha setting
  it was rendered with.
- **History**: every render (finished or stopped) is listed, newest first,
  with its number, size, samples and render time; the shown one is highlighted.
- **Zoom**: Fit, 100 %, zoom in and out buttons; the mouse wheel zooms about the
  pointer; dragging pans. Pictures with transparency show over a checkerboard.
- **Closing the window** does not stop a render; reopening shows it.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| Progressive, cancellable rendering (`Progressive`: samples added in passes, identical to `render`) | `grid4d-raytrace` |
| PNG writing (`save_png`, atomic) | `grid4d-io` (`png.rs`) |
| Render jobs on a worker thread, the history, saving on completion | `grid4d-ui` (`rendering.rs`, files through `file.rs`) |
| The window: `PictureViewerPanel` | `grid4d-ui` (`panels/picture_viewer.rs`) |
| `EditorCommand::{RenderToPictureViewer, StopRender, SaveRenderAs}`, Shift+R | `grid4d-ui` |

Deliberate differences: one render at a time (no queue); no filters, channels,
layers or A/B comparison; no RAM player or animations (8.5); the history lives
in memory only.

## Acceptance criteria

- [x] Shift+R, the menu and the toolbar start a render and open the window.
- [x] Progressive results equal a full render once complete (same picture).
- [x] Stop ends the render quickly and keeps the partial picture.
- [x] A finished render is saved where Render Settings say, with or without
      alpha; relative files need a saved document.
- [x] The history keeps and shows earlier pictures; old ones go past the limit.
- [x] The UI never waits for the renderer.
