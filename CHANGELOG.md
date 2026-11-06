# Changelog

Notable changes to Grid4D, newest first. Versions follow
[Semantic Versioning](https://semver.org/): a minor version adds features and
opens every file of the same major version; the plugin and script interfaces
may grow in a minor version and change only in a major one.

## [Unreleased]

Toward 1.1: materials, UVs and textures.

- **Files** — documents are saved in `.g4d` format version 4. Every file of
  Grid4D 1.0 (version 3) opens unchanged; Grid4D 1.0 cannot open version 4
  files and says they come from a newer Grid4D.

## [1.0.0] - 2026-11-02

The first stable release: installable packages for Windows, macOS and Linux,
a frozen and documented file format, and everything below.

**What is in it**

- **Modelling** — parametric primitives; polygon modelling with Points,
  Edges and Polygons modes (extrude, inset, bevel, bridge, loop and line
  cuts, weld, melt, dissolve, connect, optimize; loop, ring, grow, shrink and
  connected selections); snapping; Subdivision Surfaces.
- **Splines** — spline primitives, the Pen, point and handle editing;
  Extrude, Lathe, Sweep and Loft; 3D text.
- **Deformers and motion graphics** — Bend, Twist, Taper, Bulge and FFD; a
  Cloner (linear, radial, grid, on a surface) with Plain and Random effectors.
- **Animation** — keys on transforms and object parameters, Autokeying, the
  timeline and an F-Curve editor.
- **Simulation** — rigid and collider bodies on objects and clones, cached
  and saved with the document.
- **Rendering** — materials and lights; a viewport with display modes,
  shadows, ambient occlusion, up to four views and scene cameras; a path
  tracer with an interactive Render View, a Picture Viewer, and stills and
  animations to PNG or OpenEXR.
- **Workspace** — dockable layouts, Layers and Takes, Preferences (units,
  undo depth, autosave, Dark / Darker / Light), English and Greek.
- **Files** — the `.g4d` format, version 3, frozen and documented
  ([`docs/FILE_FORMAT.md`](docs/FILE_FORMAT.md)); glTF 2.0 and OBJ import
  and export.
- **Extending** — Rhai scripts with a Script Manager; plugins written in Rust.
- **Dependability** — undo for every change; autosave with recovery after a
  crash; local crash reports (never sent anywhere); a measured performance
  budget ([`docs/performance.md`](docs/performance.md)).

**Since 1.0.0-rc.1** — the documentation for 1.0: a new README with fresh
pictures, this changelog, a review of names and wording.

**Installing** — download the package for your system from the release's
files. They are not code-signed yet: on Windows choose *More info ▸ Run
anyway*; on macOS right-click `Grid4D.app` and choose *Open*. Every
download's SHA-256 is in `SHA256SUMS.txt`.

## [1.0.0-rc.1] - 2026-11-02

The first release candidate: the packages for Windows (`.zip`, `.msi`),
macOS (`.dmg`, Apple silicon and Intel) and Linux (`.tar.gz`, AppImage), built
by the release workflow, for trying everything before 1.0.

## The road to 1.0

Before 1.0 Grid4D was built in phases, one version each; the details are in
the [progress log](docs/progress.md).

| Version | What it brought |
|---------|-----------------|
| 1.0 (Phase 14) | Stabilisation: supply-chain checks, the frozen file format with property tests and fuzzing, robustness (crash reports, failures told in words), the performance budget measured, interface tests, packages and the release workflow |
| 0.13 (Phase 13) | Plugins built in through a plugin API; Rhai scripts as one undo step; the Script Manager |
| 0.12 (Phase 12) | Docking layouts, Preferences, Layers, Takes, the interface in Greek |
| 0.11 (Phase 11) | Rigid-body simulation on objects and clones |
| 0.10 (Phase 10) | Animated object parameters; deformers; Current State to Object; the Cloner and effectors |
| 0.9 (Phase 9) | Splines, the Pen, Extrude / Lathe / Sweep / Loft, text |
| 0.8 (Phase 8) | The path tracer: Render View, Picture Viewer, PNG and OpenEXR |
| 0.7 (Phase 7) | Scene cameras, four views, display modes, lights, snapping |
| 0.6 (Phase 6) | Keyframes and F-curves, tags, materials, the Coordinate and Material Managers |
| 0.5 (Phase 5) | glTF 2.0 and OBJ import and export |
| 0.4 (Phase 4) | Polygon modelling and the Subdivision Surface |
| 0.3 (Phase 3) | Primitives with segments, Make Editable, the offscreen viewport |
| 0.2 (Phases 1 – 2) | Undo and redo; saving and opening documents |
| 0.1 | The editor shell: layout, viewport, Object and Attribute Managers |
