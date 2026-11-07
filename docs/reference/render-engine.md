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
objects), no depth of field or motion blur yet; no render-time anti-aliasing
filters other than a box.

## As built (15.6, materials and textures)

- `RenderScene::textured(evaluated, &Texturing)` reads the document's
  `Materials`, the decoded `Pictures` (by texture id; the interface hands
  over the full-size level, sharing its pixels), the render's camera and
  aspect and the camera objects; `RenderScene::new` keeps the plain colours.
- Every Material tag of an object is a layer; a hit reads them from the top
  down and takes the first that is solid there (its Opacity at or above the
  threshold); where none is, the ray goes on, and so do shadow rays.
- Coordinates: shape projections and UVs are made per triangle corner once
  per render (the tag's turn, tiling and offset baked in); Camera
  projections project the hit point through the tag's camera object, else
  the render's camera. Pictures are read bilinearly, repeating; no mip-maps
  (many samples per pixel do their work).
- Normal pictures bend the normal in the tangent frame
  (`grid4d_geom::uv::tangents`, stored per corner; per triangle for Camera
  projections); Bump pictures by the height's slope half a texel either way,
  through the triangle's change of position per unit of u and v.
- The surface: a matte part and a GGX highlight (Specular: 8 % × specular
  straight on), a glass part (Transmission × (1 − Metallic)) that reflects or
  refracts about a microfacet normal as often as the exact dielectric
  Fresnel term says, tinted by the Base Color, with total internal
  reflection; a clear coat (4 %, its own roughness) over both. A path that
  refracts continues on the other side; whether it enters or leaves is told
  by the triangle's front.
- Shadow rays pass cut-outs and are tinted by glass (Base Color ×
  transmission at each glass surface), so glass casts lighter, tinted
  shadows; caustics are not traced.
- Materials without transmission or coat render exactly as before (the
  reference picture of the demo scene did not change).
- Tests: channel reading, sampling, cut-outs, layers, glass, Camera
  projections through a camera object, normal and bump pictures, and a second
  reference picture (`textured_reference.png`: Box and UV projections, a cut
  out label layer, a bump map under a clear coat, glass).

## Acceptance criteria

- [x] The demo scene renders to a PNG through its camera.
- [x] Shadows follow each light's Shadow setting; the automatic light lights a
      scene without lights; luminance glows; the background is transparent.
- [x] Renders are repeatable, the same with one thread or many; reference-image
      tests compare a small render within a tolerance.
- [x] (15.6) Textures, layers, normal and bump pictures, cut-outs, glass and
      clear coats render; a textured reference picture guards them.
