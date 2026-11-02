# Changelog

Notable changes to Grid4D, newest first. Versions follow
[Semantic Versioning](https://semver.org/): a minor version adds features and
opens every file of the same major version; the plugin and script interfaces
may grow in a minor version and change only in a major one.

## [1.0.0-rc.1] - 2026-11-02

The first release candidate for Grid4D 1.0: installable packages for
Windows, macOS and Linux, for trying everything before 1.0.

**What is in it**

- **Modelling** — parametric primitives; polygon modelling with Points,
  Edges and Polygons modes (extrude, inset, bevel, bridge, loop and line
  cuts, weld, melt, dissolve, optimize); Subdivision Surfaces; splines drawn
  with the Pen and spline primitives; Extrude, Lathe, Sweep and Loft; 3D text.
- **Deformers and MoGraph** — Bend, Twist, Taper, Bulge and FFD; a Cloner
  (linear, radial, grid, on a surface) with Plain and Random effectors.
- **Animation** — keys on transforms and object parameters, Autokeying, the
  timeline and an F-Curve editor; rigid-body dynamics on objects and clones,
  cached and saved with the document.
- **Rendering** — materials and lights; a viewport with display modes,
  shadows and ambient occlusion, up to four views and scene cameras; a path
  tracer with an interactive Render View, a Picture Viewer, and stills and
  animations to PNG or OpenEXR.
- **Workspace** — dockable layouts, Layers and Takes, Preferences (units,
  undo depth, autosave, Dark / Darker / Light), English and Greek.
- **Files** — the `.g4d` format, version 3, frozen and documented
  (`docs/FILE_FORMAT.md`); glTF 2.0 and OBJ import and export.
- **Extending** — Rhai scripts with a Script Manager; plugins written in Rust.
- **Dependability** — undo for every change; autosave with recovery after a
  crash; local crash reports (never sent anywhere); the performance budget
  measured (`docs/performance.md`).

**Installing**

Download the package for your system from the release's files. They are not
code-signed yet: on Windows choose *More info ▸ Run anyway*; on macOS
right-click `Grid4D.app` and choose *Open*. Every download's SHA-256 is in
`SHA256SUMS.txt`.
