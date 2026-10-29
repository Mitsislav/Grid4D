# Bend deformer

> Sources (read 2026-10-29): manual —
> [Bend deformer](https://help.maxon.net/c4d/en-us/Content/html/OBEND.html),
> [its Object properties](https://help.maxon.net/c4d/en-us/Content/html/OBEND-ID_OBJECTPROPERTIES.html).
> The bending itself is the classic circular bend of a column (A. Barr,
> *Global and local deformations of solid primitives*, 1984 — public).
> Written in our own words — no copied text or images.
>
> Covers implementation-plan step **10.2**; the framework is in
> [`deformer-basics.md`](deformer-basics.md).

## Purpose

Bends objects along the deformer's Y axis: a tube into an arch or a ring, a
book's page, a bending finger, a growing stem when Strength is animated.

## Parameters

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Size | cm × 3 | 250 × 250 × 250 | The box (see the basics); its height (Y) is the part that bends |
| Mode | Limited · Within Box · Unlimited | Limited | See the basics |
| Strength | degrees, any | 0° | How far the box's height bends round: 90° a quarter circle, 360° a full ring; negative bends the other way |
| Angle | degrees, any | 0° | The direction of the bend around Y: 0° towards the deformer's +X |
| Keep Y-Axis Length | bool | off | On: the bent part keeps its length (it bends on a circle as long as the box is high). Off: the bent part's ends stay the box's height apart, so it stretches as it bends |
| Origin | Bottom · Center C-Shape · Center S-Shape · Top | Bottom | Where the bend starts: the box's bottom (the top bends over), its top (the bottom bends), or its centre — both halves curling the same way (a C) or opposite ways (an S) |
| Fit to Parent | button | — | See the basics |

**Strength in degrees.** The reference shows Strength as a percentage; Grid4D
states it as the angle the bent part turns through, which says directly what
you get. (Animatable: Strength, Angle and Size.)

## Behaviour

- In the deformer's space (rotated about Y by **Angle** so the bend goes
  towards +X), a point at height `s` into the bent part turns through
  `Strength × s / height` on a circle; points to the side of the Y axis move
  with it at their distance (the inside of the bend is compressed, the outside
  stretched). Points before the bent part stay; points after it continue
  straight along the bend's end (Limited), are bent on (Unlimited) or stay
  unless they are inside the box (Within Box).
- Strength 0 changes nothing. The box itself is drawn bent.

## Grid4D mapping

`grid4d_geom::deform::Bend` (`size`, `mode`, `strength` in radians, `angle` in
radians, `keep_length`, `origin`: `BendOrigin`), `Bend::apply(point)`,
`Bend::box_lines()`; `ObjectKind::Bend(Bend)`; animatable through
`ObjectParam::{Size, Strength, Angle}`.

## Acceptance criteria

- [x] Strength 90° bends a column of the box's height into a quarter circle
      with Keep Y-Axis Length (its centre line keeps its length; its end
      points along +X); off, its end stays the box's height from its start.
- [x] Angle 90° bends towards −Z instead; a negative strength towards −X.
- [x] Limited: a part beyond the box continues straight; Within Box: it stays;
      Unlimited: it bends on.
- [x] Origins: Top keeps the top still, Center C bends both halves to +X,
      Center S the lower half to −X.
- [x] Strength 0 is the identity.
