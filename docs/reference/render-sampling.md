# Adaptive sampling, the denoiser and a faster BVH

> Sources (2026-11-07): this spec follows no page of the reference: the
> settings are the ones path tracers commonly offer (a minimum and maximum
> number of samples per pixel with an error threshold deciding where more
> are spent; a denoiser guided by extra passes), named plainly (R8). The
> methods are published ones, implemented in
> our own code (decisions D27, D36): adaptive sampling from the difference
> of two half-sample estimates; an edge-avoiding à-trous wavelet filter
> (Dammertz et al., 2010) guided by albedo, normal, depth and variance; a
> binned surface-area-heuristic BVH build. Written in our own words.
>
> Covers implementation-plan step **16.7**. Builds on
> [`render-settings.md`](render-settings.md) and the passes of
> [`render-passes.md`](render-passes.md) (albedo, normal, depth).

## Purpose

Clean pictures in reasonable time: spend samples where the picture is still
noisy rather than everywhere, remove the noise that is left without
smearing edges or textures, and trace every ray faster.

## Parameters (Render Settings ▸ Sampling)

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Adaptive | on / off | off (new documents: on, with Samples 128) | off: every pixel gets Samples, as in 1.1 |
| Samples | 1 … 4096 | 16 | without Adaptive: samples per pixel; with it, the **maximum** |
| Minimum Samples | 1 … 4096 | 16 | with Adaptive: every pixel gets these first (kept ≤ Samples) |
| Noise Threshold | 0.001 … 0.5 (kept in thousandths) | 0.01 | with Adaptive: a pixel stops once its estimated error, relative to the square root of its brightness, is below this; smaller is cleaner and slower |
| Denoise | Off, Final Picture, Final and Render View | Off (new documents: Final Picture) | runs the denoiser on the finished picture (and on the Render View's pictures once each pixel has 4 samples) |
| Denoise Strength | 0 … 100 % (whole percent) | 100 % | blends the denoised picture with the noisy one |

Old documents keep Adaptive and Denoise off, so they render as before; new
documents start with both on (our choice; the template's settings).

## Behaviour

### Adaptive sampling

- Samples are added in passes (as today's progressive render). Each pixel
  keeps, beside the sum of all its samples, the sum of its odd-numbered
  ones. Its error estimate is the difference between the two averages
  (colour channels added up) divided by 0.0001 + the square root of its
  brightness (R + G + B) — roughly how visible noise is — or the difference
  in coverage, whichever is larger.
- Decisions are taken at Minimum Samples (rounded up to even), then each
  time half as many again have been added (at least 4; always even, so the
  halves are equal). The estimate is itself noisy, so it is averaged over
  the pixel's 3 × 3 neighbourhood (a stopped pixel keeps its last one), and
  a pixel stops when that average is below the threshold for it and its
  eight neighbours (so pixels are not stopped by luck). A stopped pixel is
  never sampled again; the render ends when every pixel stopped, or at
  Samples.
- Deterministic: the passes and which pixels continue depend only on the
  scene, the settings and the seed, never on threads (rows still run in
  parallel; decisions are taken between passes).
- The Picture Viewer's information line shows the average samples per pixel
  and how many pixels reached the maximum; a **Samples per Pixel** entry
  (a heat map, dark blue to yellow) can be shown in its Pass menu; it is
  never saved.
- Progress counts pixel samples; a pixel that stops counts its remaining
  samples as done, so the bar jumps ahead as areas come clean.

### The denoiser

- Runs after the last pass (and per frame of an animation, before saving):
  five rounds of an **à-trous** filter (a 5 × 5 B3-spline kernel with holes
  doubling each round: 1, 2, 4, 8, 16 pixels) on the lighting, each
  neighbour weighted by how alike it is in normal (cosine to the power 64),
  depth (relative to how fast the depth changes across the pixel), albedo
  and brightness (within 4 standard deviations of the pixel's remaining
  noise; the noise is filtered along, so later rounds trust cleaner
  values). Lighting is filtered **divided by albedo** (colours below 0.02
  are not divided) and multiplied back, so textures stay sharp; the noise
  is measured in the same units.
- The guides — albedo, normal and depth averaged over the samples that met
  a surface — are gathered during the render beside the picture (not
  returned as passes unless asked for), and each pixel's noise comes from
  the two halves of its samples, as for adaptive sampling.
- Pixels that met nothing (the background, a Sky or Environment seen
  directly) are left as they are and never mixed with surfaces.
- Alpha is not filtered; data passes (depth, normal, IDs) are never
  denoised; light-group and lighting passes are filtered with the
  picture's own weights, so they still add up (to within rounding).
- The picture before denoising is kept as a pass named **Noisy**: in the
  EXR as `Noisy.R G B`, with PNG passes as `<name>_Noisy.png`, in the
  Picture Viewer as *Before Denoising* (our choice, for compositors who
  prefer their own).
- Glossy reflections and what is seen through glass are guided by the
  mirror's or glass's own surface, so detail in them is softened (a known
  limit of first-hit guides).
- Pure Rust, in `grid4d-raytrace`, no new dependency; parallel by rows.
- Open Image Denoise remains an optional later build (D27).

### A faster BVH

- `grid4d_geom::Bvh` gains a **binned SAH** builder (16 bins per axis,
  leaves of up to 4 triangles); the path tracer uses it, the viewport's
  picking keeps the current median build unless the SAH build is not
  slower to build for it (measured, 16.7: about twice as slow for a million
  triangles, so picking keeps the median build). Ray results are identical
  in what they hit (same nearest hit; ties go to the lower polygon index).
  Traversal visits the nearer child box first.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `RenderSettings::{adaptive, min_samples, threshold, denoise, denoise_strength}` | `grid4d-scene` (`render.rs`) |
| Two half sums per pixel, active pixel list, passes | `grid4d-raytrace` (`trace.rs`: `Progressive`) |
| The à-trous filter guided by albedo / normal / depth / variance | `grid4d-raytrace` (`denoise.rs`: `denoise(&mut Image, strength)`, `Guides`; `Progressive::with_denoising` gathers them, `Image::guides`) |
| Binned SAH build | `grid4d-geom` (`bvh.rs`: `BvhSplit`, `Bvh::build_with`) |
| The Sampling page; the samples heat map | `grid4d-ui` |

Deliberate differences: no separate anti-aliasing filter settings, no
per-object sample counts, no machine-learned denoiser in the default build.

## The budget (measured in 16.1, re-measured in 16.7)

`docs/performance.md` ▸ *Render time* holds the reference scenes and their
numbers measured before Phase 16: time to render each at fixed samples, and
the error against a 4096-sample reference at those samples. 16.7 must reach
the error of 1.1's 256 samples in less time on every scene, and render
without Adaptive or Denoise no slower than 1.1 (the SAH build makes it
faster).

## Acceptance criteria

- [x] Adaptive off renders exactly as 1.1; on, a flat-lit plane stops near
      Minimum Samples while edges get more.
- [x] Renders with Adaptive are repeatable and identical with 1 thread or
      many.
- [ ] On each reference scene, Adaptive + Denoise reaches 1.1's 256-sample
      error (RMSE against the 4096-sample reference) in less time — on
      three of the four (interior, studio denoised; demo with adaptive
      sampling); dense comes within 4 % of it in 55 % of the time
      (`docs/performance.md`).
- [x] The denoiser keeps a checker texture's edges (albedo guidance) and a
      cube's edges (normal and depth guidance) within a set error of the
      reference.
- [x] The SAH BVH finds the same nearest hits as the median build on random
      rays over the test shapes, and the demo render is faster.
- [x] Settings are undoable, saved in v5, stripped from a v4 file.
