# The `.g4d` format, version 6

> Sources: none outside Grid4D — this is our own file format. Decisions D24
> (one new version per release that adds document data) and D38 (version 6
> holds Phase 17's data) in [`IMPLEMENTATION_PLAN.md`](../IMPLEMENTATION_PLAN.md).
> Written 2026-11-09.
>
> Covers implementation-plan step **17.1**; steps 17.3 – 17.9 add their data
> to it as they arrive.

## Purpose

Grid4D 1.3 keeps data that 1.2 could not: the live modelling objects
(Boolean, Symmetry, Instance, Join, Spline Boolean, Reduce), edge creases and
hard edges on polygon objects, weighted normals on the Phong tag, and the new
deformers. Version 5 is frozen (Grid4D 1.2), so 1.3 writes **version 6**,
opens every older file, and 1.2 refuses the new files with its existing
"newer version" message rather than losing what it does not understand.

## What version 6 is

- **The same container** as versions 3 – 5: a zip archive with
  `document.json`, `meshes/<n>.bin`, `simulation.bin`, `textures/<n>.<ext>`,
  with the same rules (fixed timestamps, little-endian, zip64 for large
  entries, no trusted sizes).
- **Version 5 plus additions.** Everything version 5 holds is written the
  same way in version 6; the header says `"version": 6`. What Phase 17 adds,
  by step (planned; each step fills in its row and the exact field names):

| Step | Addition | Where in the file |
|------|----------|-------------------|
| 17.1 | nothing yet but the version number | — |
| 17.3 | the **Boolean** object: `mode`, `merge_faces`, `select_cut_edges`, `weld` (done; hard cut edges come with 17.8) | a new object kind (`type: "boolean"`; refused in a version 5 file) |
| 17.4 | the **Symmetry** (`plane`, `weld`, `tolerance`, `hold`, `flip`), **Instance** (`reference`) and **Join** (`weld`, `tolerance`) objects (done) | new object kinds (refused in a version 5 file); the link by object index, resolved like a camera's focus object |
| 17.5 | the **Spline Boolean** (`mode`, `plane`) and **Reduce** (`keep`, `keep_borders`, `keep_seams`, `together`) objects; `keep` as a track's parameter (done) | new object kinds and a parameter name (each refused in a version 5 file) |
| 17.6 – 17.7 | nothing: soft selection and the modelling tools' options are editor state, never saved in a document | — |
| 17.8 | **edge creases** and **hard edges** on a polygon object; the Phong tag's **weighted normals** | the `polygon` kind (`creases`, `hard`); the `phong` tag (`weighted`) |
| 17.9 | the **Wrap**, **Follow Spline**, **Displace**, **Project to Surface**, **Smooth**, **Explode** (and perhaps **Point Offsets**) deformers; their links (a spline, a target object, a texture) | new object kinds; links by index |

New keyable object parameters (a Reduce's percentage, a deformer's strength)
are new values of a track's parameter name; each is listed with its step.

- **Additive until 1.3 ships.** During Phase 17 version 6 grows only by
  optional fields left out while they have their default, and by new types
  and values. A document that uses none of Phase 17's features writes exactly
  what version 5 wrote, apart from the version number. When 1.3 is released
  version 6 is frozen like version 5: later data needs version 7.

## Reading older files

- **Version 5** is read by turning its `document.json` into version 6's (a
  change of the JSON, before the document is built), then reading it as
  version 6. The rules, the same as version 4's upgrade to version 5:
  - every **field** version 6 adds is **removed** from a version 5 file —
    from the document, the render settings (the document's, a take's own and
    Main's), each material, object kinds of a given type (on objects and in
    takes' overrides) and tags of a given type (in any `tags` list). Grid4D
    1.2 ignored unknown fields, so the file means what it meant to 1.2;
  - every **value** version 6 adds to a closed set (a new object `type`, a
    new parameter name) stays an **error** in a version 5 file, as it was for
    1.2 ("… is not part of format version 5");
  - where version 6 writes an old value in a new form, the conversion is
    part of the upgrade.
- **Versions 3 and 4** are first upgraded to version 5 (unchanged since 1.2),
  then to version 6 the same way. **Versions 1 and 2** (JSON) are read as
  before.
- Errors in an upgraded file read without a line and column (the JSON was
  already parsed); everything else about errors is unchanged.

## Writing

- Grid4D 1.3 always writes version 6. A document opened from version 5 or
  older and saved becomes version 6 (it no longer opens in 1.2; nothing tells
  the user beforehand, as for every earlier version change).
- Grid4D 1.2 opening a version 6 file shows its existing message: the file is
  from a newer Grid4D (version 6; it reads up to 5).

## Grid4D mapping

| Piece | Where |
|-------|-------|
| The current format (read and write) | `grid4d-io` `v6.rs` (version 5's module, moved and renamed) |
| The upgrade mechanism (fields dropped by place, new values refused) | `grid4d-io` `upgrade.rs` (`Place`, `Added`, `NewValue`, `additions`), shared by every additive upgrade |
| Version 5, read only | `grid4d-io` `v5.rs`: `upgrade`, the lists `ADDED` and `NEW_VALUES` (empty in 17.1) |
| Version 4, read only | `grid4d-io` `v4.rs`: `upgrade` to version 5, its lists (unchanged since 1.2) |
| Version 3, read only | `grid4d-io` `v3.rs`: `upgrade` to version 4 (unchanged) |
| Dispatch by version | `grid4d-io` `lib.rs` (`read_bytes`, `upgraded`: 3 → 4 → 5 → 6 as needed) |
| The specification | `docs/FILE_FORMAT.md` (version 6 current; what version 5 lacks) |
| Golden fixtures | `demo_v6.g4d`, `full_v6.g4d` (regenerated while version 6 grows); `demo_v5.g4d`, `full_v5.g4d` frozen with 1.2, the version 4 ones with 1.1, the version 3 ones with 1.0 |

## Acceptance criteria

- [x] Grid4D writes `"version": 6`; the v6 golden fixtures pin the bytes.
- [x] Every version 5 fixture loads and, saved again, gives exactly what the
      same document gives when written now (version 5's meaning has not
      moved); the version 4 and 3 fixtures still do too.
- [x] Random documents labelled version 5 (with Phase 16's data), version 4
      and version 3 read the same (property tests).
- [x] Fields version 6 adds are dropped from a version 5 file, at every place
      they can be; values it adds are refused (unit tests of the shared
      mechanism with a trial list; each later step tests its own entries).
- [x] Damaged and hostile files of every version are errors, never panics
      (`damaged.rs` reads every fixture; the fuzz target's seed corpus is the
      fixtures folder, which now holds version 6 and the frozen version 5).
- [x] A version newer than 6 is refused with both version numbers.
- [ ] Each later step lists its additions in `v5.rs` and `FILE_FORMAT.md`,
      extends `full_v6.g4d` and the property tests.
