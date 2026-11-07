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
| Adaptive | on / off | off (new documents: on) | off: every pixel gets Samples, as in 1.1 |
| Samples | 1 … 4096 | 16 | without Adaptive: samples per pixel; with it, the **maximum** |
| Minimum Samples | 1 … 4096 | 16 | with Adaptive: every pixel gets these first (kept ≤ Samples) |
| Noise Threshold | 0.001 … 0.5 | 0.01 | with Adaptive: a pixel stops once its estimated error, relative to its brightness, is below this; smaller is cleaner and slower |
| Denoise | Off, Final Picture, Final and Render View | Off (new documents: Final Picture) | runs the denoiser on the finished picture (and on the Render View's passes once each pixel has 4 samples) |
| Denoise Strength | 0 … 100 % | 100 % | blends the denoised picture with the noisy one |

Old documents keep Adaptive and Denoise off, so they render as before; new
documents start with both on (our choice; the template's settings).

## Behaviour

### Adaptive sampling

- Samples are added in passes (as today's progressive render). Each pixel
  keeps two sums of alternate samples; their difference, scaled by the
  pixel's brightness (with a floor for dark pixels, and spread over a 3 × 3
  neighbourhood so isolated pixels are not stopped by luck), is its error
  estimate.
- After Minimum Samples, a pass adds samples only to pixels whose error is
  above the threshold; the render ends when none is, or at Samples.
- Deterministic: the passes and which pixels continue depend only on the
  scene, the settings and the seed, never on threads (rows still run in
  parallel; decisions are taken between passes).
- The Picture Viewer's information line shows the average samples per pixel
  and how many pixels reached the maximum; a **Samples** pass (a heat map of
  samples per pixel) can be shown there.
- Progress: the bar estimates from the pixels still active.

### The denoiser

- Runs after the last pass (and per frame of an animation, before saving):
  five rounds of an **à-trous** filter (a 5 × 5 B3-spline kernel with holes
  doubling each round) on the lighting, each neighbour weighted by how alike
  it is in albedo, normal and depth and by the pixel's variance (noisy
  pixels lean on their neighbours more). Lighting is filtered **divided by
  albedo** and multiplied back, so textures stay sharp.
- Alpha is not filtered; data passes (depth, normal, IDs) are never
  denoised; light-group and lighting passes are denoised each the same way
  when Denoise is on, so they still add up (to within the filter's
  rounding).
- The beauty without denoising can still be saved: the EXR gets a
  `Noisy.R G B` layer when Denoise is on (our choice, for compositors who
  prefer their own).
- Pure Rust, in `grid4d-raytrace`, no new dependency; parallel by rows.
- Open Image Denoise remains an optional later build (D27).

### A faster BVH

- `grid4d_geom::Bvh` gains a **binned SAH** builder (16 bins per axis,
  leaves of up to 4 triangles); the path tracer uses it, the viewport's
  picking keeps the current median build unless the SAH build is not
  slower to build for it (measured, 16.7). Ray results are identical in
  what they hit (same nearest hit; ties broken the same way).

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `RenderSettings::{adaptive, min_samples, threshold, denoise, denoise_strength}` | `grid4d-scene` (`render.rs`) |
| Two half sums per pixel, active pixel list, passes | `grid4d-raytrace` (`trace.rs`: `Progressive`) |
| The à-trous filter guided by albedo / normal / depth / variance | `grid4d-raytrace` (a new `denoise.rs`) |
| Binned SAH build | `grid4d-geom` (`bvh.rs`) |
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

- [ ] Adaptive off renders exactly as 1.1; on, a flat-lit plane stops near
      Minimum Samples while a soft shadow's edge gets more.
- [ ] Renders with Adaptive are repeatable and identical with 1 thread or
      many.
- [ ] On each reference scene, Adaptive + Denoise reaches 1.1's 256-sample
      error (RMSE against the 4096-sample reference) in less time.
- [ ] The denoiser keeps a checker texture's edges (albedo guidance) and a
      cube's edges (normal guidance) within a set error of the reference.
- [ ] The SAH BVH finds the same nearest hits as the median build on random
      rays over the test shapes, and the demo render is faster.
- [ ] Settings are undoable, saved in v5, stripped from a v4 file.
