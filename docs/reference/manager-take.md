# Takes and the Take Manager

> Sources (read 2026-10-30): manual — the Take Manager (the Main take holding
> the scene as it is, further takes holding changes to it, the active take,
> overrides recorded automatically while a take is active, a camera and render
> settings per take, rendering a chosen take). Written in our own words — no
> copied text or images.
>
> Covers implementation-plan step **12.5**.

## Purpose

Keep variations of one scene — another camera, a red material instead of a
blue one, an object hidden — side by side in one document, switch between
them, and render the one you need, without copies of the file.

## Behaviour

- **Main** is the scene as it is. Other **takes** hold changes to it:
  overrides of some objects' values, in groups — **Transform**,
  **Visibility** (editor, renderer, enabled), **Parameters** (the object's own
  settings; not an editable object's points), **Tags** (Material links among
  them), **Display Color** — and, optionally, their own **render settings**
  and a **camera**.
- **One take is active** at a time; the views, the managers and renders show
  it. Clicking a take in the Take Manager makes it active (one undo step);
  clicking Main goes back.
- **Auto Take**: while a take other than Main is active, every change to an
  overridable value (moving an object, recolouring it, hiding it, changing a
  parameter, linking another material, changing the render settings) is
  recorded as that take's override, in the same undo step. Main keeps its own
  values. Changes to other things — names, the hierarchy, new objects,
  animation keys, layers, materials themselves — belong to the whole document
  in every take.
- **The Take Manager** (the Takes pane):
  - **New Take** adds a take and makes it active;
  - the takes are listed under Main; the active one is marked;
  - double-click a take's name to rename it; right-click for **Delete Take**
    (deleting the active take goes back to Main first);
  - the active take's **Camera** (none: the view's) and its overrides, one
    line per object and group, each with a button that **clears** it (the
    object takes Main's value again).
- **Rendering a take.** Render Settings ▸ **Take**: *Current* (the active
  take) or a named one. A render of another take renders that take without
  making it active; its camera, if it has one, is the one renders look
  through.
- Takes are saved with the document, the active one included.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `Take` (name, overrides by object, camera, render settings), `TakeId`, `Takes` (order, active, Main's values beside it, the take to render), `Override`, `OverrideGroup`, `overridable` | `grid4d-scene::take` |
| `Document::takes`; `Document::switch_take` (a pure swap: Main's values back, the take's written, Main's kept); `Document::record_in_take` (Auto Take from an edit's inverses) | `grid4d-scene::document` |
| `SceneEdit::{SwitchTake, SetTakes}` | `grid4d-scene::edit` |
| `commands::commit` and live commits record in the active take | `grid4d-ui::commands` |
| `EditorCommand::{SwitchTake, NewTake, DeleteTake, ClearOverride}`; renders of another take switch a copy | `grid4d-ui::commands`, `rendering` |
| The Take Manager pane; Render Settings ▸ Take | `grid4d-ui::panels::{takes, render_settings}` |
| `.g4d` v3: `takes` (each with its overrides), `active_take`, `main_values`, `render_take` — omitted without takes | `grid4d-io::v3` |

## Decisions (2026-10-30)

1. **The document holds the active take** (plan D14): nothing that reads the
   document — panels, evaluation, the path tracer — knows about takes.
2. **Switching is a pure swap** whose inverse is switching back; Auto Take
   keeps the active take equal to what the document shows, so the swap loses
   nothing.
3. **Auto Take is always on** (the reference's usual way of working); there is
   no record button.
4. **Later:** take hierarchies (child takes inheriting a parent's overrides),
   override groups shared between objects, Render Marked Takes, a take per
   layer state, comparing takes side by side.

## Acceptance criteria

- [x] A take with another camera, a hidden object and a red material renders
      differently from Main, and switching back restores Main exactly.
- [x] Edits in a take are recorded as its overrides in the same undo step;
      clearing an override brings back Main's value.
- [x] Takes are created, renamed, deleted and switched, each undoable, and
      saved with the document.
- [x] Render Settings ▸ Take renders another take without making it active.
