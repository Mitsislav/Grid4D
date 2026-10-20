# The F-Curve editor

> Sources (read 2026-10-20): Cinema 4D manual —
> [Keys Area](https://help.maxon.net/c4d/s22/us/html/10608.html) (F-Curve mode:
> curves of the selected tracks; tangents shown on selected keys and dragged
> interactively, Shift edits one side alone, Ctrl keeps the angle, Alt the length;
> click or drag a box to select keys, Ctrl toggles; Ctrl+click on a curve adds a
> key), [Keys menu](https://help.maxon.net/c4d/s22/us/html/10616.html) (Spline /
> Linear / Step, Auto Tangents, Zero Angle: a flat tangent, so the curve does not
> pass the key's value nearby; Zero Length: no tangent, two such keys give a
> straight segment), [F-Curve menu](https://help.maxon.net/c4d/s22/us/html/10617.html)
> (zooming happens around the cursor; Shift changes the proportional behaviour),
> [View menu](https://help.maxon.net/c4d/s22/us/html/10613.html) (Tab switches
> between the Dope Sheet and F-Curve modes) and the
> [keyboard shortcuts](https://help.maxon.net/c4d/en-us/Content/html/Shortcuts.html)
> (Timeline: Tab switches modes, H shows all, S shows the selection). Written in our
> own words.
>
> Covers implementation-plan step **6.4**. Keys and curves are
> [`animation-keys.md`](animation-keys.md); recording them is
> [`animation-recording.md`](animation-recording.md).

## Purpose

Shaping animation precisely: every animated value of the selected objects is drawn
as a curve of value over time. You move keys in time and value, shape the curve
between them with tangent handles, and change how segments interpolate.

## Layout

- A tab **F-Curve** beside **Timeline** at the bottom of the window (**Tab** over
  the panel switches between them). The F-Curve tab is taller and can be resized.
- **Track list** (left): the selected objects and their animated values (Position
  X/Y/Z, Rotation H/P/B, Scale X/Y/Z), each with its colour (X red, Y green, Z
  blue). Click a track to show only that curve; Shift+click adds or removes tracks;
  a click on an object's name shows all tracks again.
- **Graph** (right): frames left to right, values bottom to top, a grid with
  numbers on both axes, the current frame as a vertical line. Rotation is shown in
  degrees, positions in cm.
- **Header**: the tabs, and the frame and value of the selected key (editable when
  one key is selected).

## Behaviour

| Action | Result |
|--------|--------|
| Click a key | selects it (orange); **Ctrl+click** toggles it; click on empty space clears |
| Click a curve between keys | selects all of that curve's keys |
| Drag on empty space | selects the keys inside the box (Ctrl: toggles them) |
| Drag a selected key | moves all selected keys: time in whole frames, value freely; **Shift** keeps the movement horizontal or vertical (whichever is larger) |
| Drag a tangent handle | sets manual tangents: the other handle keeps the same slope (the curve stays smooth); **Shift** moves this handle alone (broken tangent) |
| **Ctrl+click** on a curve | adds a key there, on the curve |
| Wheel | zooms around the cursor (Ctrl: time only, Shift: values only) |
| Middle drag (or Alt+left drag) | pans |
| **H** / **S** (pointer over the graph) | frames all curves / the selected keys |
| Click or drag on the frame numbers | sets the current frame |
| **Delete** after a click in the graph | deletes the selected keys |
| Right-click | Interpolation: Spline, Linear, Step · Tangents: Auto, Zero Angle, Zero Length · Delete Keys · Frame All / Selected |

- Tangent handles show on selected keys, on the sides where a Spline segment uses
  them.
- **Zero Angle** makes both handles flat (keeping their lengths); **Zero Length**
  shrinks them to nothing; **Auto** returns to automatic tangents.
- Interpolation belongs to the segment that starts at a key (as in 6.2).
- Moving keys over other keys of the same curve replaces those.
- Every drag, menu command or typed value is **one undo step**; keys of several
  objects change together.
- The view frames all curves when the shown tracks change.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| Effective handles, tangent presets, moving keys in time and value | `grid4d-scene::anim` (`Curve::handles`, `Curve::set_tangents`, `Curve::shift_keys`) |
| Curve key selection, shown tracks, focus | editor state (`EditorState::curves`) |
| Menu commands and Delete | `EditorCommand::EditCurveKeys { op }` (one `commit`, edits built in `ui/src/keys.rs`) |
| Drags | written live, then `EditorCommand::CommitLiveAll { inverses, label }` |
| Panel | `ui/src/panels/fcurve.rs`; tab switching in `EditorState::bottom` |

Deliberate differences: no Dope Sheet (the Timeline's key strip plays that role);
Ctrl / Alt while dragging a handle (keep angle / length), Ease presets, Mirror,
Quantize, copying keys and relative (normalised) curves are later.

## Acceptance criteria

- [x] The curves of the selected objects' tracks are drawn with their keys; the
      track list filters them.
- [x] Keys can be selected (click, Ctrl, box, curve), moved (whole frames, Shift
      constrains) and deleted; each change is one undo step.
- [x] Dragging a handle makes the tangents manual and keeps the curve smooth
      through the key; Shift breaks it.
- [x] Spline / Linear / Step and Auto / Zero Angle / Zero Length change the
      selected keys.
- [x] Ctrl+click adds a key on the curve without changing its shape there.
- [x] Zoom, pan, H and S work; Tab switches between the Timeline and the F-Curve tab.
- [x] Idle frames leave the document untouched.
