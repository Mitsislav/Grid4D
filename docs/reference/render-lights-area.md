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
> Covers implementation-plan steps **16.2** (light transport, emissive
> surfaces, area lights) and **16.4** (linking and groups). Builds on
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
| Mode | Exclude, Include | Exclude | Exclude: the light lights everything but the listed objects; Include: only the listed objects |
| Objects | list of objects | empty | each entry counts with its children; dropped from the Object Manager or picked; deleted objects drop out (a dangling entry does nothing) |
| Group | text | empty | the light's group: its light also goes into that group's pass (16.6) |

The Sky and Environment objects (16.3) have Group too.

## Behaviour

### Light transport (16.2)

- At every surface a path meets, **one light is chosen** from a table
  weighted by each light's power (lights, emissive triangles, the sky or
  environment), a point on it is sampled and a shadow ray tested; the
  material's own reflection direction can also reach a light. Both ways are
  weighted by **multiple importance sampling** (the power heuristic), so
  small bright lights and large dim ones are both clean.
- Omni, Spot and Infinite lights keep their 1.1 meaning: point-like (or the
  small ball / half-degree disc of a soft shadow), never seen by the camera.
  A document from 1.1 renders the **same picture once converged**; the noise
  differs (the 1.1 reference pictures are compared within their tolerance,
  or written again with the reason in the test).
- **Emissive surfaces** (Emission × Emission Strength, including pictures on
  the Emission channel, averaged per triangle for the table) are lights:
  their triangles join the table by power. Objects hidden in the renderer
  emit nothing.

### Area lights (16.2)

- **In renders**: soft shadows and broad highlights from the shape; Visible
  in Render shows the shape (camera and reflections) at the brightness it
  emits; a one-sided shape seen from behind is dark.
- **In the viewport**: the shape is drawn as lines in the light's colour
  (orange when selected), with a short arrow along −Z for one-sided flat
  shapes. It lights the scene approximately: as a point light at the shape's
  nearest point to each surface (a representative point), its shadow-map
  shadow softened by the size.
- **Creating**: Create ▸ Light ▸ Area Light, or Type ▸ Area on any light.
- glTF has no area lights: export writes a spot light (`KHR_lights_punctual`)
  of the same colour and intensity in its place, with a note; import is
  unchanged.

### Linking (16.4)

- An object a light does not light gets **none of its light and casts no
  shadow of it** (one list for both; our choice, simpler than the
  reference's per-entry switches).
- Linking applies to lights only; emissive surfaces, the Sky and the
  Environment light everything.
- The viewport honours linking in Gouraud shading (a per-object light mask;
  the 64 first lights of a scene can be linked — beyond that all light
  everything in the viewport, the render stays exact).

### Groups (16.4)

- A group is a name: every light (and Sky / Environment) with the same Group
  belongs to it. The Render Settings' passes list one pass per group in use
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
| `LightType::Area(AreaLight { shape, size, visible, two_sided })`, `LightLinking { mode, objects }`, `group: String` | `grid4d-scene::light` |
| Evaluated lights with shape and linking (object sets resolved per render) | `grid4d-eval` (`EvaluatedScene::lights`) |
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

- [ ] A 1.1 document renders the same picture once converged (reference
      tests within tolerance, or rewritten with a stated reason).
- [ ] NEE + MIS is unbiased: a white furnace test (a diffuse sphere inside a
      uniformly emitting sphere) converges to the expected value; a small and
      a large area light each converge to their analytic irradiance on a
      plane.
- [ ] Each shape lights a plane as its geometry says (a disc's centre value,
      a sphere's inverse square, a one-sided light dark behind, two-sided lit
      both ways); resizing keeps the total light.
- [ ] Emissive triangles light the scene; with the switch off the picture
      matches 1.1's behaviour.
- [ ] A linked light lights only its objects (Include) or all but them
      (Exclude), children included; their shadows follow; dangling entries
      do nothing.
- [ ] Group passes add up to the beauty within rounding.
- [ ] Area lights are drawn in the viewport and light it; idle frames stay
      allocation-free.
- [ ] Every new parameter is undoable, saved in v5, refused or stripped in v4.
- [ ] A reference picture with each shape guards the renderer.
