# Preferences

> Sources (read 2026-10-30): manual — the Edit menu (the undo buffer and its
> depth), the Preferences' Units page (the unit lengths are shown in, values
> kept in one internal unit). Written in our own words — no copied text or
> images. Autosave with recovery and the view defaults are our own design.
> The reference's Preferences shortcut (Ctrl+E in third-party lists) is not in
> Maxon's own shortcut pages, so it is not bound yet.
>
> Covers implementation-plan step **12.2** (themes follow in 12.3, the
> language in 12.6).

## Purpose

Settings that belong to the user rather than to a document: how lengths read,
how far undo reaches, whether unsaved work is kept safe, and how new views
look. They are the same for every document and survive a restart.

## Behaviour

- **Edit ▸ Preferences…** opens the Preferences window (not modal: the scene
  stays usable). Changes take effect at once; there is no OK button.
- **Units.** *Length Unit*: Centimetres (default), Metres, Millimetres, Inches
  or Feet. Every length field (positions, sizes, radii, offsets, tolerances,
  the Coordinate Manager) and the speeds and gravity built on lengths show
  their value in that unit with its symbol (`cm`, `m`, `mm`, `in`, `ft`), and
  so does the viewport's *Grid Spacing*. Typing a value means the chosen unit;
  a value typed with a symbol (`2 m`, `15mm`, `3 in`) means that unit and is
  converted. Documents keep centimetres: changing the unit changes no value.
- **Undo.** *Undo Depth* (1 – 10 000, default 100): how many steps undo keeps.
  Lowering it drops the oldest steps at once.
- **Autosave.** *Autosave* (on by default) and *Every* (1 – 120 minutes,
  default 5). While the document has unsaved changes, a recovery copy is
  written that often into the user data folder (`recovery/`); it is removed
  when the document is saved, replaced by a clean one, or Grid4D closes
  normally. Saving the copy never touches the document's own file.
- **Recovery.** If Grid4D starts and finds a recovery copy (it did not close
  normally), it asks *Recover Unsaved Work?*, naming the document and when the
  copy was written:
  - **Recover** opens the newest copy as the current document, with unsaved
    changes, linked to the file it came from (Save writes there; Save As keeps
    both);
  - **Discard** deletes the copies.
  Older copies are removed either way; **Esc** closes the question and keeps
  the copy for the next start.
- **Views.** Defaults for the views: *Shading* (any of the Display menu's
  shading modes), *Grid*, *World Axis* and *HUD*. They apply to every view
  when Grid4D starts; **Apply to Views** applies them now.
- **Remembered.** The preferences are saved with the window and come back on
  the next start; values from a newer or older version that are missing or
  out of range take their defaults.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `Preferences` (units, undo depth, autosave, view defaults; serialisable), `Units` (factor, symbol, format, parse), `ViewDefaults` | `ui::preferences`; `EditorState::preferences` |
| `widgets::length`, `widgets::length_per` — a `DragValue` over centimetres shown in the current unit (the unit is set on the egui context each frame) | `ui::widgets` |
| `History::{limit, set_limit}` (trims the oldest steps, purging what only they could restore) | `ui::history`, applied by `commands::apply_pending` |
| `Autosave` (the recovery folder, this session's copy, the last state written, a background writer), `Recovery` (the copy to offer) | `ui::autosave`; `EditorState::autosave` |
| The Preferences window; the Recover dialog | `ui::panels::{preferences, recovery}` |
| `EditorCommand::{ApplyViewDefaults, Recover}`; `ViewportPanel::apply_defaults` | `ui::commands`, `ui::panels::viewport` |
| Saved by the binary in eframe storage; the recovery folder next to it | `src/app.rs` |

## Decisions (2026-10-30)

1. **Lengths stay centimetres inside**; the unit is display and input only.
   Values are shown with up to the field's usual decimals in that unit.
2. **The unit is read from the egui context**, not passed through every
   panel function: the app sets it each frame, tests that do not set it see
   centimetres.
3. **One recovery copy per running Grid4D**, written by a background thread
   (a scene copy is cheap: meshes are shared handles), to a temporary name
   then renamed, so a crash while writing leaves the previous copy.
4. **No shortcut** for Preferences until one is verified.
5. **Later:** the F-Curve editor's value axis and the status bar in the
   chosen unit; a project scale; per-document units.

## Acceptance criteria

- [x] Lengths show and are typed in the chosen unit (with symbols converted);
      the document's values do not change.
- [x] Undo Depth limits the steps kept, at once when lowered.
- [x] A recovery copy is written only while there are unsaved changes, is
      removed when they are saved and on a normal exit, and after a crash the
      newest copy is offered and recovered with unsaved changes.
- [x] New views take the default shading, grid, axis and HUD; Apply to Views
      sets them on the open views.
- [x] Preferences survive a restart.
