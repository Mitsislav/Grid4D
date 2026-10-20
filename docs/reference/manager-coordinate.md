# The Coordinate Manager

> Sources (read 2026-10-20): Cinema 4D manual —
> [Coordinate Manager](https://help.maxon.net/c4d/en-us/Content/html/5742.html)
> (edits the selected element numerically: position, size and rotation; the left
> list chooses how values are read — *Object (Rel)*, *Object (Abs)* with frozen
> coordinates, or *World*; the right list chooses the middle column — *Scale*, the
> axis lengths, 1 / 1 / 1 by default; *Size*, the object's extent without its
> children; *Size+*, with them; a size is measured along the object's own axes;
> points have no coordinate system of their own, so their rotation is entered as a
> relative value) and the
> [keyboard shortcuts](https://help.maxon.net/c4d/en-us/Content/html/Shortcuts.html)
> (Shift+F5 Attribute Manager, Shift+F7 Coordinate Manager). Written in our own
> words.
>
> Covers implementation-plan step **6.5**.

## Purpose

Typing exact positions, sizes and rotations: of the active object, or of the
selected points, edges or polygons in a component mode, in the object's own space
or in world space.

## Layout

A tab **Coordinates** beside **Attributes** (Shift+F7 shows it, Shift+F5 the
Attribute Manager):

| | Position | Scale / Size / Size+ | Rotation |
|-|----------|----------------------|----------|
| 1 | X | X | H |
| 2 | Y | Y | P |
| 3 | Z | Z | B |

Under the fields: the coordinate system (**Object** or **World**), the middle
column's meaning (**Scale**, **Size** or **Size+**) and **Apply**.

## Behaviour

### Objects (Model mode; the active object)

| Column | Object | World |
|--------|--------|-------|
| Position | the object's position in its parent (cm), as in the Coord. tab | the object's position in the world |
| Scale | its scale | its scale in the world (the parent's included) |
| Size | its extent along its own axes, scale included (cm) | the same |
| Size+ | like Size, its children included | the same |
| Rotation | H / P / B in its parent (degrees) | H / P / B in the world |

- Values are those **at the current frame**; a change goes through the same rules as
  the Attribute Manager (temporary value, or a key with Autokeying; see
  `animation-recording.md`).
- A new **Size** is reached by changing the object's scale; an object without
  extent (a Null) has no size.
- Channels guarded by a Protection tag are read-only.
- A drag or a typed value changes the object at once; it is one undo step.
  **Apply** writes all nine fields as shown (one undo step).

### Components (Points / Edges / Polygons mode; the active polygon object)

- **Position**: the centre of the bounding box of the selected elements' points,
  in object or world space. A new value moves them.
- **Size**: the extent of that box (along the object's axes, or the world's in
  World mode). A new value scales the points around the centre along that axis.
  The middle column always shows Size here.
- **Rotation**: always 0; a value rotates the points around the centre by that
  much (relative), and the field returns to 0 afterwards.
- One undo step per drag or typed value; nothing selected: the fields are empty.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| Panel | `ui/src/panels/coordinates.rs`; the tab in `EditorState::managers` (`ManagerTab`) |
| Reading and applying values | the panel, through `NodeData::transform_at` / `set_transform_at`, `Scene::world_matrix_at`, `MeshHandle::with_points` |
| Undo | live writes, then `EditorCommand::CommitLive` (`SetData`) per interaction; Apply is an `EditorCommand::Edit` |

Deliberate differences:

- Grid4D has no frozen coordinates, so *Object (Rel)* and *Object (Abs)* are the
  same: one **Object** entry.
- A new size changes the object's scale, also for primitives (the reference may
  change their size parameters instead).
- World scale and size ignore shear from non-uniformly scaled, rotated parents.
- Several selected objects: only the active one is shown and changed (as in the
  Attribute Manager).

## Acceptance criteria

- [x] Object and World positions and rotations round-trip: entering the value shown
      changes nothing; a new world value puts the object there whatever its parent.
- [x] Scale, Size and Size+ show the right values; a new size scales the object to
      it.
- [x] Component mode: position moves, size scales and rotation rotates the selected
      points around their centre; rotation returns to 0.
- [x] Animated objects: the values at the current frame; Autokeying writes keys.
- [x] One undo step per interaction; Apply is one step; idle frames write nothing.
- [x] Shift+F7 / Shift+F5 switch between the Coordinates and Attributes tabs.
