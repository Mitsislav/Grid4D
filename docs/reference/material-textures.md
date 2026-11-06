# Textures and the Texture Manager

> Sources: the usual behaviour of image textures in 3D suites (linked files,
> relative paths, missing-file warnings) and Grid4D's decision D25 (the
> `image` crate, colour space per texture, linked or embedded). The reference
> keeps texture paths in its materials and lists them in its project asset
> tools; we keep our own table and manager. Written in our own words,
> 2026-11-06.
>
> Covers implementation-plan step **15.3** (implemented 2026-11-06; materials
> use textures from step 15.4).

## Purpose

Pictures that materials use — a photo of wood, a normal map, an HDR — kept
once per document, loaded without freezing the interface, and never lost
silently when a file moves.

## A texture

| Setting | Values | Default | Effect |
|---------|--------|---------|--------|
| Name | text | the file's name | shown in menus and the Texture Manager |
| Source | *Linked* (a path) or *Embedded* (the file's bytes in the `.g4d`) | Linked | where the picture comes from |
| Path | a file path | — | relative to the document's folder when it is inside it or below, else absolute |
| Colour space | *sRGB* or *Linear* | sRGB for colour images; Linear for HDR / EXR and for textures first linked to a data channel | how the pixel values are read |

Formats: **PNG, JPEG, TIFF, WebP, Radiance HDR, OpenEXR** (D25: the
`image` crate with only these decoders, pure Rust, permissive licences).

## Behaviour

- **Adding:** choosing an image for a material's channel (file dialog, or
  dropping a file on the channel) adds a texture, or reuses the one with the
  same path. Dropping an image on the Texture Manager adds it unused.
- **Loading:** images load on a background thread, decoded once and kept
  in a cache by path and modification time (a file changed on disk loads
  again when the document is next evaluated after the change is seen);
  mip-maps are made once at load. Until an image arrives the viewport shows
  the channel's value.
- **Missing files** never stop a document from opening: the viewport shows
  a magenta and black checker, the path tracer the channel's value, and the
  Texture Manager marks the texture with a warning and the path it looked
  for. The status bar says how many textures are missing after opening.
- **Texture Manager** (a pane, Window menu): every texture of the document
  with a thumbnail, size, source and colour space; which materials use it.
  - **Relink…** chooses another file for the selected texture (one undo
    step); **Relink Folder…** looks for every missing file by name in a folder.
  - **Embed** puts the file's bytes into the document (saved as
    `textures/<n>.<ext>` in the `.g4d`); **Unembed…** writes them out to a
    file next to the document and links it.
  - **Reveal** shows the file in the system's file browser.
  - **Remove Unused** deletes textures no material uses.
- **Save As** to another folder rewrites relative paths so they still point
  at the same files.
- **Undo:** adding, relinking, embedding and removing textures are undo
  steps; loading is not (it changes nothing in the document).

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `Texture`, `TextureId`, `Textures` (ordered table, like `Materials`) | `grid4d-scene` `texture.rs`; embedded bytes shared (`Arc<[u8]>`), never copied by undo |
| Edits | `SceneEdit::SetTextures` |
| Decoding, mip-maps | a new module of `grid4d-io` (`image.rs`), no UI or GPU |
| Background loading and cache | `grid4d-ui` (`textures.rs`, like `rendering.rs`), polled by `commands::apply_pending` |
| File | `.g4d` v4: `document.textures`, entries `textures/<n>.<ext>` |
| Pane | `panels/texture_manager.rs`, a `Pane` in every layout preset's tab set |

## As built (15.3)

- In memory a linked texture keeps the path as chosen (absolute from a
  dialog); only the file stores it relative, written against the folder it
  is saved in and read against the folder it is opened from — so Save As to
  another folder needs no rewriting at all.
- The Texture Manager shows paths inside the document's folder relative to
  it (`maps/wood.png`), others in full.
- Until materials have texture channels (15.4), no texture is "used":
  Remove Unused removes them all.
- Mip-maps are box-filtered, sRGB colour averaged in linear light; pictures
  with more than 8 bits a channel (16-bit, HDR, OpenEXR) are kept as half
  floats, the others as 8-bit RGBA.
- Dropped pictures are added when the drop holds no document or model.

## Acceptance criteria

- [x] Each of the six formats decodes; a broken image is a warning, not a
      crash; sizes a file claims are never trusted for memory.
- [x] A document with a missing texture opens, shows the checker and the
      warning, and Relink (or Relink Folder) fixes it in one undo step.
- [x] Embed / Unembed round-trip the exact bytes; a saved, reopened
      document is identical.
- [x] Save As to another folder keeps linked textures found.
- [x] Loading never blocks a frame (the interface keeps drawing while a
      large image decodes).
