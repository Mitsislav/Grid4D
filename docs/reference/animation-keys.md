# Animation data: tracks, curves and keys

> Sources (read 2026-10-20): Cinema 4D manual —
> [Key properties](https://help.maxon.net/c4d/en-us/Content/html/CKVALUE-ID_VKEY_PROPERTIES.html)
> (interpolation Spline / Linear / Step; Auto Tangents make the curve as smooth as
> possible and belong with Spline; Clamp keeps a curve from overshooting at a
> minimum or maximum and between keys of equal value) and the
> [Keys menu](https://help.maxon.net/c4d/2026/en-us/Content/html/10616.html)
> (Linear and Step give abrupt transitions; Spline keeps the tangents). Bézier
> curves and Catmull-Rom-style slopes are public textbook material. Written in our
> own words.
>
> Covers implementation-plan step **6.2** (data, evaluation, file format and tests).
> Recording keys, autokeying, key indicators and keys on the timeline are 6.3; the
> F-Curve editor is 6.4.

## Purpose

An animated parameter changes over time. Each animated parameter of an object has
a **track** holding a **curve**: a list of **keys** (a frame and a value). Between
keys the curve interpolates; during playback the viewport shows the object as the
curves say at the current frame.

## Parameters

| Element | Field | Type / range | Default |
|---------|-------|--------------|---------|
| Track | parameter | Position X/Y/Z (cm), Rotation H/P/B (radians), Scale X/Y/Z | — |
| Key | frame | any finite frame (fractions allowed) | — |
| | value | finite number | — |
| | interpolation | **Spline** · Linear · Step — of the segment that starts at this key | Spline |
| | tangents | **Auto** (smooth, clamped) · Manual (left and right handles: frames ≥ 0 and value offsets) | Auto |

## Behaviour

- **At a key** the curve has the key's value exactly.
- **Step**: the value stays until the next key, then jumps.
- **Linear**: a straight line to the next key.
- **Spline**: a cubic Bézier segment from the key's right handle to the next key's
  left handle; the frame of a point on the curve always increases, so every frame
  has one value.
- **Auto tangents**: the slope at a key is that of the line through its two
  neighbours (the first and last keys are flat); **clamped**: a key at a local
  minimum or maximum, or next to a key with the same value, gets a flat slope, so
  the curve does not overshoot there. Handles reach a third of the way to the
  neighbouring key on each side, which keeps the slope the same on both sides of a
  key (the curve is smooth through it).
- **Before the first key / after the last key**: the curve keeps the first / last
  value (the reference's *Constant*; *Repeat*, *Oscillate* … are later).
- **Keys are kept sorted**; setting a key at a frame that already has one replaces
  it.
- **Evaluation** happens at the current (whole) frame. It changes what is drawn,
  picked and framed, never the document: the object's own Coordinates keep the
  values the user set. A child of an animated object follows it. The evaluation
  runs again only when the frame changes *and* something is animated.
- **Undo and files:** tracks belong to their object (deleting the object deletes
  them, undo brings them back) and are saved in `.g4d` (version 3 grows
  additively). Non-finite frames or values are refused.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `Param` (`Position(Axis)`, `Rotation(Axis)`, `Scale(Axis)`), `Key`, `Interpolation`, `Tangents`, `Curve` (`set_key`, `remove_key`, `value_at`), `Track` | `grid4d-scene` (`anim.rs`); tracks live in `NodeData::tracks` (one per parameter), so edits are `SetData` and undo/save work as for any attribute |
| The animated transform | `NodeData::animated_transform(frame)` |
| Evaluation at a frame | `grid4d-eval`: `EvaluatedScene::set_frame`; world matrices recomputed in pre-order with animated transforms (before Target tags are applied) |
| Viewport | sets the document's current frame before each update |
| File format | `.g4d` v3 `TrackDto` (optional per object) |

The plan sketched a document-level list of tracks naming their nodes; storing them
on the object instead keeps animation with its object through deletes, undo,
merges and saves without extra bookkeeping.

## Acceptance criteria

- [x] Step, Linear and Spline segments give the right values at and between keys;
      before the first and after the last key the value holds.
- [x] Auto tangents: the slope is continuous through interior keys; a local
      extremum is flat (no overshoot); manual handles set the slope at their key.
- [x] Keys stay sorted; a key at an existing frame replaces it; removing works.
- [x] An animated object (and its children) is drawn where its curves put it at the
      current frame; the document's transform is unchanged; an unanimated scene is
      not re-evaluated when the frame changes.
- [x] Tracks survive save and reload exactly; non-finite data is refused.

Known gaps until 6.3: the gizmo and the Coordinates fields work on the object's own
values (an animated parameter shows its curve, not the dragged value); a
Subdivision Surface's result does not follow an animated cage yet.
