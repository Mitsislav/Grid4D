# Cloner

> Sources (read 2026-10-29): manual — the Cloner object's description and its
> Object properties (Mode; Clones: Iterate / Random / Blend / Sort; Seed; Reset
> Coordinates; Instance Mode; the Linear, Radial and Grid Array settings).
> Written in our own words — no copied text or images. The manual states few
> defaults; ours are marked.
>
> Covers implementation-plan steps **10.6** (the framework: Linear, Radial,
> Grid Array) and **10.7** (Object mode, cloned groups, Cloners inside
> Cloners). Effectors follow in 10.8 (`mograph-effectors.md`).

## Purpose

Repeats objects: put one or more objects under a Cloner and it shows copies of
them — the **clones** — in a row, on a circle or in a grid. Change the Cloner's
values (or animate them) and every clone follows; edit a child and every clone
of it changes.

## Parameters

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Mode | Linear, Radial, Grid Array | Linear | The layout; each has its own values below |
| Clones | Iterate, Random | Iterate | Which child each clone repeats: the children in turn, or one picked at random |
| Seed | 0 – 9 999 999 | 1 234 567 (ours) | Random only: another seed, another picking |
| Reset Coordinates | on / off | on | On: each clone sits exactly where the layout puts it (the child's own position, rotation and scale are ignored). Off: the child's placement under the Cloner is added on top |

**Linear** — clones in a row, starting at the Cloner.

| Name | Range | Default (ours) | Effect |
|------|-------|---------|--------|
| Count | 0 – 10 000 | 3 | Number of clones |
| Offset | 0 – 10 000 | 0 | Steps skipped at the start: the row begins further along |
| Spacing | Per Step, End Point | Per Step | Per Step: the values below are the change from one clone to the next. End Point: they are the change from the first clone to the last |
| P . X / Y / Z | cm | 0, 250, 0 | Position change |
| S . X / Y / Z | % | 100 % | Scale change (per step it multiplies: 50 % halves each clone again) |
| R . H / P / B | degrees | 0° | Rotation change |

**Radial** — clones on a circle around the Cloner.

| Name | Range | Default (ours) | Effect |
|------|-------|---------|--------|
| Count | 0 – 10 000 | 6 | Number of clones |
| Radius | ≥ 0 cm | 400 cm | The circle's radius (animatable) |
| Plane | XY, ZY, XZ | XZ (on the floor) | The circle's plane, in the Cloner's space |
| Align | on / off | on | Each clone turned about the circle's axis by its angle, so all face the same way relative to the centre |
| Start / End Angle | degrees | 0° / 360° | The part of the circle used (animatable). A full turn spaces the clones evenly; a part puts the last clone on the end |
| Offset | degrees | 0° | Turns every clone along the circle |

**Grid Array** — clones in a box-shaped grid, centred on the Cloner.

| Name | Range | Default (ours) | Effect |
|------|-------|---------|--------|
| Count . X / Y / Z | 1 – 10 000 each, 10 000 clones in all | 3 × 3 × 3 | Clones along each axis |
| Spacing | Per Step, End Point | Per Step | Per Step: Size is the distance between neighbours. End Point: Size is the whole grid's extent |
| Size . X / Y / Z | cm | 250 cm | As Spacing says (animatable) |

**Object** (10.7) — clones on another object's surface.

| Name | Range | Default | Effect |
|------|-------|---------|--------|
| Object | a link to any object | none (no clones) | The object whose surface carries the clones |
| Distribution | Vertex, Polygon Center | Vertex | A clone on every point, or on every polygon's centre |
| Align Clone | on / off | on | Each clone's Y axis along the surface's normal there (a point's normal: the mean of its polygons', weighted by area). Off: clones keep the Cloner's orientation |

## Behaviour

- **Children are the sources.** Every visible child of an enabled Cloner is
  repeated (deformers among them are not cloned, nor are cameras and lights).
  The children themselves are not drawn: only the clones. They stay in the
  Object Manager, where they are selected and edited as usual.
- **Groups (10.7):** a child with children of its own is cloned whole: each
  clone repeats the child and everything below it, keeping their placement
  relative to the child (Reset Coordinates resets only the child's).
- **Cloners inside Cloners (10.7):** a Cloner among the children (or inside a
  cloned group) is cloned with its clones — a Linear Cloner of three inside a
  Radial one of ten gives thirty. The inner Cloner's clones belong to the
  outer one (clicking one selects the outer Cloner).
- **Object mode (10.7):** the linked object's shape as the viewport shows it
  (a polygon object, a primitive, a generator's result, a deformed shape) at
  the current frame, wherever it is: the clones follow it when it moves or
  changes, also while it is hidden (a hidden polygon object or primitive still
  carries clones; a hidden generator does not). The object may be anywhere in
  the hierarchy; without one, or with a deleted one, there are no clones.
- **Clones are not objects.** They are made each time the scene is evaluated,
  so a thousand clones cost no more memory than their shapes, and identical
  clones are drawn together. They take their child's shape as shown (a
  deformed child gives deformed clones, a generator its result), colour and
  material.
- **Selecting:** clicking a clone, or framing the Cloner with a rectangle,
  selects the Cloner. The selected Cloner's clones are highlighted and its
  selection box surrounds them all; framing (H / O) frames them.
- **Disabled** (the check mark, `Q`), the Cloner is a plain group: its
  children show as they are. Hidden, everything goes.
- **Alt + create** puts the selected object under a new Cloner, as for the
  generators.
- **Make Editable** (C) turns the Cloner into a Null holding one polygon object
  per clone (named after its child: `Cube.0`, `Cube.1`, …); the children are
  removed. **Current State to Object** adds the same as a copy.
- **Rendering and export:** the path tracer renders the clones; glTF and OBJ
  export write the Cloner as a group of its clones.
- **Animation:** Radius, Start / End Angle (Radial) and Size (Grid Array) are
  animatable like other parameters; counts and choices are not (as everywhere).

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `Cloner` (layout, clones, seed, Reset Coordinates), `Layout::{Linear, Radial, Grid}`, `StepMode`, `CloneOrder`; `count`, `matrix(index)`, `source(index, children)` | `grid4d-geom::cloner` |
| `ObjectKind::Cloner`, in `GENERATORS`, `generator_inputs` = every child; animatable parameters by layout | `scene/node.rs`, `scene/param.rs` |
| `EvaluatedScene::instances()` → `EvaluatedInstance { owner, source, kind, world, bounds, drawable, pick }`, built after animation and Target tags; the Cloner's bounds are its clones'; `PickShape::Clones` | `grid4d-eval` |
| Instances drawn with the objects (same batches), picked (→ the Cloner), filtered as Generators | `grid4d-render` (`frame.rs`, `picking.rs`, `filter.rs`) |
| Instances baked into the render scene | `grid4d-raytrace` |
| Object mode: `Layout::Object(Surface)` (`Distribution`, align), `Cloner::placements(surface, …)`; `ObjectKind::Cloner { cloner, object }`; the object's mesh from the evaluation (or, hidden, its own polygons / a cached primitive mesh) | `grid4d-geom::cloner`, `grid4d-eval` |
| Groups and nested Cloners: every object inside an enabled Cloner is a source (`top` for its children); Cloners built deepest first, an inner Cloner's instances carried into the outer one's and then dropped | `grid4d-eval` |
| The object link saved by index (`"object"`), resolved after every object exists; Merge remaps it (`remap_objects`) | `grid4d-io` |
| `.g4d` v3 `cloner` kind (layout by mode, defaults omitted, counts clamped) | `grid4d-io::v3` |
| Make Editable, Current State to Object, export: `current_state::clone_objects` | `grid4d-ui` |
| Icon, Create ▸ Generator, toolbar, Attribute Manager rows | `grid4d-ui` |

## Decisions (2026-10-29)

1. **Clones are instances, not nodes** (plan): the list is rebuilt with the
   evaluation, reusing its buffer; nothing per frame allocates.
2. **Reset Coordinates ignores the child's whole placement** (position,
   rotation and scale), as the manual's "the first clone takes the Cloner's
   matrix" reads.
3. **Per Step scale multiplies** from clone to clone; End Point scale blends
   from 100 % to the value.
4. **Defaults are ours** where the manual states none (250 cm steps for the
   200 cm default cube; a 400 cm ring on the floor).
5. **Limits:** at most 10 000 clones per Cloner.
6. **Align Clone turns the clone's Y axis** along the normal (ours: a cone
   stands on the surface like a spike), from the clone's unturned orientation.
7. **Later:** Honeycomb, Blend and Sort, Step Mode / Step Size / Step Rotation,
   Amount, Offset Variation, Instance Mode choices, grid forms and Fill.

## Acceptance criteria

10.6:

- [x] Linear, Radial and Grid Array place clones as described (per step and end
      point); Iterate alternates children, Random follows the seed.
- [x] The children are hidden, the clones drawn instanced (a 10 × 10 × 10 grid
      is one draw), selected, filtered and picked as the Cloner.
- [x] Reset Coordinates off keeps the child's placement; a disabled Cloner is a
      group.
- [x] Saved and loaded; counts in files clamped.
- [x] Make Editable, Current State to Object and export give one object per
      clone; the path tracer renders them.
- [x] The instance list reuses its capacity (hot-path test).

10.7:

- [x] Object mode puts a clone on every point or polygon centre of a polygon
      object, primitive or generator result, aligned to the normals or not;
      a hidden object still carries them; no object, no clones.
- [x] A child group is cloned whole; a Cloner inside a Cloner is cloned with
      its clones, all selected as the outer Cloner.
- [x] The object link is saved, loaded and remapped by Merge.
