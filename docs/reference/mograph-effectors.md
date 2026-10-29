# Effectors: Plain and Random

> Sources (read 2026-10-30): manual — the Plain effector's Effector and
> Parameter properties (Strength; Position, Scale with Uniform Scale, Rotation;
> Color Mode) and the Random effector's description (random values per clone
> from a seed); the older falloff options (shape, size, softness). Written in
> our own words — no copied text or images. Defaults the manual leaves open are
> ours and marked.
>
> Covers implementation-plan step **10.8**. The Cloner is described in
> [`mograph-cloner.md`](mograph-cloner.md).

## Purpose

Change a Cloner's clones without touching the Cloner: lift some, scale them,
turn them, colour them — everywhere (Infinite) or only near the effector, with
a soft edge. The **Plain** effector applies its values as they are; the
**Random** effector applies a different random part of them to each clone.

## Parameters

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Strength | % (negative allowed) | 100 % | Scales everything the effector does (animatable) |
| Seed | 0 – 9 999 999 | 12 345 (ours) | Random only: another seed, another set of random values |
| Position + P . X / Y / Z | on / off, cm | Plain: on, (0, 100, 0); Random: on, ± (50, 50, 50) (ours) | Moves each clone along its own axes |
| Scale + Uniform Scale + S | on / off, % | off | Changes each clone's size: +50 % makes it 1.5 times as large, −50 % half (Uniform: one value for all axes) |
| Rotation + R . H / P / B | on / off, degrees | off | Turns each clone |
| Color + Effector Color | on / off, colour | off | Plain: blends the clones towards the colour; Random: towards a random colour per clone |
| Falloff | Infinite, Sphere, Box, Linear | Infinite | Where the effector works (in its own space) |
| Size . X / Y / Z | cm | 200 cm | The falloff's extent (animatable; not for Infinite) |
| Soft Edge | 0 – 100 % | 50 % | How much of the way in from the edge fades (Sphere, Box) |

## Behaviour

- **Linking:** an effector works on the Cloners whose **Effectors** list holds
  it. Creating an effector while Cloners are selected adds it to their lists
  (one undo step); the list in the Cloner's Object tab adds other effectors and
  removes linked ones. Effectors apply in the list's order, each to the result
  of the one before. A deleted effector simply drops out.
- **Falloff:** each clone is weighted by where its placement lies in the
  effector's space: Infinite — fully everywhere; Sphere / Box — fully inside
  the inner part, fading smoothly to nothing at the edge across the soft part;
  Linear — nothing on the −Z side of the size, fully on its +Z side, smoothly
  between. The weight times Strength is the **amount**.
- **Transform:** a clone's placement is multiplied (in its own space) by a move
  of P × amount, a turn of R × amount and a scale of 1 + S × amount. Random
  multiplies each value by its own random factor in −1 … 1, fixed per clone
  (the clone's number and the seed), so the result never flickers.
- **Colour:** the clone's colour moves towards the effector's colour (Random: a
  random colour per clone) by the amount, clamped to 0 … 1.
- **Moving the effector** moves its falloff: clones near it react. Hidden
  effectors still work; switched off (the check mark) they do nothing.
- **Display:** a small cross at the effector's origin and the falloff's
  outline (a sphere's three circles, a box, the Linear's two planes and arrow)
  in its colour, picked like a deformer's box; *Filter ▸ Effector* hides them.
- **Results:** everything that reads clones (the viewport, picking, framing,
  the path tracer, Make Editable, Current State to Object, export) sees them
  changed.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `Effector` (strength, P / S / R with switches, Uniform Scale, colour, seed, `Falloff`), `Falloff::weight`, `Effector::offset(amount, &Randoms)`, `Randoms::of(seed, clone)`, `Effector::lines` | `grid4d-geom::effector` |
| `ObjectKind::{PlainEffector, RandomEffector}` (`ObjectKindTag::EFFECTORS`), animatable Strength (`ObjectParam::Amount`) and falloff Size; `ObjectKind::Cloner { effectors: Vec<NodeId>, .. }` | `scene/node.rs`, `scene/param.rs` |
| Effects applied per clone while instances are built (placement, then colour as `c × keep + tint`), effectors' lines as curves (`CurveSource::Effector`) | `grid4d-eval` |
| `FilterItem::Effector` | `grid4d-render::filter` |
| `.g4d` v3 `plain_effector` / `random_effector`; the Cloner's `effectors` by index; Merge remaps them | `grid4d-io` |
| Create ▸ Effector, toolbar, `create_effector` (one step, added to selected Cloners), Attribute Manager rows, the Cloner's Effectors list | `grid4d-ui` |

## Decisions (2026-10-30)

1. **One falloff per effector, of its own** (plan): the reference's modular
   Fields are later.
2. **Transform Mode is Relative** (added to the clone's own values); Remap,
   Absolute Scale, Min / Max and the Random modes other than plain uniform
   randomness (Gaussian, Noise, Turbulence, Sorted) are later.
3. **Defaults are ours** where the manual states none.
4. **Linear falloff runs along the effector's Z axis.**

## Acceptance criteria

- [x] A Plain effector lifts, scales, turns and colours every clone by its
      values times Strength; a Sphere / Box / Linear falloff limits it, with a
      soft edge.
- [x] A Random effector varies each clone within ± its values, repeatably,
      changing with the seed.
- [x] Effectors apply in list order; switched off they do nothing.
- [x] Creating an effector with a Cloner selected links it in one undo step.
- [x] Saved and loaded with the Cloner's list; drawn, filtered; rendered.
