# Reference — understanding the behaviour we aim for

Grid4D follows the workflow of Cinema 4D's Standard layout. This folder is where we
collect **what the reference application does**, so features match it closely in
behaviour while every line of Grid4D stays our own.

> **Clean-room rule.** We read public documentation to understand *behaviour*
> (parameters, defaults, what a tool does) and write it down **in our own words**.
> We never copy Maxon's text, screenshots, icons or code into this repository, and
> we never decompile or disassemble Maxon binaries — the EULA forbids it, and it
> would put an MIT/Apache project at legal risk. Names of parameters and the idea of
> a feature are fine; expression (prose, images, code) is not.

## Sources (all public, free)

Links checked 2026-10-19 against Cinema 4D **2027.0**. Maxon versions every SDK
URL (`2027_0_0`); swap the version segment for older or newer releases.

| Source | URL | Use it for |
|--------|-----|------------|
| User manual | <https://help.maxon.net/c4d/en-us/> | What each object, tool, tag and manager does; every parameter with its meaning; menus and workflow. Start here. |
| Python SDK | <https://developers.maxon.net/docs/py/2027_0_0/index.html> | Scene model as scripts see it: `BaseObject`, `PolygonObject`, tags, tracks, curves, keys. |
| Resource index (Python) | <https://developers.maxon.net/docs/py/2027_0_0/indexes/resource_index.html> | Parameter list of every object/tag/track (filter by *Objects*, *Tags*, …): exact names, types, ranges and often defaults. |
| C++ SDK | <https://developers.maxon.net/docs/cpp/2027_0_0/index.html> | Architecture: generators, deformers, caches and dirty flags (compare with `grid4d-eval`), the *Scene* and *Handbooks* chapters. |
| SDK docs hub | <https://developers.maxon.net/docs/> | Version selector; also the Cineware C++ API (reading/writing `.c4d` files — separate licence, see below). |
| Developer forum | <https://developers.maxon.net/forum/> | Answers from Maxon's SDK team on edge cases and internals. |
| Video tutorials | <https://www.cineversity.com/>, Maxon's YouTube channel | Seeing a tool in action: interaction, feedback, gizmos. |

The SDK pages link Maxon's *SDK License* and *EULA* in their footer. Reading the
documentation is fine; **do not** add SDK headers, sample code or Cineware binaries
to this repository. Supporting `.c4d` import would need a separate, optional plugin
after a licence review (not planned yet).

## What lives here

One Markdown file per feature or tool, written from the sources above **before** we
implement it, using [`_template.md`](_template.md):

```
docs/reference/
├── README.md            # this file
├── _template.md         # copy it for a new spec
└── <area>-<feature>.md  # e.g. modeling-bevel.md, generator-subdivision-surface.md
```

Area prefixes: `object-` (primitives, cameras, lights), `generator-`, `deformer-`,
`modeling-` (mesh tools and modes), `animation-` (keys, F-curves, timeline),
`manager-` (Object/Attribute/Coordinate/Material managers), `viewport-`, `render-`.

A spec is done when it lists the parameters with defaults, the behaviour (including
edge cases) and acceptance criteria that can become tests. The matching phase in
[`IMPLEMENTATION_PLAN.md`](../IMPLEMENTATION_PLAN.md) links to it.

## Planned specs (follow the implementation plan)

| Phase | Spec | Status |
|-------|------|--------|
| 3.6, 4.1–4.3 | [`modeling-modes.md`](modeling-modes.md) — Make Editable, Points / Edges / Polygons modes, component selection & transforms | draft 2026-10-19 |
| 4.4–4.6 | [`modeling-tools.md`](modeling-tools.md) — Extrude, Inset (4.4); Delete, Melt, Dissolve, Connect, Loop/Path Cut, Line Cut, chords (4.5); Bevel, Bridge, Weld, Optimize (4.6) | draft 2026-10-20 |
| 4.7 | [`generator-subdivision-surface.md`](generator-subdivision-surface.md) — generator framework, Catmull-Clark, levels, boundaries | implemented 2026-10-20 |
| 4.8 | [`modeling-snapping.md`](modeling-snapping.md) — snap targets, 2D/3D, radius, Shift+S | implemented 2026-10-20 |
| later | `modeling-sds-weights.md` — point/edge weights (creases) | todo |
| 6 | `animation-keyframes.md` — keys, tracks, F-curve interpolation, timeline | todo |
| 6 | `manager-coordinate.md`, `manager-material.md` | todo |
| 7 | `object-camera.md`, `object-light.md`, `viewport-layouts.md` | todo |
