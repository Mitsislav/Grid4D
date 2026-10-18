# OpenC4D

An open-source, modular 3D content creation suite written in Rust, with a
workflow and layout inspired by Cinema 4D.

![OpenC4D workspace](docs/screenshot.png)

**Status: v0.1 — editor shell.** The workspace, scene graph, GPU viewport,
selection and transform tools work; modelling, keyframes, file I/O and
rendering are on the roadmap (see `ARCHITECTURE.md`).

## What works today

- C4D-style **Standard layout**: menu bar, command palette, mode palette,
  3D viewport, Object Manager, Attribute Manager, timeline, status bar — all
  panels resize with the window and the right-hand managers can be dragged wider.
- **wgpu viewport**: instanced, MSAA, depth-tested primitives (cube, sphere,
  cylinder, plane, light and camera markers), anti-aliased adaptive grid,
  orange selection rim, axis triad and "Grid Spacing" HUD.
- **Navigation**: Alt+LMB orbit · Alt+MMB / MMB pan · Alt+RMB / wheel zoom ·
  `H` frame scene · `O` frame selection. Drag the icons at the top-right of the
  viewport header to orbit / zoom / pan with one button.
- **Selection & tools**: click to pick (Shift adds, Ctrl toggles), Move (`E`),
  Scale (`T`) and Rotate (`R`) gizmos with X/Y/Z axis locks and World/Object
  coordinates.
- **Object Manager**: hierarchy tree, fold arrows, editor/render visibility dots,
  enable check mark, double-click rename, drag-and-drop re-parenting (keeps the
  world position), search.
- **Attribute Manager**: Basic / Coord. / Object tabs (P/S/R in cm and degrees),
  primitive parameters, and project settings when nothing is selected.
- **Timeline**: ruler with scrubbable playhead, play/pause/step/loop
  (`F8`, `F6`, `F7`, `F`, `G`), preview range and fps.

## Build & run

Requires Rust **1.95+** (`rustup update stable`).
On Windows you also need the *MSVC build tools* (installed by `rustup` / Visual Studio Installer).

```bash
cargo run --release
```

The first build compiles egui and wgpu and takes a few minutes; later builds are
incremental. On Linux you may need the usual X11/Wayland development packages
(`libxkbcommon-dev`, `libwayland-dev`, `libx11-dev` …).

```bash
cargo test --workspace                                  # unit tests + WGSL validation
cargo clippy --workspace --all-targets -- -D warnings   # lints
```

## Architecture

```
oc4d-scene  ◀──  oc4d-render  ◀──  oc4d-ui  ◀──  open-c4d (binary)
(data)           (wgpu)            (egui)         (composition root)
```

| Crate         | Responsibility                                                      |
|---------------|---------------------------------------------------------------------|
| `oc4d-scene`  | Scene graph (slotmap arena, generational `NodeId`), transforms (HPB), bounds, document time. No GPU, no UI. |
| `oc4d-render` | Orbit camera, per-frame `FrameData` extraction, wgpu pipelines + WGSL shaders, picking. No UI. |
| `oc4d-ui`     | Theme, painted vector icons, panels, gizmo, shortcuts, command queue. |
| `open-c4d`    | Window / GPU surface setup and panel layout.                         |

The rules (dependency direction, command queue, allocation-free render loop,
module layout) are in [`ARCHITECTURE.md`](docs/ARCHITECTURE.md).

## Legal

OpenC4D is an independent open-source project inspired by the Cinema 4D
workflow. It is not affiliated with or endorsed by Maxon. Cinema 4D is a
trademark of Maxon Computer GmbH. All icons and artwork in this repository are
original.

License: to be decided (MIT OR Apache-2.0 is the Rust ecosystem default).
