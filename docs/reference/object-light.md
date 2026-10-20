# Light objects and viewport lighting

> Sources (read 2026-10-21): Cinema 4D manual — Light object
> [General tab](https://help.maxon.net/c4d/r21/us/html/OLIGHT-LIGHT_GROUP_GENERAL.html)
> (Color; Intensity in percent, above 100 % allowed; Type: Omni shines in every
> direction like a bulb, Spot shines along the light's axis in a round cone,
> Infinite only has a direction — like the sun, its position does not matter;
> further types: Square Spot, Area, Parallel, Parallel Spot, IES) and
> [Details tab](https://help.maxon.net/c4d/r21/us/html/OLIGHT-LIGHT_GROUP_DETAILS.html)
> (Use Inner: off gives a hard-edged cone; Inner Angle: full brightness inside it,
> fading to nothing at the Outer Angle, which bounds the cone; both 0–175°;
> Falloff: none, linear from full to nothing at Radius/Decay, inverse square
> reaching the set intensity at Radius/Decay and decreasing with the square of
> the distance beyond); the
> [Display menu](https://help.maxon.net/c4d/r21/us/html/5860.html) (Gouraud
> Shading uses the scene's lights, Quick Shading the automatic light instead).
> Written in our own words.
>
> Covers implementation-plan step **7.4** (light types, viewport lighting,
> shading from materials). Shadows are 7.6.

## Purpose

Lighting the scene in the viewport the way it is lit for rendering — where the
lights are, their type, colour and strength — and seeing the materials'
roughness, metalness and luminance there.

## Behaviour

### The Light object

| Parameter | Values | Default | Effect |
|-----------|--------|---------|--------|
| Color | linear RGB | white | the light's colour |
| Intensity | 0 … 10 000 % | 100 % | brightness; colour × intensity |
| Type | Omni, Spot, Infinite | Omni | see below |
| Use Inner (Spot) | on / off | on | off: the whole cone at full brightness, a hard edge |
| Inner Angle (Spot) | 0 … 175° | 0° | full brightness inside it |
| Outer Angle (Spot) | 0 … 175° | 30° | the cone's opening (the full angle between opposite sides); brightness fades from the inner to the outer angle |
| Falloff (Omni, Spot) | None, Linear, Inverse Square | None | how brightness changes with distance |
| Radius/Decay | > 0 cm | 500 cm | Linear: full at the light, nothing at this distance; Inverse Square: exactly the set intensity at this distance, brighter nearer, dimmer farther (with the square of the distance) |

- **Omni** shines in every direction from its position.
- **Spot** shines from its position along its −Z axis, in a cone. (Grid4D is
  right-handed: objects look along −Z — cameras, Target tags, *Selected Object as
  Camera* — so lights do too.)
- **Infinite** shines along its −Z axis everywhere, like the sun: its position
  and falloff do not matter.
- Defaults are our choice where the reference states none. Square Spot, Area,
  Parallel, IES, shadows, visible light, "No Illumination", "Ambient
  Illumination" and the per-light diffuse/specular switches are later.

### In the viewport

- **Gouraud Shading** (with or without lines) lights the scene with its **light
  objects** — those visible in the viewport (a light hidden in the editor does not
  light it). A little ambient light keeps the sides that no light reaches
  readable, and a dim studio reflection (brighter overhead) keeps metals from
  turning black under a single light (our choices; the reference describes
  neither).
- **When the scene has no visible lights**, the **default light** lights it: a
  light from over the viewer's shoulder that moves with the view.
- **Quick Shading** always uses the default light and ignores the scene's lights.
  **Constant Shading** stays unlit.
- Lights are drawn as before (a small sphere in their colour); a **Spot** also
  shows its cone (outer angle) and an **Infinite** light an arrow along its
  direction, in the light colour, orange when selected.

### Shading from materials

An object's material (its own or inherited, see `manager-material.md`) sets how
it reflects light; objects without one use the material defaults:

- **Color**: the diffuse colour.
- **Roughness** 0 … 1: the size of highlights — small and sharp near 0, broad and
  faint near 1.
- **Metallic** 0 … 1: metals have no diffuse colour; their highlights take the
  colour instead of white.
- **Luminance** (emission): added to the surface, whatever the light — in every
  lit and constant mode.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `LightType`, `LightDetails` in `ObjectKind::Light` | `grid4d-scene::light` |
| Light parameters in the file (`.g4d` v3, additive: omitted at their defaults) | `grid4d-io::v3` |
| glTF `KHR_lights_punctual` point / spot / directional both ways | `grid4d-io` |
| `EvaluatedScene::lights` (world position and direction, colour × intensity); `Drawable::surface` (roughness, metallic, emission) | `grid4d-eval` |
| `FrameData` lights → a storage buffer (grows geometrically); shading in `mesh.wgsl` | `grid4d-render` |
| Type / Details parameters | `ui/panels/attributes.rs` |
| Spot cones, Infinite arrows | `viewport/overlay.rs` |

## Acceptance criteria

- [x] A scene with lights is lit by them in Gouraud Shading (type, colour,
      intensity, cone, falloff); without lights by the default light; Quick
      Shading always by the default light.
- [x] Roughness, metallic and luminance change the viewport's shading.
- [x] Light parameters are saved (older files open unchanged), undoable, and
      travel through glTF.
- [x] The light list and its GPU buffer are rebuilt without per-frame allocation.
