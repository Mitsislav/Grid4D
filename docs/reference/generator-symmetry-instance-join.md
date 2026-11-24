# Symmetry, Instance and Join generators

> Sources (read 2026-11-09, Cinema 4D 2027.0): manual — the Instance
> object's [Object properties](https://help.maxon.net/c4d/2027/en-us/Content/html/OINSTANCE-ID_OBJECTPROPERTIES.html)
> (the reference link, its modes, swapping with the reference) and the
> Connect object's page (children welded into one mesh within a tolerance).
> The Symmetry object's page was not found under its expected address; its
> behaviour here (a mirror plane, welding at it, points near it held on it)
> is the long-standing common one and is ours to decide. Written in our own
> words — no copied text or images.
>
> Covers implementation-plan step **17.4**. Decision D40. Names (R8):
> *Symmetry* and *Instance* are plain words; the reference's *Connect* is
> called **Join** here.

## Purpose

Three small live objects that make modelling faster without destroying
anything: **Symmetry** models half and shows the whole, **Instance** shows
another object again without copying it, **Join** turns a group into one
mesh so another generator (a Subdivision Surface, a Boolean) can take it as a
single input.

## Symmetry

### Parameters

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Mirror Plane | XY · ZY · XZ (the generator's own axes) | ZY | The plane the children are mirrored across (ZY mirrors left and right). |
| Weld Points | bool | on | Points of the half and its mirror that meet at the plane are made one, so the whole has no seam. |
| Tolerance | length ≥ 0 | 0.01 cm | How near the plane a point must be to be welded (and held on it). |
| Hold Points on Plane | bool | on | Points within the tolerance are moved onto the plane before mirroring, so a slightly-off point never opens a gap or a sliver. |
| Flip | bool | off | Mirrors towards the other side (the result is the same shape; which half is the original changes for component editing and Make Editable's point order). |

### Behaviour

- **Inputs**: every visible child, merged in the generator's space (the
  children's own transforms apply). The half is not clipped: points on the
  "wrong" side are mirrored too, as the user placed them.
- **Result** = the half and its mirror image (faces turned so both face
  outwards), welded at the plane when Weld Points is on. Polygons lying *in*
  the plane would be doubled; they are dropped from the mirror (a face whose
  every point is within the tolerance).
- **Editing**: the children are hidden while the generator is enabled, and in
  component modes the active child stays editable as a cage under the whole;
  every edit shows mirrored the same frame.
- UVs: the mirror reuses the half's UVs with U turned round (U → 1 − U),
  so both halves share the texture and neither shows it mirrored (every
  generator's rule: no face with a mirrored picture).
- Make Editable bakes the welded whole into one polygon object (the half's
  points first, so its selections still name the same points); Current State
  to Object copies it; export bakes it.

## Instance

### Parameters

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Reference Object | a link to any object | none | What is shown again here. Dropped from the Object Manager or picked; menu: Clear, Show in Object Manager. |

### Behaviour

- **Creating**: with an object selected, Create ▸ Generator ▸ Instance (or
  its button, with or without Alt) makes an Instance of it; without a
  selection the Instance shows nothing until a Reference Object is chosen.
- Shows the reference object **and its subtree** as evaluated (deformed,
  animated, a generator's result) at the Instance's own placement: the
  Instance's matrix replaces the reference's world matrix for the copy.
- **No copy is made**: the renderer and the path tracer draw the reference's
  meshes again (the same `MeshKey`, one more instance), picking ray-casts the
  same meshes, so a thousand Instances of a heavy object cost almost nothing.
- Materials: the Instance's own Material tags, if any, cover the copy;
  otherwise the reference's own (and inherited) materials are kept.
- **Cycles are refused**: an Instance of itself or of one of its ancestors,
  or a chain of Instances that comes back, shows nothing (no hang); chains
  are followed up to 16 deep.
- A dangling link (the reference deleted) shows nothing; undo of the delete
  brings it back (the link is a `NodeId`).
- Picking the copy selects the Instance. Make Editable turns it into real
  objects (copies of the reference's evaluated meshes); Current State to
  Object and export do the same.
- Not in 1.3: the reference's *Render Instance* and *Multi-Instance* modes
  (a Cloner already places many copies cheaply), and Swap with Reference.

## Join

### Parameters

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Weld | bool | on | Points of different children that lie within the tolerance become one. |
| Tolerance | length ≥ 0 | 0.1 cm | The welding distance. |

### Behaviour

- **Inputs**: every visible child and its subtree (generators' results
  included), merged in the Join's space into **one** polygon mesh; splines
  and markers give nothing (joining splines is later).
- Each child's UVs are kept; selections are not (the result is new geometry).
- The children are hidden while the Join is enabled. As the input of another
  generator it counts as **one** object: a Subdivision Surface over a Join
  smooths the welded whole (a Subdivision Surface's cage may be a Join,
  Symmetry or Boolean: its result is smoothed); a Boolean takes a Join of
  several cutters as B.
- Make Editable gives one polygon object; Current State to Object copies it;
  export bakes it.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `ObjectKind::{Symmetry(Symmetry), Instance { reference: Option<NodeId> }, Join(Join)}`, tags in `GENERATORS` (Instance too: it builds from a link), labels, icons, Attribute Manager pages | `scene/node.rs`, `ui/icons.rs`, `ui/panels/attributes.rs` |
| Mirroring and welding at a plane; merging meshes with an optional weld | `grid4d-geom` (pure; welding on a grid of cells as `optimize` does, only points near the plane for Symmetry) |
| Inputs, caches, Instance's instances (reusing the reference's drawables) | `grid4d-eval::generator` and the instance list the Cloner already fills (`EvaluatedScene::instances`) |
| Make Editable / Current State / export | `ui/src/current_state.rs`, `file::exportable` |
| File | `.g4d` v6 kinds `symmetry`, `instance` (the link by object index, resolved like a Cloner's), `join` (17.4) |

## Budget (measured 2026-11-09; `modeling_budget` example)

| Case | Target |
|------|--------|
| Symmetry of a half of 100 000 points | ≤ 10 ms a rebuild (by hand, 21 ms): **5.1 ms** (17.4) |
| A thousand Instances of a 10 000-triangle object | the frame stays within the 10 000-object budget (≥ 60 fps); no mesh copied: **0.17 ms** to evaluate the scene after one moves (17.4) |

## Decisions made while implementing (2026-11-24)

1. The mirror's U is turned round (see UVs above).
2. An Instance is listed with the generators (menu, toolbar) but builds
   nothing from children: it adds its shapes to the list the Cloner's clones
   are in, so the viewport, picking and the renderer draw them without a
   copy; filtered with the generators.
3. An Instance inside a Boolean's input gives no surface (its copies are not
   meshes of its own); Make Editable it first.
4. A Subdivision Surface's cage may be a generator of solids (Boolean,
   Symmetry, Join); other generators inside a cage are still left out.

## Acceptance criteria (all met in 17.4)

- [x] Symmetry of a half cube (open at the plane) gives a closed cube: every
      edge between two faces, no point doubled at the plane.
- [x] A point 0.005 cm off the plane is held on it; with Hold off and Weld
      off, the halves stay apart.
- [x] Moving a child point updates both halves the same frame; idle frames
      do not rebuild.
- [x] An Instance draws its reference at its own placement, adds no GPU mesh
      and follows the reference's edits and animation.
- [x] An Instance of its own parent, and two Instances of each other, show
      nothing and never hang.
- [x] Join of two cubes that touch, welded, has the shared points once; a
      Subdivision Surface over it smooths across the join.
- [x] Make Editable on each gives the shown mesh in one undo step; undo
      restores the generator and its children exactly.
- [x] Saved as v6 and reopened the same; a v5 file naming these kinds is
      refused.
