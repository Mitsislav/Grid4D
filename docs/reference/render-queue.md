# The Render Queue, the render region and the interactive Render View

> Sources (read 2026-11-07): Cinema 4D manual —
> [Render Queue](https://help.maxon.net/c4d/en-us/Content/html/43084.html)
> (saved scene files rendered unattended one after another; jobs added from
> files, the current scene or its takes; per job a take, a camera, render
> settings and an output path; each job enabled or not, with a status —
> ready, in progress, finished, stopped, error — and a message; Start
> renders the enabled jobs from the top; the list is kept after the program
> closes; unsaved changes are not rendered) and
> [Interactive Render Region](https://help.maxon.net/c4d/en-us/Content/html/OSNIPER.html)
> (a frame in the view, moved and sized with handles, rendered continuously
> and again after every change that affects it, at an adjustable detail;
> kept with the scene). Written in our own words.
>
> Covers implementation-plan steps **16.8** (queue, region) and **16.9**
> (interactive Render View). Builds on
> [`render-picture-viewer.md`](render-picture-viewer.md),
> [`render-view.md`](render-view.md), [`render-animation.md`](render-animation.md)
> and [`manager-take.md`](manager-take.md).

## Purpose

Rendering several shots or takes overnight without sitting next to the
machine; rendering only the part of the picture being worked on; and seeing
the final look update while editing, instead of starting a render after each
change.

## The Render Queue (16.8)

### Parameters (per job)

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Enabled | on / off | on | Start renders it |
| Document | a `.g4d` file | — | the saved file rendered (read from disk when the job starts) |
| Take | the file's takes | the file's render take (Render Settings ▸ Take), else Main | renders that take, as Render ▸ Render Take does |
| Frames | From Document / Manual (From, To, Step) | From Document | the file's Render Settings' range, or this one |
| Output | From Document / a file | From Document | the file's Render Settings' output (its folder), or another path; numbering and format as the document says |
| Status | Ready, Rendering, Finished, Stopped, Error | Ready | with a message (frames done, the error, missing pictures) |

### Behaviour

- **Opening**: Render ▸ Render Queue…, a window beside the workspace (like
  the Picture Viewer).
- **Adding**: *Add Current Document* (it must be saved; with unsaved changes
  the job renders the file on disk and says so), *Add Files…* (several
  `.g4d`), *Add All Takes* (one job per take of the current document), or
  `.g4d` files dropped on the window. Jobs can be reordered by dragging,
  duplicated and removed (Delete).
- **Start** renders the enabled jobs from the top, **one at a time in the
  background** (the editor stays usable; the Picture Viewer's renders wait
  their turn behind a running job, and a job waits for a Picture Viewer
  render). Each job loads its file on the worker, applies its take to its
  copy, decodes its textures, simulates what its frames need, and renders
  and saves each frame as Shift+R does.
- **Stop** stops after the current pass; the job is Stopped, its finished
  frames saved. **Start** again continues a stopped job **from the first
  frame whose file does not exist yet**; *Reset* sets a job back to Ready.
- **Errors** (a file that cannot be read, no output path, a write error)
  mark that job Error with the message and the queue goes on with the next.
  Missing pictures render with their channels' values and are listed in the
  message.
- **Progress**: each job's frames done / total and the current frame's bar;
  the window title shows the queue's overall progress. The last finished
  picture of the running job can be shown in the Picture Viewer (*Show*).
- **Kept by the app**, never in documents: the job list (documents by path,
  settings, status) is saved in the app's storage on exit and comes back on
  the next start; a job Rendering when the app closed comes back Stopped.
- The queue is not undoable (it is not document data); removing a job asks
  nothing.

## The render region (16.8)

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Region | a rectangle in picture fractions, or none | none | renders only the pixels inside |
| Use for Final Render | on / off | off | Shift+R and the queue render only the region too (the rest of the picture transparent) |

- **Drawing**: Render ▸ Render Region, then drag a rectangle in the active
  view; its edges and corners can then be dragged (within 6 points; the
  cursor shows which); Esc during a drag puts it back; *Clear Render
  Region* removes it. Shown as a dashed frame in the active view. The
  region is kept in ten-thousandths of the view's width and height; a final
  render takes the same fractions of its picture (our choice: the view
  shows no picture area of its own yet, so the two match when the
  proportions do).
- Saved with the document (`document.render`), undoable (one step per
  drag), part of takes like other render settings.
- The Render View renders only the region (the rest of the view stays the
  editor's drawing); the final render honours it only with Use for Final
  Render.
- **Shortcut**: none at first. The plan names Alt+R; it is bound only once
  verified against the reference's shortcut list (the region page names no
  key), and checked against Grid4D's own Alt bindings.

## The interactive Render View (16.9)

- **Interactive** (Render ▸ Interactive Render View, on by default): once
  started with Ctrl+R, the Render View **keeps rendering** and follows the
  work instead of going away:
  - a **camera change** (orbiting, a linked camera moving) restarts the
    samples with the scene already built for rendering (no rebuild);
  - a **scene change** (an edit, undo, a frame change, Render Settings)
    rebuilds the render scene **on the worker**; a newer change cancels an
    older rebuild;
  - the **old picture stays** until the new one's first pass arrives — the
    view never goes blank; during drags the first pass renders every fourth
    pixel in each direction (blocky, filled in by the next passes) to keep
    up;
  - selection changes, panel switches and idle frames change nothing.
- It stops with Esc, Ctrl+R again, another view becoming active (it moves
  with Ctrl+R), or a new document. Interactive off: the 1.1 behaviour (the
  picture goes away on any change).
- Adaptive sampling and the denoiser (Final and Render View) apply as their
  settings say; the view's progress line shows samples and time since the
  last restart.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `RenderQueue`, `Job`, `JobStatus`; one worker at a time, shared with Shift+R renders | `grid4d-ui` (a new `render_queue.rs`, `rendering.rs`) |
| The window | `grid4d-ui` (`panels/render_queue.rs`) |
| Saving the job list in the app's storage | the binary (`src/app.rs`), like recent files |
| `RenderSettings::{region, region_final}` (`RenderRegion`) | `grid4d-scene` (`render.rs`) |
| Rendering a pixel rectangle | `grid4d-raytrace` (`RenderOptions::region`; `Progressive` samples only inside; inside pixels are exactly those of a whole render) |
| Region drawing and handles | `ui/panels/viewport/render_region.rs` (`EditorState::drawing_region`; `EditorCommand::{DrawRenderRegion, ClearRenderRegion}`) |
| Interactive restarts, the kept picture, the scene rebuilt on the worker, camera-only restarts | `grid4d-ui` (`rendering.rs`: the view job keeps its `RenderScene`), `ViewKey` split into camera and scene parts |

Deliberate differences: no Team Render or network jobs, no per-job camera or
render-settings override other than take, frames and output, no presets of
whole queues, no log files (messages in the list); one region for the
document rather than one per view; the region's detail slider is replaced by
the blocky first pass.

## Acceptance criteria

- [ ] Two jobs (two takes) run one after another unattended and save their
      frames; a stopped job continues from its first missing frame.
- [ ] A broken file is an Error and the queue goes on.
- [ ] The job list survives a restart of the app (statuses: Rendering comes
      back Stopped); documents never contain it.
- [ ] The queue and Shift+R never render at once.
- [x] The region renders only its pixels (Render View; final render with the
      switch); it is undoable and saved in v5.
- [ ] The interactive Render View restarts on camera moves without
      rebuilding the scene, rebuilds on edits on the worker, never shows a
      blank view, and ignores selection changes (interface tests).
- [ ] The interface never waits for a render or a rebuild.
