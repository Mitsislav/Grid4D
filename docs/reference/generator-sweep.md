# Sweep generator

> Sources (read 2026-10-29): manual —
> [Sweep Object](https://help.maxon.net/c4d/en-us/Content/html/OSWEEP.html),
> [its Object properties](https://help.maxon.net/c4d/en-us/Content/html/OSWEEP-ID_OBJECTPROPERTIES.html),
> [Caps (S22)](https://help.maxon.net/c4d/s22/us/html/OSWEEP-ID_CAPSANDBEVELS.html).
> Rotation-minimising frames: parallel transport along a polyline (a public,
> textbook technique). Written in our own words — no copied text or images.
>
> Covers implementation-plan step **9.6**. Builds on
> [`generator-extrude.md`](generator-extrude.md) (profiles with holes, caps).

## Purpose

Moves a cross-section (the *contour* or *profile*) along a path: a circle along
a curve becomes a hose or a cable, a rectangle along a helix a spiral ramp, a
moulding profile along a rectangle a picture frame.

## Generator behaviour

- **Inputs:** the generator's **first child** is the profile, the **second
  child** the path; both must be splines. (A third child, the *rail*, is later.)
- **The profile** is read in its own object's space and should lie in its local
  **XY plane**: moving the profile object changes nothing; moving or editing the
  path moves the result. Along the path, the profile's Z axis follows the path's
  direction, its X and Y axes turn with it.
- **The path** is read in the generator's space. Each of its segments is swept
  separately; a closed segment gives a closed tube (no caps, the last ring joined
  to the first).
- **Orientation of the profile along the path:**
  - **Banking on** (default): the profile's X axis starts in the path's mean
    plane and then turns as little as possible from one point to the next
    (parallel transport). On a closed path the remaining twist is spread evenly,
    so the start and end match.
  - **Banking off**: at every point the X axis stays level (perpendicular to the
    world Y axis); the profile twists where the path rises steeply.
  - **Parallel Movement**: the profile is not turned at all, only moved.
- **Constant Cross Section** (default on): at a corner of the path the profile
  is stretched across the corner (by 1 / cos of half the turn) so the walls keep
  their thickness; off, corners pinch.
- **End Scale / End Rotation** grow and twist the profile linearly from the
  start (100 %, 0°) to the end of the path.
- **Start Growth / End Growth** sweep only part of the path (by length):
  0 % – 100 % is all of it. A partly swept closed path becomes open and gets
  caps. End scale and rotation keep following the whole path.
- **Caps:** closed profiles get a start and an end cap (holes cut out, as for
  the Extrude) where the swept path is open. Surfaces face outwards (outlines
  counter-clockwise in the profile's XY plane); **Reverse Normals** flips the
  shell only.
- Q, Alt + create (wraps the selection: with two objects selected, the first
  selected becomes the profile and the second the path; with one, it becomes
  the profile), Make Editable (bakes one polygon object and removes profile and
  path) and rendering work as for every generator; the input splines stay
  drawn.

## Parameters

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| End Scale | 0 % – any | 100 % | The profile's size at the end of the path |
| End Rotation | degrees, any | 0° | How far the profile has turned about the path at its end |
| Start Growth | 0 – 100 % | 0 % | Where along the path the sweep starts |
| End Growth | 0 – 100 % | 100 % | Where it ends |
| Parallel Movement | bool | off | Move the profile without turning it |
| Constant Cross Section | bool | on | Keep the thickness at corners |
| Banking | bool | on | Turn the profile with the path's curvature (parallel transport) rather than keeping it level |
| Reverse Normals | bool | off | Turns the shell inside out (caps untouched) |
| Start Cap / End Cap, Caps Type | as the Extrude | on, on, N-Gons | |

Not in 9.6 (later): the rail spline (Use Rail Direction / Scale, 2-Rail),
the Scale and Rotation function graphs, Keep Segments, Stick UVs, Isoparm
display, bevels.

## Edge cases

- Fewer than two children, a child that is not a spline, an empty spline, a
  path of zero length, End Growth ≤ Start Growth: nothing.
- Repeated path points are skipped; a path turning back on itself keeps the
  previous direction.
- Corner stretch is limited (at most 5×) so a hairpin cannot explode.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `Sweep` options, `sweep(&profile, &path, &Sweep) -> PolyMesh` (frames, growth, miters; profile nesting and caps shared) | `grid4d-geom::surface` |
| `ObjectKind::Sweep(Sweep)`, tag `Sweep`, `GENERATORS` | `scene/node.rs` |
| `Options::Sweep`: the first two children as inputs (profile in its own space, path in the generator's) | `grid4d-eval::generator` |
| Make Editable removes both inputs | `ui/src/commands.rs` |
| `.g4d` v3 `sweep` kind (additive; defaults omitted; values clamped) | `grid4d-io::v3` |
| Icon, Create ▸ Generator, toolbar, Attribute Manager rows | `grid4d-ui` |

## Decisions (2026-10-29)

1. **Profile in its own space**, path in the generator's (the reference reads
   the contour in its local XY plane).
2. **Banking's start**: the mean plane's normal (Newell) with its largest
   component made positive is the profile's Y axis at the start; a straight
   path starts level, as with Banking off.
3. **Growth** cuts the path by length, exactly (no Keep Segments).

## Acceptance criteria

- [x] A circle along a straight path makes a closed, capped cylinder of the
      path's length facing outwards; along a closed path, a closed tube without
      caps.
- [x] Banking off keeps the profile's X axis level; Parallel Movement keeps it
      unturned; banking on a closed path ends where it started.
- [x] A square path with Constant Cross Section keeps the wall thickness at
      its corners; off, the corners are thinner.
- [x] End Scale 50 % halves the last ring; End Rotation 90° turns it; growth
      50 % sweeps half the length and caps it.
- [x] Saved and loaded; rebuilt only when inputs change; Make Editable removes
      both inputs in one undo step; rendered.
