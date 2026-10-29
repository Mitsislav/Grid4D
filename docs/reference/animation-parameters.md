# Animating object parameters

> Sources (read 2026-10-29): manual — the Attribute Manager's animation
> functions (key buttons beside parameters, Autokeying) as described for
> transforms in [`animation-recording.md`](animation-recording.md), which this
> extends to an object's own parameters. Written in our own words — no copied
> text or images.
>
> Covers implementation-plan step **10.1**. Builds on
> [`animation-keys.md`](animation-keys.md), [`animation-recording.md`](animation-recording.md)
> and [`animation-fcurves.md`](animation-fcurves.md).

## Purpose

Lets an object's own numbers change over time, not only its position,
rotation and scale: a sphere that grows, an Extrude whose depth rises, a
Lathe that closes from 0° to 360°, a Sweep that grows along its path, a light
that dims. Later steps rely on it (a Bend's strength, a Cloner's count).

## Behaviour

- **Which parameters:** every real-valued parameter in an object's Object tab
  — sizes, radii, heights, angles, offsets, spacings, percentages, a camera's
  field of view, a light's intensity, angles and falloff radius. Counts
  (segments, sides, subdivision) and choices (types, planes, check boxes) are
  not animatable in this step. Optional values (a Circle's ellipse or ring
  radius, a Rectangle's rounding radius) are animatable while they are on.
- **Key buttons:** each animatable row in the Attribute Manager's Object tab
  has the same small key button as the Coordinates fields: grey ring (not
  animated), red ring (animated), red disc (a key at this frame), yellow (a
  temporary value). Click: set a key at the current frame with the value
  shown, or remove the key shown; Shift+Ctrl+click: delete the parameter's
  track. Each click is one undo step.
- **Values at the current frame:** the Object tab shows every parameter as
  it is at the current frame. Editing an animated parameter with
  **Autokeying** on writes a key at the current frame; with Autokeying off the
  value is a temporary one, shown only at that frame and dropped when the frame
  changes (the yellow key button), exactly as for transforms. Editing an
  unanimated parameter changes it for all frames.
- **Everywhere:** the viewport, picking, generators (an Extrude of an animated
  Circle, a Lathe whose angle is keyed), renders of an animation and the
  Render View all use the parameters at the frame they show.
- **Curves:** the F-Curve editor lists the new tracks under each object after
  its transform tracks, labelled like the Attribute Manager rows; angles are
  shown in degrees, percentages in percent. The timeline's key strip shows
  their keys, and moving or deleting keys there moves or deletes them too.
- **Record (F9)** still records position, scale and rotation only (its
  settings); parameters are keyed with their buttons or Autokeying.
- **Make Editable** bakes the shape at the current frame and drops the tracks
  of parameters the new object no longer has (a polygon object has none).
- **Saving:** parameter tracks are saved with the object, by stable names.

## Parameters and their names

| Objects | Animatable (row label in the Attribute Manager) |
|---------|--------------------------------------------------|
| Cube | Size . X / Y / Z |
| Sphere | Radius |
| Cylinder, Capsule | Radius, Height |
| Plane | Width, Height |
| Cone | Top Radius, Bottom Radius, Height |
| Torus | Ring Radius, Pipe Radius |
| Disc | Inner Radius, Outer Radius |
| Camera | Field of View (Vertical) |
| Light | Intensity; Inner Angle, Outer Angle (spots); Radius/Decay (with a falloff) |
| Circle | Radius; Radius Y (ellipse on); Inner Radius (ring on) |
| Rectangle | Width, Height; Rounding Radius (rounding on) |
| Arc | Radius, Inner Radius (Ring type), Start Angle, End Angle |
| n-Side | Radius; Rounding Radius (rounding on) |
| Star | Inner Radius, Outer Radius, Twist |
| Helix | Start Radius, Start Angle, End Radius, End Angle, Radial Bias, Height, Height Bias |
| Flower | Inner Radius, Outer Radius |
| Text | Height, Horizontal Spacing, Vertical Spacing |
| Extrude | Offset (not with Absolute), Movement . X / Y / Z (Custom, Absolute) |
| Lathe | Angle, Movement, Scaling |
| Sweep | End Scale, End Rotation, Start Growth, End Growth |

(The rounding radius row was renamed from "Radius" to "Rounding Radius" so
that each label names one parameter.)

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `ObjectParam` (closed enum; `label`, `index`, angle / percent display), `ObjectKind::{params, param, set_param}` | `grid4d-scene::param` |
| `Param::Object(ObjectParam)`; `ParamSet` widened (u128); `NodeData::{kind_at, set_kind_at}`; `value_at`, `set_value_at`, `key_state`, `record`, `remove_keys` for any parameter | `grid4d-scene::anim` |
| The kind at the frame in evaluation (drawables, curves, generators and their inputs, lights) — borrowed, no allocation, when nothing is animated | `grid4d-eval` |
| Key buttons in the Object tab (`KeyRows`: label → parameter for the shown kind), Autokeying, temporary values | `ui/panels/attributes.rs` |
| Tracks listed after the transform ones, angle / percent display, colours | `ui/panels/fcurve.rs` |
| Make Editable and export at the current frame | `ui/src/commands.rs`, `ui/src/file.rs` |
| `.g4d` v3: parameter names as strings (`"radius"`, `"size_x"`, …), additive | `grid4d-io::v3` |

A parameter counts as present exactly when its row is shown: a test draws the
Object tab of every object (optional values on, a spot light) and checks that
each parameter gets exactly one key button.

## Decisions (2026-10-29)

1. **Real-valued parameters only** in 10.1; counts follow when a step needs
   them (the Cloner's count in 10.6), rounded to whole numbers.
2. **One enum, semantic names** (Radius, Height, …) shared across kinds, so a
   file names parameters by meaning; a track whose parameter the object does
   not have is ignored.

## Acceptance criteria

- [x] A Sphere's Radius keyed 100 → 200 between frames 0 and 10 is 150 at
      frame 5 in the viewport, in a render and in the Attribute Manager.
- [x] An Extrude's Offset and a Lathe's Angle animate their results; an
      animated Circle under an Extrude changes the solid.
- [x] Key buttons: set, remove, delete track, each one undo step; Autokeying
      writes keys; without it the value is temporary and goes when the frame
      changes.
- [x] Every animatable parameter of every object has exactly one key button
      in the Object tab.
- [x] The F-Curve editor and the key strip show and edit the new tracks.
- [x] Saved and loaded; idle frames do not touch the document and evaluate
      without allocating.
