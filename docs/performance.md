# Performance

What Grid4D promises about speed, how it is measured, and what was measured
on the reference machine. The budget itself is part of the release
specification (`reference/app-release.md`); this file holds the numbers.

## The reference machine

| | |
|-|-|
| Processor | AMD Ryzen 7 5800H (8 cores) |
| Graphics | NVIDIA GeForce RTX 3050 Laptop GPU (Vulkan) |
| Memory | 16 GB |
| Display | 240 Hz (frames are not held back to 60) |
| System | Windows 11 Home |
| Build | release (`cargo build --release`), Rust 1.99.1 |

Measured on 2026-11-01, at commit after `d0febb9` (step 14.4).

## The budget and the results

| Scenario | Budget | Measured | |
|----------|--------|----------|-|
| 10 000 primitive objects, playing (1 000 of them animated) | ≥ 60 fps | 150 – 160 fps (6.2 – 6.7 ms a frame; worst frame of two seconds 8.9 – 9.4 ms) | met |
| … the same, steady frames | no memory allocated by evaluation and extraction | 0 allocations (test) | met |
| One mesh of 1 000 000 triangles, redrawn every frame | ≥ 60 fps | 167 – 196 fps | met |
| … in Points / Edges / Polygons mode | ≥ 30 fps | 132 – 189 fps (Points the slowest) | met |
| Select All on 10 000 objects | no frame noticeably slower | one frame of 8.6 ms among frames of 3.4 – 4.3 ms (the whole interface, without the GPU) | met |
| Opening a 50 MB `.g4d` | under 2 s | 0.41 – 0.53 s to read and replace the document; 1.3 – 1.7 s until it is drawn | met |

Nothing missed the budget, so nothing is accepted as an exception.

### How each was measured

- **Frames on screen** — the scenes from `cargo run --release --example
  stress -- <folder>`, opened in a maximised window (1936 × 1048), read
  with **View ▸ Frame Time** while playing (F8). Playing redraws every frame;
  in `stress-10k.g4d` it also evaluates the animated objects whenever the
  document's frame changes, so it costs more than orbiting a still scene.
  The 1-million-triangle sphere was measured at the view's starting
  distance (about a fifth of the view's height); the component modes were
  measured with the sphere active.
- **Allocations** — `crates/render/tests/steady_frame.rs` counts the
  allocations of the viewport's CPU side (evaluation of 10 000 objects with
  1 000 animated, extraction, the component overlay of a polygon object)
  over a whole playback, an orbit and selection changes, after one warm-up
  pass: none. It runs with every `cargo test`.
- **Select All** — `select_all_on_10k_objects_timed` (in
  `crates/ui/src/panels/mod.rs`, ignored by default) draws every panel
  headless and times each frame: `cargo test --release -p grid4d-ui --
  --ignored --nocapture select_all_on_10k`. The frame that draws the new
  selection the first time takes about 5 ms more than its neighbours.
- **Opening** — the time from starting Grid4D with the file to its title
  showing the file, against starting without one (the difference is the
  opening), and the time `file::open_path` took, measured in a local build.

## Benchmarks

`cargo bench -p <crate>` runs a crate's `criterion` benches (CI only builds
them: `cargo bench --workspace --no-run`). Median times on the reference
machine:

| Bench | Crate | Time |
|-------|-------|------|
| Topology of 1 M triangles | geom | 57 ms (580 ms before 14.4) |
| BVH of 1 M triangles | geom | 270 ms (348 ms before 14.4) |
| Triangulating 1 M triangles (Phong) | geom | 54 ms |
| Subdividing 250 k quads into 1 M | geom | 32 ms (74 ms before 14.4) |
| Evaluating 10 000 objects after an edit | eval | 1.2 ms |
| Evaluating the next frame, 1 000 of 10 000 animated | eval | 2.2 ms |
| Subdivision Surface, 3 levels over a 6 × 6 × 6 cube | eval | 3.4 ms |
| Extrude of a 20-letter text | eval | 2.6 ms |
| Cloner of 1 000 cubes after an edit | eval | 0.13 ms |
| Extracting 10 000 objects (orbiting / selection toggled) | render | 1.1 / 1.3 ms |
| Component overlay of 1 M triangles (points / edges / polygons) | render | 14.8 / 10.4 / 8.2 ms, once per change |
| Loading / saving a 50 MB `.g4d` (in memory) | io | 0.45 s / 1.5 s |
| Demo scene: building its BVH / rendering 320 × 180 at 4 samples | raytrace | 0.24 ms / 4.0 ms |
| A thousand falling cubes (one frame) | sim | see `cargo bench -p grid4d-sim` |

## What changed in 14.4

- **Topology without hashing** (`grid4d_geom::Topology::build`): edges were
  found through a hash map of point pairs and orientation through a hash
  set of directed edges — 580 ms for a million triangles, paid the first
  time a mesh enters a component mode or a modelling tool. Corners are now
  grouped by their edge's lower point (a counting sort) and an edge's
  earlier corners found among that point's few: 57 ms, the same edges in
  the same order (a test compares both ways on every test shape and on
  random, non-manifold meshes). Subdivision, which builds topology, went
  from 74 to 32 ms.
- **BVH build** (`grid4d_geom::Bvh::build`): the triangles' boxes now move
  with them while the median split reorders them, instead of being looked
  up through an index on every comparison: 348 → 270 ms (the first click on
  a large mesh).

## Not measured

- The graphics card's own time per frame: the readout's parts are processor
  time; the time between frames includes the card's work only when it is
  the bottleneck (it was not, in any scenario above).
- Scenes filling the whole view with a million triangles: the test driver
  could not move the view (the window took no mouse input from it), so the
  sphere was measured at its starting size.
- macOS and Linux: the budget is promised on the reference machine; other
  systems are expected to be in the same range and are checked when the
  packages are tried (14.6).
