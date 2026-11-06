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
`manager-` (Object/Attribute/Coordinate/Material managers), `viewport-`, `render-`,
`interchange-` (other file formats), `tag-`, `material-` (materials and textures),
`app-` (the program as a whole: releases, packages, the file format).

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
| 5.1–5.2 | [`interchange-gltf.md`](interchange-gltf.md) — glTF 2.0 import and export, File ▸ Merge / Export, drag and drop | implemented 2026-10-20 |
| 5.3 | [`interchange-obj.md`](interchange-obj.md) — Wavefront OBJ import and export | implemented 2026-10-20 |
| 6.1 | [`tag-basics.md`](tag-basics.md) — tags: Phong, Protection, Target | implemented 2026-10-20 |
| 6.2 | [`animation-keys.md`](animation-keys.md) — tracks, curves, keys, evaluation | implemented 2026-10-20 |
| 6.3 | [`animation-recording.md`](animation-recording.md) — Record (F9), Autokeying, key buttons, keys on the timeline | implemented 2026-10-20 |
| later | `modeling-sds-weights.md` — point/edge weights (creases) | todo |
| 6.4 | [`animation-fcurves.md`](animation-fcurves.md) — the F-Curve editor | implemented 2026-10-20 |
| 6.5 | [`manager-coordinate.md`](manager-coordinate.md) — the Coordinate Manager | implemented 2026-10-20 |
| 6.6 | [`manager-material.md`](manager-material.md) — materials, the Material Manager, the Material tag | implemented 2026-10-21 |
| 7.1 | [`object-camera.md`](object-camera.md) — scene cameras in the viewport | implemented 2026-10-21 |
| 7.2 | [`viewport-layouts.md`](viewport-layouts.md) — four views, orthographic projections, the Panel menu | implemented 2026-10-21 |
| 7.3 | [`viewport-display.md`](viewport-display.md) — display modes: shading, lines, Box | implemented 2026-10-21 |
| 7.4 | [`object-light.md`](object-light.md) — light types, viewport lighting, shading from materials | implemented 2026-10-21 |
| 7.5 | [`viewport-filter.md`](viewport-filter.md) — the Filter menu per view | implemented 2026-10-21 |
| 7.6 | [`viewport-effects.md`](viewport-effects.md) — shadows and SSAO in the viewport, the light's Shadow | implemented 2026-10-21 |
| 8.1 | [`render-engine.md`](render-engine.md) — the final renderer: what a render shows | implemented 2026-10-21 |
| 8.2 | [`render-settings.md`](render-settings.md) — Render Settings: size, frames, file, sampling, Auto Light | implemented 2026-10-21 |
| 8.3 | [`render-picture-viewer.md`](render-picture-viewer.md) — the Picture Viewer: background renders, history, zoom, saving | implemented 2026-10-28 |
| 8.4 | [`render-view.md`](render-view.md) — the Render View: rendering the active view in place | implemented 2026-10-28 |
| 8.5 | [`render-animation.md`](render-animation.md) — rendering frame ranges to numbered files, OpenEXR | implemented 2026-10-29 |
| 9.1–9.3 | [`object-spline.md`](object-spline.md) — spline objects: types, intermediate points, primitives, editing points, commands and the Pen | implemented 2026-10-29 |
| 9.4 | [`generator-extrude.md`](generator-extrude.md) — the Extrude generator: direction, offset, caps with holes, hierarchical | implemented 2026-10-29 |
| 9.5 | [`generator-lathe.md`](generator-lathe.md) — the Lathe generator: angle, subdivision, movement, scaling, caps | implemented 2026-10-29 |
| 9.6 | [`generator-sweep.md`](generator-sweep.md) — the Sweep generator: profile along a path, banking, growth, end scale and rotation | implemented 2026-10-29 |
| 9.7 | [`generator-loft.md`](generator-loft.md) — the Loft generator: a skin over sections, U / V, organic, loop, linear | implemented 2026-10-29 |
| 9.8 | [`object-text.md`](object-text.md) — the Text spline: outlines from the built-in font, lines, alignment, spacing | implemented 2026-10-29 |
| 10.1 | [`animation-parameters.md`](animation-parameters.md) — keys on object parameters | implemented 2026-10-29 |
| 10.2 | [`deformer-basics.md`](deformer-basics.md), [`deformer-bend.md`](deformer-bend.md) — who a deformer deforms, the box and its modes; the Bend | implemented 2026-10-29 |
| 10.3 | [`deformer-twist-taper-bulge.md`](deformer-twist-taper-bulge.md) — Twist, Taper, Bulge; strength handles | implemented 2026-10-29 |
| 10.4 | [`deformer-ffd.md`](deformer-ffd.md) — the FFD lattice | implemented 2026-10-29 |
| 10.5 | [`modeling-current-state.md`](modeling-current-state.md) — Current State to Object | implemented 2026-10-29 |
| 10.6–10.7 | [`mograph-cloner.md`](mograph-cloner.md) — the Cloner: Linear, Radial, Grid Array, Object; clones as instances | implemented 2026-10-30 |
| 10.8 | [`mograph-effectors.md`](mograph-effectors.md) — Plain and Random effectors, their falloff | implemented 2026-10-30 |
| 11.1–11.2 | [`simulation-rigid-body.md`](simulation-rigid-body.md) — Rigid Body and Collider Body tags: motion, shapes, mass, bounce, friction; project settings | implemented 2026-10-30 |
| 11.3, 11.5 | [`simulation-cache.md`](simulation-cache.md) — start state, the cache, Bake / Clear, playback and renders, saved caches | implemented 2026-10-30 |
| 11.4 | [`simulation-clones.md`](simulation-clones.md) — clones as bodies | implemented 2026-10-30 |
| 12.1 | [`workspace-layouts.md`](workspace-layouts.md) — docking panels, the Standard / Modeling / Animate layouts, Reset Layout | implemented 2026-10-30 |
| 12.2–12.3 | [`workspace-preferences.md`](workspace-preferences.md) — units, undo depth, autosave, viewport defaults, themes | implemented 2026-10-30 |
| 12.4 | [`manager-layer.md`](manager-layer.md) — layers and their switches, the Layer Manager | implemented 2026-10-30 |
| 12.5 | [`manager-take.md`](manager-take.md) — takes, overrides, Auto Take, rendering a take | implemented 2026-10-30 |
| 12.6 | [`workspace-language.md`](workspace-language.md) — the interface's languages, Greek | implemented 2026-10-30 |
| 13.1–13.3 | [`extend-plugins.md`](extend-plugins.md) — plugin generators, deformers and commands; parameters; missing plugins | implemented 2026-10-30 |
| 13.4–13.5 | [`extend-scripting.md`](extend-scripting.md) — scripts, the document API, the Script Manager | implemented 2026-10-31 |
| 14.1, 14.3–14.5 | [`app-release.md`](app-release.md) — the version and licences in Help ▸ About, crash reports, the frame-time readout, the interface pass | implemented 2026-11-01 – 02 |
| 14.6 | [`app-packaging.md`](app-packaging.md) — the downloads for Windows, macOS and Linux, file associations, the release workflow | implemented 2026-11-02 (every package built in a dry run; installs on each system to try) |
| 15.1 | [`app-format-v4.md`](app-format-v4.md) — `.g4d` version 4: what it adds by step, reading version 3, compatibility with 1.0 | implemented 2026-11-06 (the container; each later step adds its data) |
| 15.3 | [`material-textures.md`](material-textures.md) — the document's textures (linked, embedded), colour spaces, missing files, the Texture Manager | draft 2026-11-06 |
| 15.4 | [`material-channels.md`](material-channels.md) — the principled channels, a channel's texture, preview balls | draft 2026-11-06 |
| 15.4 | [`tag-material.md`](tag-material.md) — the Material tag's projections (UV, Box, Spherical, Cylindrical, Flat, Camera) and layers | draft 2026-11-06 |
| 15.7 | [`manager-uv-editor.md`](manager-uv-editor.md) — the UV editor: modes, transforms, Unwrap (LSCM), seams, Project, Relax, Pack | draft 2026-11-06 |
