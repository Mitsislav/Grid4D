# Current State to Object

> Sources (read 2026-10-29): manual — the Mesh ▸ Conversion menu (Make
> Editable, Current State to Object) and the descriptions of how generators and
> deformers are baked. Written in our own words — no copied text or images.
>
> Covers implementation-plan step **10.5**. Make Editable is described in
> [`modeling-modes.md`](modeling-modes.md); deformers in
> [`deformer-basics.md`](deformer-basics.md).

## Purpose

Freezes what the viewport shows into plain objects: bent, twisted or
lattice-deformed shapes, the result of generators (Subdivision Surface,
Extrude, Lathe, Sweep, Loft) and animation at the current frame all become
ordinary polygon objects you can model further, while the original stays as it
was and keeps working.

It differs from **Make Editable**, which converts the object *itself* (a
primitive into its own polygons, a generator into its result) in place. Current
State to Object leaves the selection untouched and adds a converted *copy*.

## Behaviour

- **Mesh ▸ Conversion ▸ Current State to Object**, enabled when a selected
  object is visible in the viewport. There is no shortcut (the reference has
  none by default).
- Each selected object (an object below another selected one goes with it) is
  copied **with its hierarchy**, as the viewport shows it at the current frame:
  - an object deformed by deformers → a polygon object of the deformed shape;
  - an enabled generator → a polygon object of its result (viewport detail,
    deformed if deformers reach it); the objects it consumes (the Subdivision
    Surface's cage, the Extrude's or Lathe's spline, the Sweep's profile and
    path, the Loft's sections) are not copied;
  - a primitive → a polygon object of its shape;
  - a polygon object → a copy (deformed if deformers reach it); its selections
    and hidden elements are kept when the shape keeps its polygons;
  - a spline primitive or a Text → an editable spline (splines are not
    deformed);
  - a disabled generator → a Null holding its children's copies;
  - Nulls, cameras, lights and editable splines → copies;
  - deformers → not copied (their effect is in the shapes).
- Objects hidden in the viewport (and their children) are not copied.
- Animated parameters and transforms are frozen at the current frame: the
  copies have no animation tracks.
- The copies keep their originals' names, colours, visibility and tags
  (Material, Phong, Protection, Target); a generator's copy takes its cage's
  material and Phong tag when it has none of its own, as the viewport shows.
- Each copy is inserted right after its original, under the same parent; the
  copies become the selection. One undo step.

## Export

glTF and OBJ export bake deformers the same way: an object reached by deformers
is written in its deformed shape at the current frame.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `EvaluatedScene::deformed(id)` — a node's deformed mesh after the last update | `grid4d-eval` |
| `current_state::bake(doc, selection)` — the copies as `NodeData` with parent indices, from an `EvaluatedScene` at the current frame | `grid4d-ui::current_state` |
| `EditorCommand::CurrentStateToObject`: inserts them as one undo step, selects the copies | `grid4d-ui::commands` |
| Mesh ▸ Conversion entry | `grid4d-ui::panels::menu_bar` |
| Export of deformed shapes (`file::exportable`) | `grid4d-ui::file` |

## Decisions (2026-10-29)

1. **The copy follows the viewport** (visibility, viewport detail of
   generators), since the command is about what you see; the final render's
   detail is not used.
2. **Names are kept** (as the reference does), not made unique.
3. **Clones** (10.6) will be baked here too, as one polygon object per clone
   under a Null.

## Acceptance criteria

- [x] A cube bent by a Bend becomes a new polygon object with the bent points;
      the cube and the Bend are unchanged; one undo step removes the copy.
- [x] A Subdivision Surface with a cage becomes one polygon object of its result;
      an Extrude does not copy its spline.
- [x] Animated objects are copied as they are at the current frame, with no
      tracks.
- [x] Hierarchies are copied with their structure; hidden objects and
      deformers are left out; copies sit after their originals and are
      selected.
- [x] Export writes deformed shapes.
