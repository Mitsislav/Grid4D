# The final renderer: what a render shows

> Sources (read 2026-10-21): Cinema 4D manual —
> [Light object](https://help.maxon.net/c4d/r21/us/html/OLIGHT.html) (a scene
> without lights is lit by the automatic light; the Auto Light option of the
> render settings switches it off),
> [Light object, General tab](https://help.maxon.net/c4d/r21/us/html/OLIGHT-LIGHT_GROUP_GENERAL.html)
> (Shadow None casts no shadow; soft, hard and area shadows),
> [Light object, Details tab](https://help.maxon.net/c4d/r21/us/html/OLIGHT-LIGHT_GROUP_DETAILS.html)
> (cones, falloff) and the Basic tab's *Visible in Renderer* and the
> Subdivision Surface's *Subdivision Renderer* (already in Grid4D since 4.7).
> Written in our own words.
>
> Covers implementation-plan step **8.1**: the rendering itself, as a library
> with an example program. Render Settings (8.2), the Picture Viewer (8.3) and the
> Render View (8.4) put it in the application.

## Purpose

Turning the scene into a finished picture: materials and lights as they would
look for real — soft shadows, light bouncing between surfaces, reflections —
where the viewport only approximates them.

## Behaviour

**What is rendered**

- Objects **visible in the renderer** (Basic tab) and enabled; an object hidden
  in the renderer hides its children, like in the editor.
- Each object's **render-time geometry**: a Subdivision Surface at its
  *Subdivision Renderer* level; primitives as their mesh, smoothed by their Phong
  angle; hidden polygons left out, as in the viewport (our choice).
- Light and camera objects are not visible themselves.
- The picture is seen through a **camera**: the view's, or a camera object's —
  perspective (vertical field of view) or parallel.
- The background is black and **transparent**: each pixel's alpha is how much
  of it objects cover.

**Lighting**

- Every light visible in the renderer lights the scene as in the viewport: its
  type, colour × intensity, cone and falloff.
- **Shadows** follow the light's Shadow setting: *None* lets its light through
  objects; *Raytraced (Hard)* gives sharp shadows; *Shadow Maps (Soft)* and
  *Area* soft ones — the light is then treated as a small ball (5 cm across for
  Omni and Spot lights, half a degree wide for an Infinite light; our choice).
- **Without lights** (none visible in the renderer) the **automatic light**
  lights the scene: from over the camera's shoulder, without shadows.
- **Light bounces**: light reflected by surfaces lights other surfaces (two
  bounces by default), so floors brighten what stands on them and colours bleed.
- **Luminance** makes a surface glow; it lights others only through bounces.

**Materials** (as in the viewport, physically): the colour is diffuse;
*Roughness* the size of highlights and the blur of reflections; *Metallic*
makes reflections take the colour and removes the diffuse part.

**Quality**: every pixel averages several samples (16 by default) spread over
it, which smooths edges and the noise of soft shadows and bounces. Light that
arrives after a bounce is capped, so a bounce landing on a sharp highlight does
not leave bright specks (our choice: slightly darker caustics, far less noise).

**Repeatable**: the same scene and settings always give the same picture,
however many processor cores render it.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| Evaluation for the renderer (`Purpose::Render`: render visibility and levels) | `grid4d-eval` |
| The path tracer: `RenderScene` (world-space triangles in one BVH, materials, lights), `RenderCamera`, `RenderOptions`, `render` → `Image` (linear RGBA) and 8-bit sRGB | new crate `grid4d-raytrace` |
| Tiles in parallel | `rayon` |
| Example: render a `.g4d` (or the demo scene) to PNG | `crates/raytrace/examples/render.rs` |

Deliberate differences: no environment or sky (bounces reach only other
objects), no textures, transparency, refraction, depth of field or motion blur
yet; no render-time anti-aliasing filters other than a box.

## Acceptance criteria

- [x] The demo scene renders to a PNG through its camera.
- [x] Shadows follow each light's Shadow setting; the automatic light lights a
      scene without lights; luminance glows; the background is transparent.
- [x] Renders are repeatable, the same with one thread or many; reference-image
      tests compare a small render within a tolerance.
