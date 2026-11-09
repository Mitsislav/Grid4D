# Subsurface scattering and displacement

> Sources (read 2026-11-07): Cinema 4D manual —
> [Sub-polygon Displacement](https://help.maxon.net/c4d/en-us/Content/html/5786.html)
> (a height picture moves the surface at render time; the object is
> subdivided internally to a level set on the material's displacement page —
> each level doubles the divisions, so a cube at level 8 becomes hundreds of
> thousands of polygons — without changing the object itself; with a
> Subdivision Surface the displacement is applied after it). Subsurface
> scattering as a principled material channel (weight, colour, a scattering
> distance per colour) is common to physically based materials; the random
> walk is a published method, implemented in our own code. Written in our
> own words.
>
> Covers implementation-plan step **16.10**, which may move to 1.3 if the
> phase runs long (risk R5). Builds on
> [`material-channels.md`](material-channels.md) and
> [`material-textures.md`](material-textures.md).

## Purpose

Materials light passes into — skin, wax, marble, milk, leaves — that glow
softly at thin edges and soften shadows; and surfaces with real relief —
bricks, rocks, carved wood — whose silhouettes and shadows show it, from a
picture rather than modelling.

## Parameters (new channels of the Material)

### Subsurface

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Subsurface | 0 … 1 (a channel: value or picture) | 0 | how much of the diffuse part scatters inside instead of reflecting at the surface |
| Subsurface Color | linear RGB (a channel) | the Base Color's (a check box "Use Base Color", on) | the colour light takes inside |
| Radius | RGB, 0 … 1000 cm each | 1.0, 0.2, 0.1 cm | how far each colour travels on average before leaving; red farther, as in skin |
| Radius Scale | 0 … 1000 × | 1 × | multiplies the three radii |

### Displacement

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Displacement | a height picture (texture slot only) | none | what moves the surface |
| Amount | −1000 … 1000 cm | 1 cm | how far white moves along the normal |
| Mid Level | 0 … 1 | 0.5 | the picture value that stays in place (below it moves inwards) |
| Subdivision | 0 … 8 | 3 | each level splits every triangle in four before moving (level 3: 64 per triangle) |

Render Settings ▸ Sampling gains **Triangle Budget** (1 … 100 million
triangles, default 20 million): an object whose subdivision would exceed its
share is subdivided less, with a note in the render's information line.

## Behaviour

### Subsurface

- Rendered by a **random walk**: light entering the surface (by the
  Subsurface part of the diffuse lobe) takes steps inside the closed mesh,
  scattering in random directions with mean free paths from the radii, until
  it leaves through the surface somewhere (often elsewhere) and picks up
  light there. Thin parts glow when lit from behind.
- Meshes must be **closed** to look right; an open mesh lets the walk leave
  through the hole (it then counts as leaving there). Overlapping or
  touching objects are separate media.
- Subsurface and Metallic: metals do not scatter (Metallic wins as it does
  for the diffuse part); Transmission (glass) wins over Subsurface.
- The **viewport** shows Subsurface as a softened diffuse term (wrapped
  lighting tinted by Subsurface Color); renders show the real thing.
- Material preview balls show it.

### Displacement

- At **render time**, each object with a displaced material is subdivided
  (flat subdivision, splitting triangles in four per level; on top of a
  Subdivision Surface's own render levels), then every point moves along its
  smoothed normal by (picture − Mid Level) × Amount, reading the picture
  through the Material tag's projection like every channel. Points shared
  by faces move once, so the surface stays closed; UV seams may open small
  cracks where the picture differs across them (our note in the manual).
- Shadows, silhouettes and reflections show the relief; the shading normal
  comes from the displaced surface (a Normal or Bump picture on top still
  bends it).
- Several Material tags: displacement comes from the **top tag that has one**
  covering the point (layers above without Displacement leave it).
- The **viewport** keeps showing Displacement as a bump (the height
  picture's slope), as the plan decides; Current State to Object does not
  bake it (it is a render-time effect).
- Memory and time grow by four per level; the budget guards against
  runaway levels.

### Files, undo, interchange

- New material fields, undoable like the other channels; saved in `.g4d` v5
  (stripped from a v4 file). glTF: `KHR_materials_volume`'s attenuation
  distance maps to Radius on import (approximation) and back on export; glTF
  has no displacement (not exported). OBJ `.mtl`: `disp` reads and writes
  the Displacement picture.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `Material::{subsurface, subsurface_color, subsurface_radius, subsurface_scale}` (16.10a), the displacement fields (16.10b) | `grid4d-scene` (`material.rs`) |
| Flat subdivision + displacement at render time | `grid4d-geom` (`displace.rs`: `subdivide_flat`, `displacement_directions`, `displace`) and `grid4d-raytrace` (`scene.rs`: `Build::displace`; `Texturing::displacement_budget`, `RenderScene::split_less`) |
| Random walk | `grid4d-raytrace` (`shade.rs`: `Lobe::Subsurface`, `Material::exit`; `subsurface.rs`: `Medium`, `walk`; `trace.rs`: `light_at`, shared by surfaces and walk exits) |
| Viewport: wrapped diffuse, displacement as bump | `grid4d-render` (`mesh.wgsl`) |
| The channels' rows | `ui/panels/attributes.rs` (material page) |
| `.g4d` v5 fields; glTF and OBJ | `grid4d-io` |

As built, subsurface (16.10a), our choices: Subsurface and Subsurface
Color are values without a picture (no texture slot: the viewport's material
already binds ten pictures, and a picture of either is rare; a later
step may add them); glTF's `KHR_materials_volume` is not mapped yet. The
walk picks the colour it flies by as often as that colour still carries
light, scatters isotropically, ends at 256 steps or by Russian roulette
after 8; the medium comes from the colour by van de Hulst's relation
inverted (a thick white slab gives back its colour within a few %). Where it
comes out the light is gathered as by a white matte surface and the path
goes on from there; that light is not capped by Clamp Indirect (it belongs
to the surface the light went into). Light that travels far comes back in
one colour at a time, so low sample counts show coloured speckles; the
denoiser cleans them. The viewport wraps the matte lighting round to the
shadow side by up to half (Subsurface × 0.5), in the Base Color, not the
Subsurface Color.

As built, displacement (16.10b), our choices: the height is the **Bump
picture** — Displace in Renders turns it from shading into geometry
(Blender's way) — so there is no separate Displacement slot; **Amount** is
the Bump Height, Mid Level and Subdivision are new fields. The renderer
displaces an object by the top Material tag that displaces, through its
coordinates (Camera projections do not displace); every layer's coordinates
are split alike. Points at the same place move along one direction (the
normals of the triangles meeting there, weighed by their angles); normals
are found again from the moved surface, smoothed across edges up to 60°.
The budget is spent object by object in the order they are built; an
object that would exceed what is left is split less (counted, not yet shown
in the information line). OBJ `disp` reads into the Bump slot with
Displace on, and is written beside `bump` for displaced materials.

Deliberate differences: no adaptive (screen-space) subdivision, no vector
displacement, no displacement in the viewport, no separate subsurface
methods (only the random walk), no subsurface anisotropy.

## Acceptance criteria

- [x] A sphere with Subsurface lit from behind lets light through at its
      thin edge; a white slab with a large radius approaches its diffuse
      albedo (energy test); Subsurface 0 renders exactly as before.
- [x] A plane displaced by a stripes picture shows ridges of the right
      height (measured by rays); Mid Level keeps mid-grey in place; a
      closed mesh stays closed after displacement.
- [x] The budget limits the triangles made; level 0 moves the original
      points only.
- [x] The viewport shows displacement as bump and subsurface as soft
      diffuse; no viewport allocation in steady frames.
- [x] Every field undoable, saved in v5, stripped from v4; OBJ `disp` both
      ways.
- [x] Reference pictures: a wax ball, a displaced brick pavement.
