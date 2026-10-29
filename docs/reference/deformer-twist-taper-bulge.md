# Twist, Taper and Bulge deformers; the strength handle

> Sources (read 2026-10-29): manual —
> [Twist Object properties](https://help.maxon.net/c4d/en-us/Content/html/OTWIST-ID_OBJECTPROPERTIES.html),
> [Taper Object properties](https://help.maxon.net/c4d/en-us/Content/html/OTAPER-ID_OBJECTPROPERTIES.html),
> [Bulge Object properties](https://help.maxon.net/c4d/en-us/Content/html/OBULGE-ID_OBJECTPROPERTIES.html).
> The manual names the parameters and their effect, not the formulas; the
> shapes below are our own (classic twist and taper of Barr, 1984, public).
> Written in our own words — no copied text or images.
>
> Covers implementation-plan step **10.3**; the framework (box, Mode, who is
> deformed, Fit to Parent) is in [`deformer-basics.md`](deformer-basics.md).

## Twist

Turns the object about the deformer's Y axis, more and more along the box's
height: a twisted column, a screw, a wrung cloth.

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Size, Mode, Fit to Parent | as every deformer | 250 cm × 3, Limited | |
| Angle | degrees, any | 0° | How far the top of the box turns relative to its bottom (720° = two turns); positive turns X towards −Z |

A point at height `s` into the box (from its bottom) turns by `Angle · s / height`.
Limited: below the box nothing turns, above it everything turns by the whole
Angle; Unlimited: the turn continues; Within Box: only points inside turn.

## Taper

Narrows (or widens) the object towards the top of the box: a spire, a tree
trunk, a bottle's neck.

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Size, Mode, Fit to Parent | as every deformer | 250 cm × 3, Limited | |
| Strength | percent, any | 0 % | How much narrower the top gets: 100 % to a point, negative widens |
| Curvature | percent, ≥ 0 | 0 % | 0 %: a straight taper; more: the narrowing curves, slow at the bottom and fast at the top |
| Fillet | bool | off | Soft start and end: the taper eases in at the bottom and out at the top |

Each point's distance from the Y axis (X and Z) is scaled by
`1 − Strength · g(t)`, `t` the height through the box (0 at the bottom, 1 at
the top), `g(t) = t^(1 + Curvature)`, eased by `3g² − 2g³` with Fillet. The
scale never goes below zero.

## Bulge

Swells (or pinches) the middle of the box: a barrel, a vase, a waist.

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Size, Mode, Fit to Parent | as every deformer | 250 cm × 3, Limited | |
| Strength | percent, any | 0 % | How much wider the middle gets: 100 % twice as wide; negative pinches |
| Curvature | percent, ≥ 0 | 0 % | 0 %: a round swell; more: a flatter, wider swell with steeper ends |
| Fillet | bool | off | Softens the swell where it meets the top and bottom of the box |

The distance from the Y axis is scaled by `1 + Strength · b(t)`, with
`b(t) = 1 − |2t − 1|^(2 + 2·Curvature)`, eased by `3b² − 2b³` with Fillet, and
never below zero. Outside the box's height the bulge is zero (Limited and Within
Box); Unlimited continues the curve (clamped at zero).

## The strength handle

- With a deformer selected (Model mode), an **orange handle** sits on its box:
  - **Bend**: at the end of the bending part (where the box's top centre went);
  - **Twist**: at the top of the box, on its +X side, turned with the twist;
  - **Taper**: at the top of the box, on its +X side, at the tapered width;
  - **Bulge**: at the middle of the box, on its +X side, at the swollen width.
- **Drag it** to change the strength: the Bend follows the pointer along its
  arc (in the bend's plane), the Twist turns with the pointer around the axis
  (in the top's plane, winding on past a full turn), Taper and Bulge follow
  the pointer's distance from the axis (in the deformer's XY plane).
- **Shift** steps the value: 5° for angles, 5 % for percentages.
- One undo step per drag; **Esc** while dragging restores. With Autokeying on
  (and the parameter animated), the drag writes a key, like any other edit.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `Twist`, `Taper`, `Bulge` (`apply`, `box_lines`), the handle of every deformer (`handle`, `handle_plane`, `with_handle`) | `grid4d-geom::deform` |
| `ObjectKind::{Twist, Taper, Bulge}`; animatable Size, Angle (Twist), Amount (Strength) and Curvature (Taper, Bulge) | `scene`, `scene::param` |
| `Deformer::{Twist, Taper, Bulge}` in chains; their boxes as curves | `grid4d-eval` |
| The handle: drawn and dragged (`viewport/deformer_tools.rs`), one `CommitLive`, Esc, Shift steps | `grid4d-ui` |
| `.g4d` v3 kinds (additive) | `grid4d-io::v3` |

## Decisions (2026-10-29)

1. **Formulas are ours**, chosen to match the described effects; the curves of
   Curvature and Fillet are stated above so they can be tested.
2. **Taper and Bulge** scale both X and Z (no Angle); the manual's Angle for
   the Taper is not in its parameter list.
3. **Shift steps** handle values (5° / 5 %), as Shift snaps rotations elsewhere.
4. **The handle comes before the gizmo**: a press on the handle drags it even
   where it sits on one of the move arrows (Bulge and Taper handles lie on the
   X axis).

## Acceptance criteria

- [x] Twist 90°: the top of the box turns a quarter, the bottom stays, the
      middle turns 45°; the modes as for the Bend.
- [x] Taper 100 %: the top shrinks to the axis, the bottom stays; Curvature
      bends the profile; Fillet flattens both ends; negative widens.
- [x] Bulge 100 %: the middle doubles its distance from the axis, the ends stay;
      negative pinches; never inside out.
- [x] Each handle sits where described; dragging it to a point gives the
      strength that puts the handle there; Shift steps; one undo step; Esc
      restores.
- [x] Saved and loaded; animated; rendered.
