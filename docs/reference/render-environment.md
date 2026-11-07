# The Sky and the Environment

> Sources (read 2026-11-07): Cinema 4D manual —
> [Sky object](https://help.maxon.net/c4d/r21/us/html/OSKY.html) (an infinitely
> large sphere around the scene; with several, only the topmost one in the
> Object Manager renders) and
> [Physical Sky](https://help.maxon.net/c4d/2027/en-us/Content/html/OSKYSHADER.html)
> (sun, sky and atmosphere from a date, time and place; the sun is a light
> whose position, colour and brightness follow the time of day; a turbidity
> setting for haze). The daylight model is the analytic sky model published
> by Preetham, Shirley and Smits (1999), implemented from the paper's
> formulas in our own code (decision D33). Written in our own words.
>
> Covers implementation-plan step **16.3**.

## Purpose

Light from everything around the scene: an open daylight sky with a sun for
outdoor shots, or a photographed surrounding (an HDR panorama) that lights
and reflects in products — and seeing it in the viewport while working.

## Parameters

### Sky (a new object, Create ▸ Light ▸ Sky)

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Sun Direction | From Object / Elevation and Azimuth | From Object | From Object: the sun shines along the object's −Z (rotate it like an Infinite light); otherwise the two angles below |
| Elevation | −10 … 90° | 45° | the sun's height above the horizon |
| Azimuth | 0 … 360° | 135° | its compass direction, measured from −Z towards +X |
| Turbidity | 1.7 … 10 | 3 | haze: 2 a clear mountain sky, 3 a clear day, 6 – 10 hazy; the sky whitens and the sun reddens and dims |
| Intensity | 0 … 10 000 % | 100 % | brightness of sky and sun together |
| Sun | on / off | on | the sun as a light and a disc |
| Sun Size | 0.1 … 20 × | 1 × | the disc's angular size relative to the real sun (0.53°); larger gives softer shadows |
| Sun Intensity | 0 … 10 000 % | 100 % | the sun's brightness relative to the model's |
| Visible to Camera | on / off | on | camera rays that miss everything see the sky; off: the background stays transparent, the sky still lights and reflects |
| Group | text | empty | its light group ([`render-lights-area.md`](render-lights-area.md)) |

- Below the horizon the sky is a dim, uniform ground colour (our choice: the
  model's horizon colour × 0.3) so the lower half is never black.
- The sun's colour comes from the model (warmer near the horizon, below
  −2° it is gone).

### Environment (a new object, Create ▸ Light ▸ Environment)

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Texture | one of the document's textures | none | an equirectangular (latitude-longitude, 2 : 1) picture, best an HDR / EXR; linear colour space |
| Intensity | 0 … 10 000 % | 100 % | brightness |
| Rotation | −360 … 360° | 0° | turns the picture about the vertical axis (the object's own rotation turns it too) |
| Visible to Camera | on / off | on | as for the Sky |
| Group | text | empty | its light group |

- Without a texture (or while it loads, or if it cannot be read) the
  Environment is a uniform grey of its intensity × 0.5.
- The picture's centre column looks along −Z; `u` runs eastwards (towards
  +X), `v` from the zenith (top row) to the nadir.

## Behaviour

- **One at a time**: the topmost visible (in the renderer, for renders; in
  the editor, for the viewport) Sky or Environment in the Object Manager
  lights the scene; others are ignored (shown greyed in their Attribute
  Manager note, "Not used: another sky or environment is above it").
- **Lighting**: it surrounds the scene at infinity; its position and scale do
  not matter, only its rotation (and Elevation / Azimuth). Rays that miss
  everything pick up its colour; it is sampled as a light — the sun as a
  small disc, the sky and environment **by importance** (a luminance table
  over the picture, so a bright window or sun in an HDRI gives clean
  shadows) — with MIS like every light.
- **Auto Light** is off whenever a Sky or Environment lights the scene.
- **Reflections and glass** see it.
- **The viewport**: the view's background shows it (Gouraud and Quick
  shading; Constant and Lines keep the gradient) — the Sky drawn from the
  model per pixel, the Environment from its picture; surfaces get its
  **ambient light** from nine spherical-harmonic numbers made once per
  change (on a worker for pictures), and the Sky's sun lights and shadows
  like an Infinite light. View ▸ Filter ▸ Sky hides it per view.
- **Alpha**: a Sky or Environment visible to the camera fills the
  background, so the picture is opaque; with Visible to Camera off the
  background stays transparent and Alpha Channel saves the objects'
  coverage as before.
- **Animating**: Elevation, Azimuth, Turbidity, Intensity, Rotation animate.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `ObjectKind::Sky(Sky)`, `ObjectKind::Environment(Environment)`; `ObjectKindTag`s; defaults, params | `grid4d-scene` (`node.rs`, a new `sky.rs`) |
| The Preetham sky and sun (pure functions: direction → radiance; sun direction, colour) | `grid4d-eval::sky` (pure, no GPU), shared by the viewport and the tracer |
| The chosen surround at the frame (`EvaluatedScene::environment`) | `grid4d-eval` |
| Importance sampling (2D luminance table), sun disc, miss colour | `grid4d-raytrace` (a new `environment.rs`) |
| Background pass and SH ambient | `grid4d-render` (`sky.wgsl`, the mesh shader's ambient) |
| SH projection of pictures off the interface thread | `grid4d-ui` (`textures.rs`) |
| Kinds in the file (new `type` values: `sky`, `environment`) | `grid4d-io` `v5.rs`; refused in a v4 file |

Deliberate differences: no date, time, place or time zone (the sun is placed
by angles or by turning the object), no clouds, fog, rainbows, moon or stars;
no Hosek-Wilkie model unless its data's licence passes `cargo deny` (D33);
no spherical or cube-map pictures other than equirectangular; one
surrounding at a time.

## Acceptance criteria

- [ ] The Sky's zenith and horizon colours match the published model's
      values for test directions within 1 %; the sun reddens with lower
      elevation and higher turbidity.
- [ ] An Environment of a uniform picture lights a white diffuse ball to the
      picture's value (furnace test); a picture with one bright texel gives
      sharp shadows with little noise at low samples (importance sampling
      measured against uniform sampling).
- [ ] Rotation and the object's rotation turn the picture; the topmost one
      wins; Visible to Camera only changes the background.
- [ ] The viewport shows both as the background and as ambient light; idle
      frames allocate nothing; SH numbers are made once per change.
- [ ] Undoable, animatable, saved in v5; a v4 file with these kinds is
      refused.
- [ ] Reference pictures: a daylight scene, an HDRI-lit product.
