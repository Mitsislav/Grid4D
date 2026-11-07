# glTF 2.0 import and export (File ▸ Merge, File ▸ Export)

> Sources (read 2026-10-20): Khronos [glTF 2.0 specification](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html)
> (units, axes, node transforms, meshes and accessors, cameras, `.glb` container) and
> [`KHR_lights_punctual`](https://github.com/KhronosGroup/glTF/tree/main/extensions/2.0/Khronos/KHR_lights_punctual);
> Cinema 4D 2026/2027 manual — [glTF Import settings](https://help.maxon.net/c4d/2026/en-us/Content/html/FGLTFIMPORTER.html)
> (a Scale factor with a unit, toggles for cameras, geometry and materials; the tip
> to run Optimize on objects that arrive as separate polygons),
> [Import/Export preferences](https://help.maxon.net/c4d/r21/us/html/FC4D-C4DIMPORTFILTER_GROUP.html)
> (*Merge on Drag & Drop*; Shift reverses it),
> [Keyboard Shortcuts](https://help.maxon.net/c4d/2025/en-us/Content/html/Shortcuts.html)
> (*Merge Project* = Shift+Ctrl+O). Written in our own words.
>
> Export (read 2026-10-20): the manual's [glTF Export settings](https://help.maxon.net/c4d/s22/us/html/FGLTFEXPORTER-GLTFEXPORTER_GROUP.html)
> (glTF text or GLB binary — GLB at least a third smaller —, a scale with a unit,
> optional cameras, normals following the object's smoothing, the first UV set, and
> *Flip Z* because the reference application is left-handed while glTF is
> right-handed; Grid4D is right-handed, so nothing flips).
>
> Covers implementation-plan steps **5.1** (import) and **5.2** (export).

## Purpose

Bring models from other applications into Grid4D: open a `.gltf` or `.glb` file as
a new document, or merge it (or another Grid4D document) into the current one.

## Parameters

None yet (an import dialog with Scale and toggles is *later*). Fixed choices:

| What | Value | Why |
|------|-------|-----|
| Scale | × 100 | glTF lengths are metres, Grid4D's centimetres |
| Axes | unchanged | both are right-handed and Y-up; cameras look along −Z in both |
| Welding | identical positions merged | glTF repeats a vertex for each normal/UV seam; merged, the mesh is connected (so modelling tools and Subdivision Surface work) and the seams live on in the per-corner UVs |

## Behaviour

- **File ▸ Open…** also accepts `.gltf` and `.glb`. The model becomes a new,
  untitled document named after the file ("Save changes?" first, as for any open);
  saving asks for a `.g4d` path.
- **File ▸ Merge…** (Shift+Ctrl+O) adds the objects of a `.gltf`, `.glb` or `.g4d`
  file to the current document, at the top level, as **one undo step**; the merged
  top-level objects become the selection.
- **Drag and drop:** a dropped `.g4d` opens (as before); a dropped `.gltf`/`.glb`
  is merged (the reference's *Merge on Drag & Drop*). Other files: an alert.
- **What is read** (the file's default scene, or its first):
  - the node hierarchy, names, and TRS transforms (matrices are decomposed);
    translations × 100, rotations converted to HPB, scales kept;
  - each mesh → one **polygon object** (all its triangle primitives merged; strips
    and fans become triangles; points and lines are skipped); positions × 100,
    identical positions welded, `TEXCOORD_0` as per-corner UVs (origin top-left, as
    in glTF), degenerate triangles dropped. Normals are not kept: Grid4D smooths by
    angle. A mesh used by several nodes is shared;
  - the base colour factor of a primitive's material → the object's display colour;
  - perspective cameras → Camera objects (vertical field of view); orthographic ones
    become cameras with the default field of view;
  - `KHR_lights_punctual` point, spot and directional lights → Omni, Spot and
    Infinite Light objects (colour, intensity as given; a spot's cone half-angles
    doubled into opening angles, an inner cone as wide as the outer one is a hard
    edge);
  - a node with a mesh *and* a camera or light keeps the mesh; the other becomes a
    child at the same place;
  - a node without content → a Null; empty names → the kind's label.
- **Not read (later):** animation, skins, morph targets, Draco/meshopt
  compression, sparse accessors beyond what the reader supports; occlusion
  pictures, `KHR_texture_transform` rotation, second UV sets (materials and
  textures: 15.8, below).
- **Errors** leave the document untouched and show an alert: unreadable JSON or
  binary, a buffer shorter than declared, a missing external `.bin`, non-finite
  numbers, an external URI that is absolute or not a file.
- **Welding** compares exact positions (with `-0.0` equal to `0.0`); points that are
  only nearly equal stay separate (Optimize merges them).
- **Safety:** external buffers are read only from paths relative to the glTF file;
  `data:` URIs are decoded (base64); cycles in a broken node graph are cut.

## Export (5.2)

- **File ▸ Export…** asks for a file name; the type picks the flavour:
  **`.glb`** (one binary file, suggested) or **`.gltf`** (JSON plus a `.bin` file of
  the same name next to it). The document itself does not change (no undo step, no
  unsaved-changes mark); a failed export shows an alert.
- **What is written:** every object of the document, in its hierarchy, with names
  and TRS transforms (HPB → quaternion, translations ÷ 100: centimetres → metres).
  - Polygon objects → meshes, triangulated (an n-gon with *n* corners gives
    *n* − 2 triangles), with per-corner **normals from Grid4D's own smoothing** (what
    the viewport shows) and the UVs when the mesh has them. Vertices that agree in
    position, normal and UV are shared.
  - **Primitives** → meshes of their current shape (what Make Editable would give).
  - **Generators** → their result at the viewport level, in the cage's display
    colour (as the viewport draws it); the cage is not written. A switched-off
    generator is written as an empty node with its children as they are.
  - Display colours → materials (base colour, not metallic), one per colour.
  - Cameras → perspective cameras (vertical field of view); lights →
    `KHR_lights_punctual` point, spot (cone half-angles; a hard cone's inner angle
    just inside the outer one) and directional lights (colour, intensity); Nulls
    and empty polygon objects → nodes without content.
- **Round trip:** importing an exported file gives back the hierarchy, names,
  transforms, cameras and lights, and every mesh as a polygon object with the same
  points (triangulated, welded).
- **Not written (later):** animation, an export dialog (scale, visible objects
  only, selection only); pictures in formats other than PNG and JPEG
  (materials and textures: 15.8, below).

## As built (15.8a, materials and textures)

- **In**: each image becomes a texture once per colour space (sRGB for base
  colour and emission, linear for data), embedded when it is in the file (a
  buffer view or a `data:` URI; named after the image, else `image<n>.png`)
  and linked when it is a relative path next to the file (absolute paths are
  not followed). Base colour → Base Color; metallic-roughness → Roughness
  (green) and Metallic (blue), the factors becoming the ranges' tops;
  normal (its scale → Normal Strength); emissive → Emission; alpha Mask
  (cutoff → Threshold) or Blend (cut at 0.5) → an Opacity slot reading the
  base colour picture's alpha, the alpha factor → Opacity;
  `KHR_texture_transform` offset and scale → Offset and Tiling;
  `KHR_materials_transmission` (and its picture), `_ior`,
  `_emissive_strength`, `_specular` (factor × 0.5 → Specular) and
  `_clearcoat` (read from the raw extension); `doubleSided` → Two-Sided.
- **Out**: pictures go into the binary buffer (PNG and JPEG, by their first
  bytes; one image and texture per document texture), so `.gltf` files need
  no extra files. Base Color, Normal (scale = Normal Strength) and Emission
  pictures; Roughness and Metallic as one metallic-roughness picture when
  both read one texture's green and blue (the ranges' tops as factors); an
  Opacity slot on the base colour picture's alpha → `MASK` with the
  threshold as cutoff; Tiling / Offset → `KHR_texture_transform`;
  transmission, IOR, specular, clear coat and emission strength → their
  `KHR_materials_*` extensions; Two-Sided → `doubleSided`. Every extension
  used is listed in `extensionsUsed`.
- `TextureSlot::source` (`PictureChannel`: Red, Green, Blue, Alpha) says
  which channel a number channel reads; the viewport's shader and the path
  tracer read it; `.g4d` v4 stores it as `channel`.
- `Objects` carry the textures; File ▸ Merge adds them to the document's
  (a picture already linked is reused) and points the merged materials at
  them, in the same undo step.
- Tests: every channel and picture through `.glb` and back; pictures glTF
  cannot hold left out; a hand-written sample with a data-URI picture,
  `KHR_texture_transform`, a picture next to the file, an absolute one,
  alpha Mask and a clear coat.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `Objects { nodes: Vec<ObjectEntry { parent, data }> }` (pre-order), `import(path)`, `import_gltf(bytes, base)`, `Objects::from_document` / `into_document`, `IMPORT_EXTENSIONS` | `grid4d-io` (`import.rs`, `gltf_import.rs`; the `gltf` crate without its image/import features, base64 decoded by us) |
| Merge as one undo step | `ui/file.rs::merge`: the nodes inserted in order with `SceneEdit::Insert`, their inverses recorded as one history step "Merge" |
| `EditorCommand::Merge` (dialog) / `MergeFile(path)` (drop), File ▸ Merge…, Shift+Ctrl+O | `commands.rs`, `menu_bar.rs`, `shortcuts.rs` |
| Open accepting glTF | `file::open_path` (by extension) and the dialog's filters |
| Export: `Objects` → `.glb` / `.gltf` + `.bin` (`export_gltf(objects, path)`, `gltf_document(objects, buffer_uri)`, `glb_bytes(objects)`) | `grid4d-io` (`gltf_export.rs`; JSON written with `serde_json`, meshes triangulated by `grid4d_geom::Triangulator`) |
| Baking primitives and generators into polygon objects for export | `ui/file.rs::exportable` (uses `grid4d_eval::editable_mesh` / `generator_mesh`; `grid4d-io` stays independent of evaluation) |
| File ▸ Export… | `EditorCommand::Export`, `FileDialogs::pick_export` |

## Acceptance criteria

- [x] A minimal triangle and a cube (embedded base64 buffer, external `.bin` and
      `.glb`) import with the right points, faces, UVs and ×100 scale.
- [x] Node hierarchy, names, TRS and matrix transforms arrive as in the file.
- [x] Split vertices are welded: a glTF cube with 24 vertices becomes 8 points and
      6·2 triangles, closed.
- [x] Cameras, lights and Nulls become the right kinds; base colour → display colour.
- [x] Broken files (bad JSON, short buffer, missing `.bin`, absolute URI, NaN) give
      errors, never panics.
- [x] Merge adds the objects as one undo step (undo removes them all, redo brings
      the same ones back) and selects them; Open makes a new untitled document.
- [x] A dropped glTF merges; a dropped `.g4d` still opens.

Export:

- [x] Export → import round-trips the demo scene: names, hierarchy, transforms,
      cameras, lights; every primitive arrives as a polygon object with its points.
- [x] `.glb` and `.gltf` + `.bin` give the same model on import.
- [x] A Subdivision Surface is written as its smoothed result without its cage; a
      switched-off one as an empty node with its children.
- [x] N-gons become *n* − 2 triangles; normals follow the smoothing; UVs are kept.
- [x] Non-finite numbers are refused; File ▸ Export changes nothing in the document
      and a failed export shows an alert.

The phase's acceptance also names the Khronos sample models (Box, Duck, …). They
are not in the repository (downloading them needs a decision on test assets);
the generated scenes in the tests cover the same features.
