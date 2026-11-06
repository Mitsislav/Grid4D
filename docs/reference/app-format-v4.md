# The `.g4d` format, version 4

> Sources: none outside Grid4D — this is our own file format. Decisions D20
> (version 3 frozen with 1.0) and D24 (one new version per release that adds
> document data) in [`IMPLEMENTATION_PLAN.md`](../IMPLEMENTATION_PLAN.md).
> Written 2026-11-06.
>
> Covers implementation-plan step **15.1**; steps 15.2 – 15.8 add their data to
> it as they arrive.

## Purpose

Grid4D 1.1 keeps data that 1.0 could not: materials with texture channels, the
document's images, projections on Material tags. Version 3 is frozen, so 1.1
writes **version 4**, opens every older file, and 1.0 refuses the new files
with a clear message rather than losing what it does not understand.

## What version 4 is

- **The same container** as version 3: a zip archive with `document.json`,
  `meshes/<n>.bin`, `simulation.bin`, with the same rules (fixed timestamps,
  little-endian, zip64 for large entries, no trusted sizes).
- **Version 3 plus additions.** Everything version 3 holds is written the
  same way in version 4; the header says `"version": 4`. What Phase 15 adds,
  by step:

| Step | Addition | Where in the file |
|------|----------|-------------------|
| 15.1 | nothing yet but the version number | — |
| 15.2 | UVs made by primitives and generators are evaluated, not stored (polygon objects already store theirs) | — |
| 15.3 | the document's **textures**: each linked by a path or embedded | `document.textures`; embedded images as `textures/<n>.<ext>` entries |
| 15.4 | **material channels** (see [`material-channels.md`](material-channels.md)); **projections** on Material tags, several Material tags per object (see [`tag-material.md`](tag-material.md)) | `document.materials`, tags |
| 15.7 | **seams** marked in the UV editor, kept with a polygon object's components (its UVs are already stored) | the `polygon` kind |

- **Additive until 1.1 ships.** During Phase 15 version 4 grows only by
  optional fields that are left out while they have their default, and by new
  types. When 1.1 is released version 4 is frozen like version 3: later data
  needs version 5.

## Reading older files

- **Version 3** is read by turning its `document.json` into version 4's
  (a change of the JSON, before the document is built), then reading it as
  version 4. The rules of that change:
  - every field version 4 adds is **removed** from a version 3 file (Grid4D
    1.0 ignored unknown fields, so the file means what it meant to 1.0);
  - where version 4 writes an old value in a new form (a material's four
    numbers become its base channels), the conversion is part of the upgrade;
  - an unknown `type` stays an error, as in version 3.
- **Versions 1 and 2** (JSON) are read as before.
- Errors in an upgraded file read without a line and column (the JSON was
  already parsed); everything else about errors is unchanged.

## Writing

- Grid4D 1.1 always writes version 4. A document opened from version 3 and
  saved becomes version 4 (it no longer opens in 1.0; nothing tells the user
  beforehand, as for every earlier version change).
- Grid4D 1.0 opening a version 4 file shows its existing message: the file
  is from a newer Grid4D (version 4; it reads up to 3).

## Grid4D mapping

| Piece | Where |
|-------|-------|
| The current format (read and write) | `grid4d-io` `v4.rs` (version 3's module, moved and renamed) |
| Version 3, read only | `grid4d-io` `v3.rs`: `upgrade` (JSON → JSON) and the list of fields version 4 adds |
| Dispatch by version | `grid4d-io` `lib.rs` (`from_bytes_with_cache`) |
| The specification | `docs/FILE_FORMAT.md` (version 4 current; what version 3 lacks) |
| Golden fixtures | `demo_v4.g4d`, `full_v4.g4d` (regenerated while version 4 grows); `demo_v3.g4d`, `full_v3.g4d`, `demo_v3_initial.g4d` frozen, read only |

## Acceptance criteria

- [x] Grid4D writes `"version": 4`; the v4 golden fixtures pin the bytes.
- [x] Every version 3 fixture loads and, saved again, gives exactly what the
      same document gives when written now (version 3's meaning has not
      moved).
- [x] Random documents labelled version 3 read the same (property test).
- [x] Damaged and hostile files of both versions are errors, never panics
      (`damaged.rs`, the fuzz target's seed corpus holds both).
- [x] A version newer than 4 is refused with both version numbers.
- [ ] Each later step lists its additions in `v3.rs` and `FILE_FORMAT.md`,
      extends `full_v4.g4d` and the property tests.
