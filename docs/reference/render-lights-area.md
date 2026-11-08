# Area lights, light transport, linking and groups

> Sources (read 2026-11-07): Cinema 4D manual — Light object,
> [Details tab](https://help.maxon.net/c4d/r21/us/html/OLIGHT-LIGHT_GROUP_DETAILS.html)
> (the Area type's *Area Shape*: disc, rectangle, sphere, cylinder and more;
> *Size X / Y / Z*; *Show in Render* makes the light's shape itself visible,
> *Show in Reflection* lets surfaces mirror it; a sample count and grain
> against visible sample patterns; an edge falloff angle for flat shapes);
> the reference's lights also carry Include / Exclude lists of objects they
> do or do not light (their Project tab; its page was not re-read, the
> behaviour below is our own); the
> [Multi-Pass](https://help.maxon.net/c4d/en-us/Content/html/DRENDERSETTINGS-RDATA_GROUP_MULTIPASS.html)
> page (lights rendered into passes of their own: none, all, or selected
> ones). Light groups as named sets of lights with a pass each are a common
> renderer feature; the name and design here are our own. Written in our own
> words.
>
> Covers implementation-plan steps **16.2** (built as 16.2a: light transport
> and emissive surfaces; 16.2b: area lights) and **16.4** (linking and
> groups). Builds on
> [`object-light.md`](object-light.md) and [`render-engine.md`](render-engine.md);
> the passes the groups feed are [`render-passes.md`](render-passes.md).

## Purpose

Soft, believable light from lights that have a size — a softbox, a window, a
strip, a bulb — and from glowing surfaces, rendered with little noise; and
control over which light touches which object, and how much of the picture
each set of lights makes, for compositing.

## Parameters

### The Light object, type **Area** (a fourth Type)

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Shape | Rectangle, Disc, Sphere, Tube | Rectangle | the emitting shape, in the light's own space: Rectangle and Disc lie in its XY plane facing −Z; Sphere is centred on it; Tube lies along its Z axis |
| Size X | 0.1 … 100 000 cm | 100 cm | width (Rectangle), diameter (Disc, Sphere, Tube) |
| Size Y | 0.1 … 100 000 cm | 100 cm | height (Rectangle), length (Tube) |
| Visible in Render | on / off | off | camera rays and reflections see the shape, at its brightness; off: it lights the scene but is unseen |
| Two-Sided | on / off | off | Rectangle and Disc emit from both faces (each at full brightness); off: only along −Z. Sphere and Tube emit all round |

- Colour, Intensity, Shadow, Group and the Include / Exclude list (below)
  work as for the other types. **Falloff** is not offered: an area light
  falls off with distance by itself, as real lights do.
- **Intensity is the light's power, independent of its size**: making a
  light larger softens its shadows and highlights without brightening the
  scene. 100 % is set so that a small area light lights a facing surface
  at Radius/Decay's default distance (500 cm) about as an Omni of 100 % with
  Inverse Square falloff does (our choice; the exact constant is fixed with
  a test in 16.2), so switching a light's type keeps the exposure close.
- **Shadow**: *None* lets its light through objects; any other setting casts
  shadows, always soft (their softness comes from the shape).

### Render Settings ▸ Sampling (added)

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Clamp Indirect | on / off | on | light found after a bounce is capped, as in 1.0 – 1.1 (fewer bright specks, slightly darker caustics); off: unbiased, noisier |
| Emissive Surfaces Light the Scene | on / off | on | surfaces with Emission are sampled as lights; off: they glow and reach others only through bounces, as in 1.1 |

### Linking and groups (every light; 16.4)

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Linking | Exclude, Include | Exclude | Exclude: the light lights everything but the listed objects; Include: only the listed objects |
| Objects | list of objects | empty | each entry counts with its children; added from a list of the document's objects (our choice: no dragging from the Object Manager yet), removed with ×; a deleted object shows as "(deleted)" and does nothing |
| Light Group | text | empty | the light's group: its light also goes into that group's pass (16.6) |

The Sky and Environment objects (16.3) have Group too.

## Behaviour

### Light transport (16.2)

- **Lights with a surface** — emitting triangles now, area lights (16.2b),
  the sky and environment (16.3) — are kept in **a table by power**; at
  every surface a path meets, **one** of them is chosen from it, a point on
  it sampled and a shadow ray tested. The material's own bounce can also
  meet such a light. Both ways are weighted by **multiple importance
  sampling** (the power heuristic), so small bright lights and large dim
  ones are both clean.
- **Omni, Spot and Infinite** lights keep their 1.1 meaning and sampling:
  point-like (or the small ball / half-degree disc of a soft shadow), never
  met by a ray, each sampled at every surface as before — MIS does not
  apply to lights a bounce cannot find. A scene without emitting surfaces
  therefore renders **exactly** as in 1.1 (the 1.1 reference pictures are
  unchanged); one with them gives the same picture once converged, with far
  less noise.
- **Emissive surfaces** (Emission × Emission Strength, including pictures on
  the Emission channel) are lights: each triangle's power is its area times
  its emission's brightness (the mean of four points, so a picture counts
  roughly; the light itself is read exactly where the sample lands). Objects
  hidden in the renderer emit nothing. Emission is two-sided, as before.
- **Glass and emitters**: shadow rays towards an emitting surface stop at
  glass (unlike shadow rays towards lights, which pass it tinted): light
  through glass from a surface is found by the paths that refract, so it is
  never counted twice.
- Each emitter sample costs a shadow ray at every surface of every path:
  per sample, a scene with emitters renders about half as fast again, and
  reaches a given cleanliness in about a third of the time (budget,
  `docs/performance.md`).

### Area lights (16.2)

- **In renders**: soft shadows and broad highlights from the shape; Visible
  in Render shows the shape (camera and reflections) at the brightness it
  emits; a one-sided shape seen from behind is dark.
- **In the viewport**: the shape is drawn as lines in the light's colour
  (orange when selected), with a short arrow along −Z for one-sided flat
  shapes. It lights the scene approximately: from the shape's point nearest
  each surface (a representative point), as bright as the matching Omni light
  with Inverse Square falloff but never brighter than right at its face, a
  flat shape by the cosine it is seen at. Its shadow map is cast from its
  centre, like an Omni light's (not softened by its size yet).
- **Sampling in renders** (16.2b, as built): flat shapes evenly over their
  area; a Sphere over the cone of directions it fills from the shading point
  (nothing of it hidden behind itself: almost no noise from a small bulb); a
  Tube over the half that faces the point.
- **Brightness** (as built): radiance = colour × intensity × π × (500 cm)² ÷
  the shape's area seen face on (a Sphere's disc, a Tube from the side).
- **Creating**: Create ▸ Light, then Type ▸ Area (16.2b; a menu entry of its
  own would need a command that creates a light of a given type, left for
  later).
- glTF has no area lights: export writes a one-sided flat one as the widest
  spot light (`KHR_lights_punctual`), the others as a point light, of the
  same colour and intensity; import is unchanged.

### Linking (16.4)

- An object a light does not light gets **none of its light and casts no
  shadow of it** (one list for both; our choice, simpler than the
  reference's per-entry switches).
- Linking applies to lights only; emissive surfaces, the Sky and the
  Environment light everything.
- The viewport honours linking in Gouraud shading, the key light's shadow
  map included. Evaluation gives each linked light a bit and each object (and
  clone, as the object it shows) a 64-bit mask of the linked lights that do
  not light it; the viewport and the renderer read the same masks. Our
  choice: the first 64 linked lights of a scene (in Object Manager order) are
  linked; any further linked light lights everything, in the viewport and in
  renders. Lights without linking need no bit.
- Light found by a bounce on a visible area light counts only if that light
  lights the surface the ray left.
- Merge keeps the links among the merged objects (others are dropped).

### Groups (16.4)

- A group is a name: every light (and Sky / Environment) with the same Light
  Group belongs to it. 16.4 stores and saves it; the passes come with 16.6. The Render Settings' passes list one pass per group in use
  (16.6); light from lights without a group, and emission seen directly, goes
  into a **Default** group pass, so the group passes add up to the beauty.

### Undo, files, takes

- Every parameter is undoable (`SetData` like other object parameters);
  Intensity, Size X / Y animate (`ObjectParam`). Takes override them like
  other parameters. Saved in `.g4d` v5 (`light_type: "area"`, `area`,
  `linking`, `group`; a version 4 file with them is refused / stripped, see
  [`app-format-v5.md`](app-format-v5.md)).

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `LightType::Area` with `AreaShape` and the area fields of `LightDetails`; `Linking { mode: LinkMode, objects }` and `group: String` on `ObjectKind::Light` (and `group` on `Sky`, `Environment`) | `grid4d-scene::light`, `node.rs`, `sky.rs` |
| Evaluated lights with shape and their link bit (`EvaluatedLight::link`), objects' and clones' `unlit` masks, resolved on every evaluation without allocating | `grid4d-eval` (`link_lights`) |
| Masks in the viewport: the three free `w`s of `InstanceRaw::normal` (24 bits each), `LightRaw::link` (112 bytes), the shadow pass's face uniform | `grid4d-render` |
| Linked lights in renders: `RenderScene::lit_by`, shadow rays passing objects a light does not light | `grid4d-raytrace` |
| Light table by power, NEE + MIS, emissive triangles, area shapes | `grid4d-raytrace` (`scene.rs`, `trace.rs`, a new `lights.rs`) |
| Viewport: representative point, size-softened shadow, light masks | `grid4d-render` (`mesh.wgsl`, `effects.rs`) |
| Shapes as overlay lines | `ui/panels/viewport/overlay.rs` |
| The Area fields, linking list, Group | `ui/panels/attributes.rs` |
| Clamp Indirect, Emissive Surfaces | `RenderSettings`, the Sampling page |

Deliberate differences: no cube, hemisphere, line or object-shaped area
lights, no per-light sample count or grain (the light table and MIS make
them unnecessary), no edge falloff angle (shapes emit like real diffusers),
no separate diffuse / specular switches per light.

## Acceptance criteria

- [x] A 1.1 document renders the same picture once converged; without
      emitting surfaces exactly the same (the 1.1 reference tests pass
      unchanged).
- [x] (16.2a) Sampling emitting surfaces is unbiased: a floor under a
      glowing panel gets the same light sampled or found by bounces alone,
      also behind a glass slab (no light counted twice), with a quarter of
      the noise at 16 samples; the material's direction pdf integrates to
      the share of bounces that reflect.
- [x] (16.2b) A small area light of every shape lights a plane as an Omni
      light with Inverse Square falloff does at 500 and 250 cm; a large one
      gives the same light farther away.
- [x] Each shape lights a plane as its geometry says (the inverse square
      for every shape, a one-sided light dark behind, two-sided lit both
      ways); resizing keeps the total light; visible lights are seen and,
      found by bounces too, light the same.
- [x] Emissive triangles light the scene; with the switch off the picture
      matches 1.1's behaviour.
- [x] A linked light lights only its objects (Include) or all but them
      (Exclude), children included; their shadows follow; dangling entries
      do nothing.
- [ ] Group passes add up to the beauty within rounding (16.6: the passes).
- [x] Area lights are drawn in the viewport and light it; idle frames stay
      allocation-free.
- [x] Every new parameter is undoable, saved in v5, refused or stripped in v4.
- [x] A reference picture with each shape guards the renderer
      (`area_reference.png`).
