# Wavefront OBJ import and export

> Sources (read 2026-10-20): the Wavefront OBJ and MTL file format descriptions
> ([OBJ](https://paulbourke.net/dataformats/obj/), [MTL](https://paulbourke.net/dataformats/mtl/)):
> `v`, `vt`, `vn`, `f` with 1-based (or negative, relative) indices, `o` objects,
> `g` groups, `usemtl` / `mtllib`, `Kd` diffuse colour; texture coordinates with
> their origin at the bottom left. Cinema 4D manual —
> [OBJ import geometry options](https://help.maxon.net/c4d/2026/en-us/Content/html/FOBJIMPORT2-OBJIMPORTOPTIONS_GROUP_GEOMETRY.html)
> and [export](https://help.maxon.net/c4d/2026/en-us/Content/html/FOBJEXPORT2-OBJEXPORTOPTIONS_GROUP_GEOMETRY.html)
> (a scale with a unit; groups can become separate objects or be combined; flips
> for files that arrive upside down or with swapped axes; flip faces; a Phong angle
> when the file has no normals). Written in our own words.
>
> Covers implementation-plan step **5.3**. Builds on
> [`interchange-gltf.md`](interchange-gltf.md) (Open / Merge / Export, `Objects`).

## Purpose

OBJ is the oldest and most widely supported mesh format: almost every 3D
application reads and writes it. It holds geometry only — no hierarchy, cameras or
lights — but keeps polygons with any number of corners, which glTF cannot.

## Parameters

None yet (an import/export dialog with scale, flips and splitting is *later*):

| What | Value | Why |
|------|-------|-----|
| Scale | 1 OBJ unit = 1 cm | OBJ has no units; the reference's default unit is the centimetre |
| Axes | unchanged | OBJ is Y-up and right-handed by convention, like Grid4D |
| UVs | V flipped (`v' = 1 − v`) both ways | OBJ's texture origin is the bottom left, Grid4D's (like glTF's) the top left |
| Splitting | one polygon object per `o` / `g` | the reference can split by groups too |

## Behaviour

- **Import** (File ▸ Open, File ▸ Merge, drag and drop — as for glTF):
  - each object or group becomes a **polygon object** at the origin, named after it
    (a file without `o`/`g` gives one object named after the file);
  - faces without texture coordinates get a stand-in from the reader when the file
    has some elsewhere; a UV map that is the same value everywhere is dropped;
  - **polygons keep their corners** (quads stay quads, n-gons stay n-gons); points
    shared by index stay shared; texture coordinates become per-corner UVs;
    normals are not kept (Grid4D smooths by angle);
  - the first material's diffuse colour (`Kd`, from the `.mtl` named by `mtllib`,
    read only from a path relative to the OBJ) becomes the display colour;
  - lines, points, curves and surfaces are skipped; a face with fewer than three
    corners is skipped; repeated corners are dropped;
  - errors (an index out of range, non-finite numbers, unreadable text) leave the
    document untouched; a missing `.mtl` is not an error (default colours).
- **Export** (File ▸ Export… with the `.obj` type):
  - every polygon object — primitives and generators baked as for glTF — becomes one
    `o` with its points in **world space** (OBJ has no transforms or hierarchy);
  - polygons are written with all their corners, with per-corner `vt` (when the mesh
    has UVs) and `vn` from Grid4D's smoothing (what the viewport shows);
  - display colours go to a `.mtl` file of the same name (`Kd`, one material per
    colour) named by `mtllib`, each object using its colour with `usemtl`;
  - cameras, lights and Nulls are not written (OBJ cannot hold them).
- **Round trip:** importing an exported OBJ gives one polygon object per exported
  object, at the origin, with the same world-space points and the **same polygons**
  (corner counts included), the same UVs and colours.

## As built (15.8b, materials and pictures)

- **In**: each `.mtl` material → a material: `Kd`, `Ke`, `Ni` (≥ 1, as IOR),
  `Pr` / `Pm` / `Pc` / `Pcr` (the PBR extension; `tobj` keeps them as unknown
  parameters), else roughness √(2 / (Ns + 2)) from Phong's shininess.
  Pictures: `map_Kd` → Base Color, `map_Ke` → Emission (sRGB), `map_d` →
  Opacity, `bump` / `map_Bump` → Bump (`-bm` → Height), `norm` → Normal,
  `map_Pr` / `map_Pm` → Roughness / Metallic (linear); options `-s` (tiling),
  `-o` (offset, V turned round: offset_v = 1 − scale_v − o_v), `-imfchan`
  (`r` `g` `b`, `m` → alpha; others red); the other options are skipped; the
  file is the rest of the line (spaces allowed). Pictures are linked:
  relative paths next to the OBJ (backslashes read as folder breaks),
  absolute ones as written; one texture per file and colour space.
- **Out**: a material's block gets `Kd`, `Ka 0`, `Ks 0`, `Ke` (emission ×
  strength), `Pr`, `Pm`, `Ni` (when not 1.5), `Pc` / `Pcr` (with a coat),
  `illum 2`, and the picture lines above with `-s`, `-o`, `-bm` and
  `-imfchan` as needed. Linked pictures by path (relative inside the OBJ's
  folder); embedded ones are written next to the OBJ under their file name
  (made unique) and named so.
- Tests: picture lines and their options; a `.mtl` with every picture and
  number; export → import keeping values, tiling, offsets, picture channels
  and files.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `import_obj(text, base) -> Objects` | `grid4d-io::obj` (the `tobj` crate without default features: faces kept, separate indices; the `.mtl` read by us, relative paths only) |
| `export_obj(objects, path)` / `obj_text(objects, mtl_name) -> (obj, mtl)` | `grid4d-io::obj` (written by hand; world matrices from the `Objects` hierarchy) |
| `.obj` in Open / Merge / drop / Export | `IMPORT_EXTENSIONS`, the dialogs' filters, `export_gltf` → `export` by extension |

## Acceptance criteria

- [x] Quads and n-gons import with their corners; `vt` become per-corner UVs with V
      flipped; `o`/`g` split objects; negative indices work.
- [x] `Kd` of the `.mtl` becomes the display colour; a missing `.mtl` is fine; an
      absolute `mtllib` path is not followed.
- [x] Bad indices and non-finite numbers are errors, never panics.
- [x] Export → import gives the same world-space points, polygons and UVs; cameras,
      lights and Nulls are left out; the `.mtl` holds one material per colour.
- [x] `.obj` works with File ▸ Open, File ▸ Merge, drag and drop and File ▸ Export.
- [x] (15.8b) `.mtl` pictures and PBR numbers come in and go out; embedded
      pictures are written next to the OBJ.
