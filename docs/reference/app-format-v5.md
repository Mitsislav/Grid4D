# The `.g4d` format, version 5

> Sources: none outside Grid4D — this is our own file format. Decisions D24
> (one new version per release that adds document data) and D31 (version 5
> holds Phase 16's data) in [`IMPLEMENTATION_PLAN.md`](../IMPLEMENTATION_PLAN.md).
> Written 2026-11-07.
>
> Covers implementation-plan step **16.1**; steps 16.2 – 16.10 add their data
> to it as they arrive.

## Purpose

Grid4D 1.2 keeps data that 1.1 could not: area lights, the Sky and
Environment objects, light linking and groups, the camera's lens and shutter,
render passes, sampling and denoising settings, the render region, the
Subsurface and Displacement channels. Version 4 is frozen (Grid4D 1.1), so
1.2 writes **version 5**, opens every older file, and 1.1 refuses the new
files with its existing "newer version" message rather than losing what it
does not understand.

## What version 5 is

- **The same container** as versions 3 and 4: a zip archive with
  `document.json`, `meshes/<n>.bin`, `simulation.bin`, `textures/<n>.<ext>`,
  with the same rules (fixed timestamps, little-endian, zip64 for large
  entries, no trusted sizes).
- **Version 4 plus additions.** Everything version 4 holds is written the
  same way in version 5; the header says `"version": 5`. What Phase 16 adds,
  by step (planned; each step fills in its row and the exact field names):

| Step | Addition | Where in the file |
|------|----------|-------------------|
| 16.1 | nothing yet but the version number | — |
| 16.2 | the **Area** light type and its shape, size, *Visible in render*, two-sided; "Clamp indirect" | the `light` kind (`light_type: "area"` is a new value); `document.render` |
| 16.3 | the **Sky** and **Environment** objects | new object kinds (new `type` values) |
| 16.4 | a light's **Include / Exclude** list and its **group** name | the `light` kind |
| 16.5 | the camera's **focus**, **f-stop**, **blades**, **rotation**; **shutter** and **segments** | the `camera` kind; `document.render` |
| 16.6 | the chosen **passes** | `document.render` |
| 16.7 | **adaptive sampling** (minimum, maximum, threshold) and the **denoiser** | `document.render` |
| 16.8 | the **render region** | `document.render` |
| 16.10 | the **Subsurface** and **Displacement** channels, a render subdivision level | `document.materials`; `document.render` / object kinds |

- **Frozen since 1.2 (2026-11-09).** What follows held during Phase 16.
- **Additive until 1.2 ships.** During Phase 16 version 5 grows only by
  optional fields left out while they have their default, and by new types
  and values. A document that uses none of Phase 16's features writes exactly
  what version 4 wrote, apart from the version number. When 1.2 is released
  version 5 is frozen like version 4: later data needs version 6.

## Reading older files

- **Version 4** is read by turning its `document.json` into version 5's (a
  change of the JSON, before the document is built), then reading it as
  version 5. The rules of that change:
  - every **field** version 5 adds is **removed** from a version 4 file —
    from the document, the render settings (the document's, a take's own and
    Main's), each material, object kinds of a given type (on objects and in
    takes' overrides) and tags of a given type (in any `tags` list). Grid4D
    1.1 ignored unknown fields, so the file means what it meant to 1.1;
  - every **value** version 5 adds to a closed set (a new object `type`, the
    `area` light type) stays an **error** in a version 4 file, as it was for
    1.1 ("… is not part of format version 4");
  - where version 5 writes an old value in a new form, the conversion is
    part of the upgrade.
- **Version 3** is first upgraded to version 4 (unchanged since 1.1), then to
  version 5 the same way. **Versions 1 and 2** (JSON) are read as before.
- Errors in an upgraded file read without a line and column (the JSON was
  already parsed); everything else about errors is unchanged.

## Writing

- Grid4D 1.2 always writes version 5. A document opened from version 4 or
  older and saved becomes version 5 (it no longer opens in 1.1; nothing tells
  the user beforehand, as for every earlier version change).
- Grid4D 1.1 opening a version 5 file shows its existing message: the file
  is from a newer Grid4D (version 5; it reads up to 4).

## Grid4D mapping

| Piece | Where |
|-------|-------|
| The current format (read and write) | `grid4d-io` `v5.rs` (version 4's module, moved and renamed) |
| Version 4, read only | `grid4d-io` `v4.rs`: `upgrade` (JSON → JSON), the lists `ADDED` (fields by place) and `NEW_VALUES` |
| Version 3, read only | `grid4d-io` `v3.rs`: `upgrade` to version 4 (unchanged) |
| Dispatch by version | `grid4d-io` `lib.rs` (`read_bytes`) |
| The specification | `docs/FILE_FORMAT.md` (version 5 current; what version 4 lacks) |
| Golden fixtures | `demo_v5.g4d`, `full_v5.g4d` (regenerated while version 5 grows); `demo_v4.g4d`, `full_v4.g4d` frozen with 1.1, the version 3 ones with 1.0 |

## Acceptance criteria

- [x] Grid4D writes `"version": 5`; the v5 golden fixtures pin the bytes.
- [x] Every version 4 fixture loads and, saved again, gives exactly what the
      same document gives when written now (version 4's meaning has not
      moved); the version 3 fixtures still do too.
- [x] Random documents labelled version 4 (with Phase 15's data) and version
      3 read the same (property tests).
- [x] Fields version 5 adds are dropped from a version 4 file, at every place
      they can be; values it adds are refused (unit tests of the upgrade with
      a trial list).
- [x] Damaged and hostile files of every version are errors, never panics
      (`damaged.rs` reads every fixture; the fuzz target's seed corpus is the
      fixtures folder).
- [x] A version newer than 5 is refused with both version numbers.
- [ ] Each later step lists its additions in `v4.rs` and `FILE_FORMAT.md`,
      extends `full_v5.g4d` and the property tests.
