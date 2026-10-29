# Deformers: the basics

> Sources (read 2026-10-29): manual —
> [Bend deformer](https://help.maxon.net/c4d/en-us/Content/html/OBEND.html)
> (how a deformer is attached; a Null groups several objects for one deformer),
> [its Object properties](https://help.maxon.net/c4d/en-us/Content/html/OBEND-ID_OBJECTPROPERTIES.html)
> (Size, Mode, Fit to Parent). Written in our own words — no copied text or
> images.
>
> Covers implementation-plan step **10.2** (with [`deformer-bend.md`](deformer-bend.md)).

## Purpose

A deformer changes the shape of other objects without touching their points:
bend a cylinder into an arch, twist a column, squash a sphere — and keep
editing the original, or animate the deformer.

## Who is deformed

- A deformer deforms its **parent** and **everything below the parent** that
  has a surface: polygon objects, primitives, generator results. To deform
  several objects together, put them and the deformer under one Null.
- Several deformers apply in turn: those under the object itself first, then
  those of each ancestor outwards; among siblings, in Object Manager order.
- **Generators:** a deformer inside a generator's input (under the cage of a
  Subdivision Surface) deforms that input before the generator uses it; a
  deformer under the generator itself, beside its inputs, deforms the
  generator's result. Splines are not deformed (yet).
- A **switched-off** deformer (its check mark, or Q's sibling: the Object
  Manager's tick) does nothing. A deformer hidden in the editor still deforms
  (visibility hides its box, not its effect).
- Deforming never changes the deformed object's own data: Make Editable keeps
  the undeformed shape (Current State to Object, 10.5, bakes the deformed one).

## The box

- Every deformer has a **box** in its own space, centred on its origin, of
  **Size** X × Y × Z (250 cm each by default). The deformer's own Y axis is the
  one it works along; move, rotate and scale the deformer to place its effect.
- **Mode** decides what happens outside the box:
  - **Limited** (default): the part inside the box is deformed; the parts
    beyond it follow along (they move with the deformed part's end, unchanged
    in shape).
  - **Within Box**: only points inside the box are deformed; everything else
    stays where it is.
  - **Unlimited**: the whole object is deformed as if the box had no end.
- **Fit to Parent** sizes the box to the parent's extent and places the
  deformer at the parent's centre, unrotated (one undo step).
- The box is drawn as lines in the deformer's display colour — itself deformed,
  so the effect shows even on an empty deformer — and a click near its lines
  selects the deformer. It is never rendered.

## Creating

- **Create ▸ Deformer ▸ …** and the toolbar's deformer buttons add a deformer
  at the top of the Object Manager. With **Shift**, the deformer is added
  under the selected object instead and fitted to it — the usual way to bend
  one object.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `DeformerMode`, the box maths, each deformer's pure point function (`Bend::apply`) and its drawn box (`box_lines`) | `grid4d-geom::deform` |
| `ObjectKind::Bend(Bend)`; `ObjectKindTag::DEFORMERS`; `ObjectKind::is_deformer` | `scene/node.rs` |
| Deformer chains while walking the scene (a stack of active deformers by depth, deepest first), each object's link matrices into every deformer's space at the frame; the `Deformed` cache per node (source: mesh id or primitive parameters; links); generator inputs with their inner chains | `grid4d-eval` (`deform.rs`) |
| The box as a curve (`CurveSource::Deformer`), picked like a spline; `FilterItem::Deformer` | eval, render |
| `EditorCommand::FitToParent`, `EditorCommand::CreateChild` (Shift + create) | `ui/src/commands.rs` |
| Create ▸ Deformer, toolbar group, icons, Attribute Manager rows (Fit to Parent button) | `grid4d-ui` |
| `.g4d` v3 kinds (additive) | `grid4d-io::v3` |

## Decisions (2026-10-29)

1. **Visibility does not switch a deformer off**; only its check mark does.
2. **Shift + create** places a deformer under the selection and fits it,
   following the reference's modifier for deformers.
3. **Component modes** keep showing and editing the undeformed points (the
   overlay sits on the original mesh); showing them deformed is later.

## Acceptance criteria

- [x] A deformer under a cube deforms the cube; under a Null, every object in
      the Null; switched off, nothing.
- [x] Two deformers apply in order (inner first); moving a deformer changes
      the result; idle frames and unrelated edits rebuild nothing.
- [x] A Bend under a Subdivision Surface's cage deforms the cage before
      smoothing; a Bend beside the cage deforms the smooth result.
- [x] The deformed shape is drawn, picked, framed and rendered; the object's
      own mesh is unchanged; animation of the deformer's parameters and
      placement deforms per frame.
- [x] Fit to Parent and Shift + create are one undo step each; saved and
      loaded.
