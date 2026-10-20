# Tags: Phong, Protection, Target

> Sources (read 2026-10-20): Cinema 4D manual —
> [Phong tag properties](https://help.maxon.net/c4d/r23/en-us/Content/html/TPHONG-ID_TAGPROPERTIES.html)
> (Angle Limit, Phong Angle 0…180°, 180° meaning no limit, Use Edge Breaks;
> smoothing only spreads across connected polygons),
> [Protection tag](https://help.maxon.net/c4d/2026/en-us/Content/html/TPROTECTION-ID_TAGPROPERTIES.html)
> (position, scale and rotation each None, Lock or Limit; guards against
> accidental changes in the viewport),
> [Target tag](https://help.maxon.net/c4d/r21/us/html/TTARGETEXPRESSION-ID_TAGPROPERTIES.html)
> (aims the object's Z axis at a Target Object; an Up Vector object for the Y axis;
> a Pitch option). Written in our own words.
>
> Covers implementation-plan step **6.1** (the tag system and its first three tags;
> the Material tag follows with materials in 6.6).

## Purpose

Tags add properties or behaviour to an object without changing what kind of object
it is. They appear as small icons to the right of the object in the Object
Manager; selecting one shows its settings in the Attribute Manager.

## Parameters

| Tag | Name | Type / range | Default | Effect |
|-----|------|--------------|---------|--------|
| Phong | Angle Limit | bool | on | When on, edges sharper than the Phong Angle stay hard; when off, everything connected is smoothed (= 180°) |
| | Phong Angle | 0 … 180° | 80° | The angle up to which neighbouring polygons are shaded smooth (0° = flat) |
| Protection | Position / Scale / Rotation | bool each | all on | A protected channel cannot be changed in the viewport or in the Coordinates fields |
| Target | Target Object | an object or none | none | The object the tagged one turns to face |
| | Up Vector | an object or none | none | The tagged object's up axis turns towards it (world up without one) |
| | Pitch | bool | on | Off: the object only turns around the vertical axis (stays level) |

The reference's Protection tag also offers *Limit* ranges per axis, and the Phong
tag *Use Edge Breaks*; both are *later*. Defaults the manual does not state (the
Phong angle, the Protection channels) are ours.

## Behaviour

- **Showing and selecting:** each tag is an icon after the object's name in the
  Object Manager. Clicking it selects the tag and shows it in the Attribute Manager
  (Mode ▸ Tags); clicking an object again shows the object.
- **Adding:** Object Manager ▸ Tags (or the object's context menu) adds the tag to
  every selected object, as one undo step. An object holds one tag of each kind
  (adding a second Phong tag selects the existing one).
- **Removing:** with a tag selected, Delete removes it (one undo step).
- **Editing:** tag settings are edited like any attribute; each change is one undo
  step and is saved with the document.
- **Phong:** sets how the object's mesh is smoothed in the viewport and in exports
  (glTF normals, OBJ `vn`). Objects **without** a Phong tag keep Grid4D's default
  smoothing (80°) — a deliberate difference: in the reference, objects without one
  are shaded flat, and its primitives come with one.
- **Protection:** while a channel is protected, the gizmo of the matching tool
  (Move, Scale, Rotate) does not appear for the object and the matching
  Coordinates fields are read-only. Components can still be edited.
- **Target:** while the target exists and is visible, the object is turned so that
  its **forward axis** faces the target — −Z, the direction Grid4D's cameras look
  (the reference aims +Z, its cameras' viewing direction). Children follow. The
  object's own rotation values stay as they are (the tag overrides them in the
  view); a target that is the object itself or inside it is ignored.
- **Files:** tags are saved in `.g4d` (version 3 grows additively); references to
  objects are stored by object index and dropped if the object is not saved.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `Tag` (`Phong { angle_limit, angle }`, `Protection { position, scale, rotation }`, `Target { target, up, pitch }`), `TagKind`, `NodeData::tags` + helpers (`smoothing_angle`, `protection`, `target`) | `grid4d-scene` (`tag.rs`); edits go through `SceneEdit::SetData` (undo for free) |
| Phong in the viewport | `grid4d-eval` `Drawable::shading`; `grid4d-render` keys GPU meshes by (`MeshKey`, shading) |
| Target | `grid4d-eval` (`EvaluatedScene::update`: world matrices overridden, descendants follow) |
| Protection | `viewport/gizmo.rs` (no gizmo for a protected channel), Attribute Manager Coordinates |
| Tag selection, `EditorCommand::AddTag` / delete | `EditorState::tag_selection`, `commands.rs` |
| Icons and clicks in the Object Manager, the tag view in the Attribute Manager | `panels/outliner.rs`, `panels/attributes.rs`, `icons.rs` |
| File format | `.g4d` v3 `TagDto` (optional `tags` per object) |
| Exports | Phong angle used for glTF normals and OBJ `vn` |

## Acceptance criteria

- [x] Adding, editing and deleting tags are undo steps; tags survive save and reload
      (references included) and the version-3 golden file still loads.
- [x] A Phong tag changes the viewport's shading of exactly that object (flat at 0°,
      fully smooth with Angle Limit off) and the normals of its exports.
- [x] A Protection tag hides the gizmo of the protected channel and makes the
      matching Coordinates fields read-only.
- [x] A Target tag turns the object's −Z axis towards the target (children follow,
      Pitch off keeps it level, a missing or self target does nothing).
- [x] Clicking a tag icon selects the tag and shows its settings; Delete removes it.

Known gap: the gizmo of a Target-turned object still shows its unturned axes (the
gizmo reads the document, the turn exists only in the evaluation).
