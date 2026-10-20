# Recording keys: Record, Autokeying, key indicators, keys on the timeline

> Sources (read 2026-10-20, Cinema 4D 2026 manual):
> [Record Active Objects](https://help.maxon.net/c4d/en-us/Content/html/10639.html)
> (a key for each selected object at the current time, with the palette's
> settings), [Autokeying](https://help.maxon.net/c4d/en-us/Content/html/10640.html)
> (changes become keys by themselves; the view gets a red frame; objects need an
> existing track, so Record first),
> [Various methods of creating tracks and keys](https://help.maxon.net/c4d/en-us/Content/html/10592.html)
> (the Attribute Manager's key buttons and their colours, temporary values that
> are dropped when the time changes, Shift+Ctrl+click removes a track),
> [Animation Palette](https://help.maxon.net/c4d/s22/us/html/10594.html) (keys of
> the active objects in the lower half of the time ruler: select by clicking or
> dragging a frame, move, delete; Ctrl+click records a key without moving the
> time) and the [keyboard shortcuts](https://help.maxon.net/c4d/en-us/Content/html/Shortcuts.html)
> (F9 Record Active Objects, Ctrl+F9 Autokeying, Ctrl+F / Ctrl+G previous / next
> key). Written in our own words.
>
> Covers implementation-plan step **6.3**. The data and its evaluation are
> [`animation-keys.md`](animation-keys.md) (6.2); the F-Curve editor is 6.4.

## Purpose

Turning edits into animation: go to a frame, place an object, record. The
Attribute Manager shows which parameters are animated and whether the current
frame holds a key; the timeline shows the keys of the selected objects and lets
you select, move and delete them.

## Parameters (editor state: not saved, not undoable)

| Name | Type | Default | Effect |
|------|------|---------|--------|
| Autokeying | on / off | off | Changes to animated parameters become keys at the current frame |
| Record Position / Scale / Rotation | on / off each | all on | Which channels Record writes |

## Behaviour

### What a parameter shows

- An object's coordinates as shown (Attribute Manager, gizmo, viewport) are its
  values **at the current frame**: an animated parameter shows its curve's value.
- **Temporary (unkeyed) values.** Changing an animated parameter without
  Autokeying changes it for the current frame only: the object shows the new
  value, and its key button turns yellow. Recording keeps it (it becomes a key);
  changing the frame drops it, and the curve's value shows again.
- An unanimated parameter is simply changed, as before.

### Record Active Objects (F9, the timeline's record button, Animate menu)

- For every selected object, writes a key at the current frame for each
  parameter of the enabled channels (position X/Y/Z, scale X/Y/Z, rotation
  H/P/B), with the values shown at that frame (temporary values included).
- Tracks that do not exist yet are created. A key already at that frame gets the
  new value and keeps its interpolation and tangents; a new key gets the defaults
  (Spline, Auto).
- One undo step; nothing is recorded with nothing selected.
- **Ctrl+click in the key strip** records the same way at the clicked frame,
  without moving the time: the values are those shown at the current frame.

### Autokeying (Ctrl+F9, the timeline's autokey button, Animate menu)

- While on, the viewport is framed in red.
- A change to an **animated** parameter (gizmo, Coordinates fields, Reset
  Transform) writes its key at the current frame instead of a temporary value.
- Unanimated parameters are changed without a key, as the reference requires an
  existing track: Record once to start a parameter's animation.
- A drag or a typed value is one undo step, keys included.

### Key buttons in the Attribute Manager (Coord. tab)

One small circle before each of the nine fields:

| Circle | Meaning |
|--------|---------|
| grey ring | not animated |
| red ring | animated, no key at this frame |
| red disc | a key at this frame |
| yellow ring / disc | a temporary value differs from the curve (no key / a key here) |

- **Click**: sets a key at the current frame with the shown value (creating the
  track); on a red disc (a key with the shown value) it removes that key.
  Removing the last key removes the track; the parameter keeps the value shown.
- **Shift+Ctrl+click**: removes the parameter's track with all its keys; the
  parameter keeps the value shown at the current frame.
- Each click is one undo step. Protected channels (Protection tag) are greyed out
  but their buttons still work, like recording.

### Keys on the timeline

- A strip under the frame ruler shows a mark at every frame where any selected
  object has a key (all its parameters together), at the ruler's scale.
- **Click** a mark to select its frame's keys; **Shift+click** adds or removes
  it; a click on empty space clears the key selection; **dragging on empty
  space** selects the marks inside the dragged range (Shift adds).
- **Dragging a mark** moves the selected keys (of every selected object, every
  parameter) by whole frames; the marks follow while dragging and the keys move
  on release (one undo step). Keys already at a destination frame are replaced.
- **Delete / Backspace** after clicking in the strip, or the strip's context menu
  *Delete Keys*, removes the selected keys (one undo step). Clicking anywhere
  outside the strip gives Delete back to objects.
- Changing the object selection clears the key selection.
- **Ctrl+F / Ctrl+G** go to the previous / next frame with a key of a selected
  object.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| Values at a frame, temporary values (`Unkeyed { frame, params }` in `NodeData`, never saved), editing through `set_transform_at`, key buttons' `KeyState`, recording, removing and moving keys | `grid4d-scene` (`anim.rs`) |
| World matrices at a frame for the gizmo | `Scene::world_matrix_at` / `parent_world_matrix_at` |
| Evaluation uses the values at the frame (temporary values included) | `grid4d-eval` |
| Autokeying, record channels, key selection | `EditorState::animation`, `EditorState::keys` |
| `RecordKeys { at }`, `DeleteKeys`, `MoveKeys { by }` | `EditorCommand`, keys gathered in `ui/src/keys.rs`; each is one `commit` of `SetData` edits |
| Dropping temporary values when the frame changes | `commands::apply_pending` (not an undo step) |
| Key strip | `panels/timeline.rs` |

Deliberate differences:

- Grid4D keeps the document's own values untouched by playback (6.2). A
  temporary value is stored in the object's base transform, flagged with its
  frame, and only shown at that frame.
- Grid4D's key strip shows the keys of all selected objects merged per frame;
  the reference shows them split by channel only in an advanced mode.
- Keys removed down to none remove their track (the reference keeps an empty
  track).

## Acceptance criteria

- [x] F9 records position, scale and rotation keys of every selected object at the
      current frame (the channels follow the toggles); one undo step; undo
      restores the exact data.
- [x] Recording keeps an existing key's interpolation and tangents.
- [x] Without Autokeying, a change to an animated parameter is temporary: shown at
      that frame (viewport, Attribute Manager), dropped when the frame changes,
      kept by Record.
- [x] With Autokeying, the same change writes a key at the current frame; an
      unanimated parameter changes without a key.
- [x] The key buttons show the five states; click sets or removes a key,
      Shift+Ctrl+click removes the track; the value shown stays.
- [x] The gizmo sits on the animated object and its drags follow the rules above.
- [x] The timeline's key strip selects, moves (by whole frames) and deletes keys of
      the selected objects, each move or delete one undo step.
- [x] Ctrl+F / Ctrl+G jump between key frames; Ctrl+F9 toggles Autokeying and the
      view shows a red frame.
- [x] Idle frames still leave the document untouched.
