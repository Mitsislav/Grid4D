# Layers and the Layer Manager

> Sources (read 2026-10-30): manual — the Layer Manager (layers as named,
> coloured groups of objects with switches: solo, visible in the editor,
> visible in the renderer, shown in the Object Manager, locked, animation,
> generators, deformers; assigning objects to a layer, selecting a layer's
> objects). Written in our own words — no copied text or images.
>
> Covers implementation-plan step **12.4**.

## Purpose

Sort a scene into groups that can be hidden, locked or switched off together —
all the lights, the background set, the rough proxies — without touching each
object or the hierarchy.

## Behaviour

- **Layers** belong to the document, in the Layer Manager's order. Each has a
  **name**, a **colour** and eight **switches**, all on except Solo and Locked:
  - **S — Solo**: while any layer is soloed, only objects on soloed layers
    show in the views and renders;
  - **V — View**: off hides the layer's objects in the views;
  - **R — Render**: off leaves them out of renders;
  - **M — Manager**: off hides them in the Object Manager;
  - **L — Locked**: on, they cannot be picked or framed by a selection in the
    views, and the gizmo and the Coordinate Manager do not move them;
  - **A — Animation**: off shows them unanimated (as if at no frame);
  - **G — Generators**: off makes generators on the layer act as switched off
    (plain groups);
  - **D — Deformers**: off makes deformers on the layer do nothing.
- **An object's layer.** An object is on at most one layer. An object without
  one is on its parent's layer (and so on up), so putting a group on a layer
  takes its contents along unless they have a layer of their own. An object
  that is hidden hides its children, as with the traffic lights.
- **The Layer Manager** (the Layers pane) lists the layers: a colour chip, the
  name and the eight switch columns. In it:
  - **New Layer** adds one (a new colour each time);
  - double-click a name to rename it; click the chip to recolour it;
  - click a switch to toggle it (one undo step);
  - **Add Selected Objects** puts the selected objects on the layer,
    **Remove Selected Objects** takes them off it;
  - **Select Objects** selects every object whose own layer it is;
  - **Delete Layer** removes it; its objects are on no layer of their own any
    more;
  - objects dragged from the Object Manager onto a layer go on it.
- **The Object Manager** shows an object's own layer as a chip of the layer's
  colour beside its name (hover: the layer's name). Objects on a layer with
  Manager off are not listed (nor their children).
- **The Attribute Manager's Basic tab** has a **Layer** field to pick an
  object's layer, or none.
- Every change — layers, switches, assignments — is one undo step, and saved
  with the document. **Merge** brings the merged file's layers along (a new
  copy of each), as it does materials.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `Layer` (name, colour, `LayerSwitches`), `LayerId`, `Layers` (ordered table; `shows`, `solo_active`) | `grid4d-scene::layer` |
| `NodeData::layer`; `Scene::{layers, layers_mut, effective_layer, animates, is_locked}`; `Scene::world_matrix_at` honours Animation | `grid4d-scene` |
| `SceneEdit::SetLayers` (the whole table, ids kept) | `grid4d-scene::edit` |
| Evaluation: View / Render / Solo while walking (a layer stack, as for materials), Animation in `animate` and parameters, Generators, Deformers; Locked objects get no pick shape | `grid4d-eval` |
| `.g4d` v3: `layers` (omitted when empty) and each object's `layer` index | `grid4d-io::v3` |
| `Objects::layers`; Merge remaps layer ids | `grid4d-io::import`, `grid4d-ui::file` |
| The Layer Manager pane; the Object Manager's chips and Manager switch; the Basic tab's Layer field; `EditorCommand::{AssignLayer, SelectLayerObjects}` | `grid4d-ui` |

## Decisions (2026-10-30)

1. **Layers are document data** like materials: an ordered table with stable
   ids, replaced whole by one edit; a link to a deleted layer counts as none.
2. **Inheritance by the hierarchy**: the effective layer is the object's own,
   else its nearest ancestor's — the same rule as materials.
3. **The traffic lights and the layer both hide**: an object shows only if
   its own switches and its layer's allow it.
4. **Later:** layer hierarchies (folders), the Expressions and XRef switches,
   layer colours shown in the views, per-layer render passes.

## Acceptance criteria

- [x] Layers can be created, renamed, recoloured, reordered by creation,
      deleted, and their switches toggled, each one undo step.
- [x] View, Render and Solo hide objects in the views and renders; Locked
      objects cannot be picked or moved; Animation, Generators and Deformers
      switch those off.
- [x] Children follow their parent's layer unless they have their own.
- [x] The Object Manager shows layer chips and hides Manager-off objects.
- [x] Layers and assignments are saved and come back; Merge carries them.
