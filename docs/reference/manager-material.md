# Materials, the Material Manager and the Material tag

> Sources (read 2026-10-20, Cinema 4D 2026 manual):
> [Material Manager](https://help.maxon.net/c4d/en-us/Content/html/5808.html)
> (materials are created, named and kept here and shown as preview images; a
> double-click on empty space or the + button creates one; the selected material's
> settings appear in the Attribute Manager; Shift+F2 shows the manager),
> [Material Tag](https://help.maxon.net/c4d/en-us/Content/html/TTEXTURE.html)
> (the link between a material and an object; dropping a material on an object in
> the Object Manager or the viewport creates the tag; Material ▸ Apply assigns it to
> the selected objects; dropping a material on an existing tag replaces its
> material; children without their own Material tag use their parent's; an object
> may carry several tags, layered), and the
> [keyboard shortcuts](https://help.maxon.net/c4d/en-us/Content/html/Shortcuts.html)
> (Material Manager: New Material Ctrl+N or double-click). Written in our own words.
>
> Covers implementation-plan step **6.6**.

## Purpose

Giving objects a surface: a material holds colour and surface properties, a
Material tag links it to an object (and the object's children), and the viewport
shows the material's colour. Physically based shading in the viewport comes in
Phase 7, the offline renderer in Phase 8.

## Parameters of a material

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Name | text | `Mat`, `Mat.1`, … | shown under the preview |
| Color | linear RGB | 80 % grey | the base colour; the viewport shows it |
| Roughness | 0–1 | 0.5 | microsurface: 0 mirror-like, 1 matte (used from Phase 7) |
| Metallic | 0–1 | 0 | dielectric (0) or metal (1) (used from Phase 7) |
| Luminance | linear RGB | black | light the surface gives off (used from Phase 7) |

Grid4D's material is a small physically based set (the plan's choice), not the
reference's channel-based Standard material.

## Behaviour

- **Material Manager**: a tab **Materials** beside Timeline and F-Curve at the
  bottom (Shift+F2 shows it). Each material is a round preview with its name under
  it (rendered by the path tracer since 1.1, see
  [`material-channels.md`](material-channels.md)).
  - **Create ▸ New Material**, **Ctrl+N** over the manager, or a **double-click on
    empty space** creates a material and selects it.
  - **Click** selects a material; the Attribute Manager shows its settings (name,
    Color, Roughness, Metallic, Luminance), edited live, one undo step per change.
    Selecting an object shows the object again.
  - **Delete** after a click in the manager (or **Edit ▸ Delete**, the context
    menu) removes the selected material. Material tags that used it stay, empty.
  - **Material ▸ Apply** (and the context menu) assigns the selected material to
    the selected objects.
  - **Drag a material** onto an object's row in the Object Manager, or onto an
    object in the viewport: the object gets a Material tag with it (an existing
    Material tag gets the new material).
- **Material tag** (Tags menu, Add Tag): links one material (*Material* list in
  the Attribute Manager). Its icon in the Object Manager shows the material's
  colour. **Double-click** the tag to show the material in the Attribute Manager.
- **Inheritance**: an object without its own Material tag uses its parent's (up
  the hierarchy). A generator's Material tag colours its result; without one the
  cage's is used.
- **Viewport**: an object with a material is drawn in the material's colour, else in
  its display colour.
- **Undo and files**: creating, editing and deleting materials and assigning them
  are undo steps. Materials are saved in `.g4d` (version 3 grows additively); glTF
  import and export carry name, colour, roughness, metallic and emission; OBJ
  export writes each material's colour under its name; merging a document brings
  its materials along.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `Material`, `MaterialId`, `Materials` (ordered table, unique names) | `grid4d-scene` (`material.rs`); the table lives in the `Scene`, so evaluation sees changes through the revision |
| `Tag::Material { material }`, `TagKind::Material`, `Tags::material` | `grid4d-scene` (`tag.rs`) |
| Edits | `SceneEdit::SetMaterials` (the whole table: cheap, ids kept); assignments are `SetData` |
| Inheritance and colour | `grid4d-eval` (walk keeps the inherited material) |
| File format | `.g4d` v3: `materials` list, `TagDto::Material` by index |
| Interchange | `Objects::materials` (+ `Objects::sources` to remap object references when merging) |
| UI | `panels/materials.rs` (bottom tab), material view in the Attribute Manager, `EditorCommand::NewMaterial` / `DeleteMaterial` / `AssignMaterial` |

Deliberate differences: one Material tag per object (no layering yet), no
projection settings (UVs are used as they are), no rendered thumbnails yet.

## Acceptance criteria

- [x] New, rename, edit and delete materials, each one undo step; names are
      unique.
- [x] Assigning by drag (Object Manager, viewport) and by Apply creates or updates a
      Material tag; the viewport shows the material's colour; children inherit it.
- [x] Materials and tags survive save and reload; glTF round-trips them; OBJ writes
      them; merging a document remaps them (and Target tags' objects).
- [x] Deleting a material leaves its tags empty; undo restores the link.
- [x] Idle frames leave the document untouched.
