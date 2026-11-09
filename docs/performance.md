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

## Textured scenes (Grid4D 1.1, step 15.9)

Measured on 2026-11-07 on the same machine, at the commit after `7bd87a2`.
The 1.0 numbers above were read from the running app; this time there was
no access to the screen, so the viewport's own work was timed without a
window: `crates/ui/tests/budget.rs` (ignored by default) loads the stress
scenes, uploads their pictures as the app does, and for each frame evaluates
the scene, extracts it with its materials and draws it into a viewport
target of 1290 × 900 pixels with 4× MSAA on the RTX 3050 (Vulkan), **waiting
for the GPU to finish every frame** — CPU and GPU never overlap, so these are
upper bounds of the viewport's part of a frame (what View ▸ Frame Time calls
Evaluation, Extraction and Drawing), and they are not comparable one to one
with the on-screen numbers above.

```text
cargo run --release --example stress -- target/stress
cargo test --release -p grid4d-ui --test budget -- --ignored --nocapture
```

| Scene | Plain | Textured (8 materials, 1024-pixel colour and normal pictures) | Budget |
|-------|-------|--------------------------------------------------------------|--------|
| 10 000 primitives, playing (`stress-10k`, Box projections) | 11.7 ms (86 fps), worst 12.7 ms | 13.7 ms (73 fps), worst 16.9 ms | ≥ 60 fps: met |
| One mesh of 1 000 000 triangles (`stress-1m`, its UVs) | 2.6 ms (383 fps), worst 2.9 ms | 2.9 ms (350 fps), worst 3.1 ms | ≥ 60 fps: met |

Textures cost about 2 ms a frame on 10 000 objects (more batches: one per
mesh shape, projection and material) and a tenth of a millisecond on the big
mesh (tangents are made once, with the mesh). The harness's own way to find a
GPU prefers a software rasteriser; the measurement picks the discrete GPU
itself, as the app does.

## Render time (Grid4D 1.2, step 16.1: before Phase 16)

Measured on 2026-11-07 on the same machine (all 16 threads), at the commit
after `a5f1317`, with the path tracer as Grid4D 1.1 ships it. These are the
numbers Phase 16 must beat or keep (spec `reference/render-sampling.md`):
renders without its new features must not get slower, and from step 16.7
adaptive sampling with the denoiser must reach the error of 256 samples
below in less time.

```text
cargo run --release -p grid4d-raytrace --example budget -- [scene …] [--pictures <folder>]
```

The scenes (`crates/raytrace/examples/budget.rs`):

- **demo** — the demo scene through its camera, 2 bounces (1.1's reference
  picture);
- **studio** — a product shot on a floor and backdrop: a glass ball, a
  brushed gold ring, a chrome cylinder, a coated red cube, a soft spot light
  and a glowing panel (which lights the scene only through bounces in 1.1),
  4 bounces;
- **interior** — a closed room lit by one small soft lamp, so almost all
  light has bounced, 4 bounces;
- **dense** — a ball of about a million triangles on a floor in the sun,
  2 bounces.

Build is evaluation, triangles and BVH (best of three); each render is the
best of two. The error is the root mean square difference of the 8-bit sRGB
values (0 – 255) from a 4096-sample render with another seed, at
320 × 180.

| Scene | Triangles | Build | 1280 × 720, 16 samples | 16 samples (320 × 180): time / error | 64 | 256 | The 4096-sample reference |
|-------|-----------|-------|------------------------|--------------------------------------|----|-----|---------------------------|
| demo | 908 | < 1 ms | 0.22 s | 15 ms / 1.87 | 57 ms / 0.96 | 0.23 s / 0.54 | 3.5 s |
| studio | 7 310 | 3 ms | 3.0 s | 0.19 s / 24.2 | 0.79 s / 13.6 | 3.2 s / 7.16 | 52 s |
| interior | 1 172 | < 1 ms | 6.8 s | 0.43 s / 17.8 | 1.7 s / 9.36 | 7.4 s / 4.86 | 117 s |
| dense | 998 002 | 0.46 s | 2.2 s | 0.15 s / 4.08 | 0.57 s / 2.05 | 2.2 s / 1.08 | 38 s |

What the numbers show:

- **Time is linear in samples** and the error halves with every four times
  the samples (16 → 64 → 256), as plain Monte Carlo does: no scene gets
  cleaner faster than that today.
- **studio** and **interior** are the noisy ones — the glass and metals, the
  panel found only by chance after a bounce, and a room lit almost entirely
  by bounced light. 256 samples still leave an error of 5 – 7 levels, the
  speckle visible at a glance; these are the scenes light sampling (16.2)
  and the denoiser (16.7) are for.
- **dense**: half a second goes to building the BVH of a million triangles
  (the median split); rays through it cost about as much as the studio's
  per sample. The SAH build (16.7) is measured against both.

### After step 16.2a (emitting surfaces sampled as lights)

Measured on 2026-11-08, same machine. Only **studio** has an emitting
surface (its panel); the other three scenes render exactly as before (same
pictures, same times within measuring noise). The reference picture of the
studio changed too: light from the panel after a bounce was capped before.

| Scene | 1280 × 720, 16 samples | 16 samples: time / error | 64 | 256 | The 4096-sample reference |
|-------|------------------------|--------------------------|----|-----|---------------------------|
| studio, 16.1 | 3.0 s | 0.19 s / 24.2 | 0.79 s / 13.6 | 3.2 s / 7.16 | 52 s |
| studio, 16.2a | 4.5 s | 0.29 s / 11.4 | 1.15 s / 6.19 | 4.8 s / 3.32 | 76 s |

A sample costs half as much again (one more shadow ray at every surface of
every path, towards the panel), but the error halves at every sample count:
1.1's 256-sample cleanliness (7.16) is now reached at 64 samples, in about a
third of the time (1.15 s against 3.2 s).

### After step 16.3 (Sky and Environment)

Measured on 2026-11-08, same machine. None of the four scenes has a Sky or
an Environment. With lights sampled directly, the last surface's bounce is
now followed one ray further for the light it meets (16.2 lost a little of
that light): **demo** renders exactly as before (same errors, same time);
**studio** costs about 5 % more for the same error.

| Scene | 1280 × 720, 16 samples | 16 samples: time / error | 64 | 256 | The 4096-sample reference |
|-------|------------------------|--------------------------|----|-----|---------------------------|
| studio, 16.2a | 4.5 s | 0.29 s / 11.4 | 1.15 s / 6.19 | 4.8 s / 3.32 | 76 s |
| studio, 16.3 | 4.5 s | 0.31 s / 11.4 | 1.24 s / 6.20 | 5.0 s / 3.32 | 81 s |

### After step 16.6 (render passes)

Measured on 2026-11-09, same machine, without passes (as every render
before 16.6): the pictures are bit for bit the same (same errors) and the
times are those of 16.3 within measuring noise (studio: 0.31 s / 11.4,
1.22 s / 6.20, 5.0 s / 3.32 at 16, 64 and 256 samples; demo 18 / 69 /
261 ms). Passes are gathered only when chosen.

### Step 16.7a (adaptive sampling, the surface-area BVH)

Measured on 2026-11-09, same machine, the same day for both builds of the
hierarchy (the machine was faster than at 16.6). The pictures are the same
(same errors); a render takes 23 – 43 % less time with the surface-area
build, which takes about twice as long to build for a million triangles
(the viewport's picking keeps the median build, made on first use):

| Scene | Build | 1280 × 720, 16 samples | 16 samples | 64 | 256 | 4096 |
|-------|-------|------------------------|------------|----|-----|------|
| demo, median | < 1 ms | 270 ms | 18 ms / 1.87 | 68 ms / 0.96 | 276 ms / 0.53 | 4.4 s |
| demo, SAH | 1 ms | 213 ms | 14 ms / 1.87 | 53 ms / 0.96 | 212 ms / 0.53 | 3.4 s |
| studio, median | 3 ms | 5.2 s | 0.32 s / 11.4 | 1.35 s / 6.20 | 5.5 s / 3.32 | 86 s |
| studio, SAH | 4 ms | 3.1 s | 0.19 s / 11.4 | 0.79 s / 6.20 | 3.1 s / 3.32 | 52 s |
| interior, SAH | 1 ms | 3.7 s | 0.22 s / 17.8 | 0.91 s / 9.36 | 3.8 s / 4.86 | 60 s |
| dense, median | 430 ms | 2.5 s | 0.15 s / 4.08 | 0.64 s / 2.05 | 2.5 s / 1.08 | 41 s |
| dense, SAH | 787 ms | 1.6 s | 0.10 s / 4.08 | 0.41 s / 2.05 | 1.6 s / 1.08 | 26 s |

Adaptive sampling from 16 samples up to 1024 (time / error, and the
samples a pixel got on average):

| Scene | Threshold 0.02 | Threshold 0.01 |
|-------|----------------|----------------|
| demo | 81 ms / 0.69 (34) | 135 ms / 0.42 (57) |
| studio | 8.7 s / 2.08 (669) | 11.8 s / 1.87 (927) |
| interior | 15.2 s / 2.66 (1020) | 15.1 s / 2.66 (1024) |
| dense | 1.8 s / 1.23 (131) | 3.9 s / 0.80 (340) |

Where much of the picture is clean and cheap to trace, adaptive sampling
pays: **demo** beats the 256-sample error (0.42 against 0.53) in 60 % of the
time. Where the whole picture is noisy (studio's and interior's indirect
light) nearly every pixel goes on to the maximum and it costs what uniform
sampling costs (no overhead measured). In **dense** the pixels that go on
are the expensive ones (the million-triangle object; the empty background
stops first), so a sample costs more on average and it is about even with
uniform sampling for the same error. The noisy scenes are the denoiser's
(16.7b).

### Step 16.7b (the denoiser)

Measured on 2026-11-09, same machine (time / error against the 4096-sample
reference; the time includes gathering the guides and denoising):

| Scene | 256 samples (1.1's mark) | 16 + denoiser | 64 + denoiser | 128 + denoiser | Adaptive 0.02 + denoiser |
|-------|--------------------------|---------------|---------------|----------------|--------------------------|
| demo | 210 ms / 0.53 | 35 ms / 1.52 | 78 ms / 0.80 | 137 ms / 0.62 | 100 ms / 0.67 |
| studio | 3.2 s / 3.32 | 0.26 s / 4.32 | 0.87 s / 2.84 | — | 8.9 s / 1.52 |
| interior | 3.6 s / 4.86 | 0.29 s / 3.96 | 1.0 s / 2.54 | — | 15.3 s / 1.51 |
| dense | 1.58 s / 1.08 | 0.15 s / 2.21 | 0.46 s / 1.36 | 0.87 s / 1.12 | 1.8 s / 0.81 |

The noisy scenes are where it pays: **interior** at 16 samples denoised is
cleaner than 1.1's 256 in a twelfth of the time, **studio** at 64 in a
quarter. **Demo** (little noise to remove; its error is mostly anti-aliasing
at the edges, which the denoiser leaves) reaches 1.1's mark with adaptive
sampling alone (0.42 in 135 ms). **Dense** comes close: 128 samples denoised
give 1.12 in 55 % of the time 256 take for 1.08 (not quite: the step's
acceptance is met on three scenes of four). Gathering the guides and
denoising cost about 25 ms at 320 × 180.

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
| Unwrapping 16 k quads over hills in one piece (LSCM) | geom | 1.8 s |
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
