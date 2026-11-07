# Material channels

> Sources: the reference's [Material Tag](https://help.maxon.net/c4d/en-us/Content/html/TTEXTURE.html)
> page (read 2026-11-06: a material is placed on an object by a tag that holds
> the projection; layers of tags) and the principled, metallic-roughness model
> that glTF 2.0 and most renderers share (public specifications). The channel
> set, names and layout are Grid4D's own (decision D26), not the reference's
> channel list. Written in our own words, 2026-11-06.
>
> Covers implementation-plan step **15.4** (scene model, Material Manager and
> Attribute Manager pages, preview balls); used by 15.5 (viewport), 15.6 (path
> tracer) and 15.8 (glTF / OBJ). Extends [`manager-material.md`](manager-material.md).
> Step 15.4a (2026-11-06) built the model, the file format and the
> Attribute Manager's page; 15.4c (2026-11-07) the preview balls.

## Purpose

A material describes a whole surface — painted wood, brushed metal, glass,
a glowing screen — with one fixed set of channels that the viewport, the
path tracer, glTF and OBJ all read the same way. Each channel is a value or
an image.

## Channels

Every channel has a **value** and may have a **texture** (an image from the
document's texture table, see [`material-textures.md`](material-textures.md))
that replaces the value per point of the surface.

| Channel | Value | Default | What it does |
|---------|-------|---------|--------------|
| Base Color | linear RGB | 80 % grey | the colour of a dielectric, the reflection colour of a metal |
| Metallic | 0 – 1 | 0 | 0 dielectric, 1 metal; between: a blend |
| Roughness | 0 – 1 | 0.5 | 0 mirror-like, 1 matte |
| Specular | 0 – 1 | 0.5 | a dielectric's reflection strength (0.5 = 4 %, the usual) |
| Normal | — (texture only) | none | a tangent-space normal map; *Strength* 0 – 2 (1) |
| Bump | — (texture only) | none | a height map; *Height* in cm (0.1); used only without a Normal texture |
| Emission | linear RGB × *Intensity* | black, 1 | light the surface gives off |
| Opacity | 0 – 1 | 1 | cut-out: below *Threshold* (0.5) the surface is not there |
| Transmission | 0 – 1 | 0 | light passing through (glass); *IOR* 1 – 3 (1.5) |
| Clearcoat | 0 – 1 | 0 | a thin varnish over the base; *Coat Roughness* 0 – 1 (0.03) |

And one switch: **Two-sided** (off) — both sides of each polygon are
shaded alike; off, the back of a polygon shows the same surface flipped
(no difference for closed objects).

## A channel's texture

| Setting | Range | Default | Effect |
|---------|-------|---------|--------|
| Texture | a texture of the document, or none | none | the image |
| Tiling U, V | > 0 | 1, 1 | repeats of the image across the UV square |
| Offset U, V | any | 0, 0 | shift, in image widths |
| Range | min, max | 0, 1 | maps the image's 0 – 1 onto this range (scalar channels) |
| Strength | 0 – 1 | 1 | blend from the value (0) to the texture (1) |

Colour channels multiply the value by the texture when the texture is
greyscale and replace it otherwise. The texture's colour space (sRGB or
linear) belongs to the texture, not the channel; a texture used on a
data channel (Roughness, Metallic, Normal, …) is read as linear unless the
user set it otherwise.

## Behaviour

- **Material Manager:** each material shows a **preview ball** rendered by
  the path tracer in the background (one at a time, small, cached until the
  material or one of its textures changes). Dropping an image file on a
  material's Attribute Manager channel adds it to the texture table and links
  it.
- **Attribute Manager:** the material's page has one group per channel
  (Base, Reflection, Normal / Bump, Emission, Opacity, Transmission, Coat),
  each with its value and a texture row (choose, clear, tiling, offset,
  range, strength). Every edit is one undo step (live while dragging).
- **Old documents:** a 1.0 material's colour, roughness, metallic and
  emission become Base Color, Roughness, Metallic and Emission; everything
  else is at its default, so it renders as it did.
- **Missing textures** render as the channel's value in the path tracer and
  as a magenta checker in the viewport, with a warning in the Texture
  Manager.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `Material` with channels (`Channel<T>`: value + optional `TextureSlot`) | `grid4d-scene` `material.rs` |
| Edits | `SceneEdit::SetMaterials` as today (the table whole) |
| Viewport shading | `grid4d-render` (15.5): the instance's surface plus texture bindings |
| Path tracer | `grid4d-raytrace` `shade.rs` (15.6) |
| File | `.g4d` v4 `materials` (see [`app-format-v4.md`](app-format-v4.md)) |
| glTF / OBJ | 15.8: metallic-roughness, `KHR_texture_transform`, `.mtl` maps |

Deliberate differences from the reference: one fixed principled set instead
of switchable channels and reflectance layers; no procedural textures (Phase
22's nodes will compile into this set); no displacement (Phase 16).

## As built (15.4a)

- `Material` keeps its four 1.0 fields as the Base Color, Roughness,
  Metallic and Emission values (so everything that read them still does)
  and gains the others; textures are `TextureSlot`s per `Channel`.
- Load… beside a channel picks a file, adds it to the texture table (Linear
  for data channels) or reuses the texture already linked to it, and sets
  the slot — one undo step for both.
- Emission's Strength multiplies its colour in the viewport and the path
  tracer already.

## As built (15.4c)

- `ui::previews::MaterialPreviews` (editor state): one worker thread renders
  `render_preview(material)` — a 96-pixel ball (radius 50 cm, seen from the
  front, a key light upper left and a fill lower right, 16 samples, 2
  bounces) inside a dim grey surrounding sphere that gives metals something
  to reflect; a second render without the surrounding gives the ball's
  coverage, so the background stays transparent.
- One picture at a time: `update` (each frame, from `apply_pending`) starts
  the first material whose picture shows another state of it; an edit during
  a render is rendered afterwards, so a drag ends with its last value. The
  old picture stays until the new one arrives; a painted ball stands in
  before the first.
- The picture is keyed by the whole `Material` (its texture slots
  included); the texture pictures themselves reach previews when the path
  tracer reads textures (15.6).
- Shown in the Material Manager, while dragging a material, and at the top
  of the material's Attribute Manager page (48 points); Material tag icons
  in the Object Manager stay painted (16 points).

## As built (15.5a)

- The viewport draws an object's top Material tag (the last one) when its
  material has a picture on a channel or cuts something out; other objects
  draw and batch exactly as before. Base Color, Metallic, Roughness,
  Specular, Emission and Opacity (cut-out, also in Hidden Line) read their
  pictures; Normal, Bump, Transmission and Clearcoat follow in 15.5b.
- Pictures reach the GPU when they arrive (`ViewportRenderer::upload_texture`,
  called by the interface's glue, never per frame): 8-bit sRGB, 8-bit linear
  or half-float RGBA, the whole mip chain, sampled with trilinear filtering,
  8× anisotropy and repeat. A picture is grey when every texel is; on a
  colour channel it multiplies the value.
- Each textured material has one bind group (its channels' tiling, offset,
  range and strength in a uniform, a picture per channel, white where there
  is none), made again only when its slots or a picture change.
- Specular sets a dielectric's reflection (8 % × specular; 4 % at 0.5).
- A frame of 10 000 objects in textured and projected materials allocates
  nothing (`render/tests/steady_frame.rs`).

## As built (15.5b)

- Normal pictures are tangent-space, +Y up the picture (glTF's way); the
  vertices of meshes drawn with a Normal or Bump picture carry tangents
  (`grid4d_geom::uv::tangents`: along increasing `u`, shared by vertices at
  one place with one normal and UV, so smooth surfaces show no seams; `w`
  says which side is up the picture, flipped again for mirrored objects).
  *Strength* scales the bend (with the slot's strength).
- Bump pictures tilt the normal against the height's slope across the pixel
  (screen-space derivatives, no tangents); *Height* is in centimetres; used
  only without a Normal picture.
- The GPU memory of all pictures is held under 1 GiB, and no side over the
  device's limit: a picture that would pass either goes up from the first
  mip level that fits.
- Not yet: Transmission and Clearcoat in the viewport; cut-outs in the
  shadow map and the ambient occlusion (they shade as solid).

## Acceptance criteria

- [x] A 1.0 document's materials open with the same four values and render
      the same.
- [x] Each channel's value and texture settings save, load and undo.
- [ ] Glass (Transmission 1, IOR 1.5) and brushed metal (Metallic 1, a
      Normal texture) render as expected in the path tracer; the viewport
      shows the normal map and opacity cut-out.
- [x] Preview balls update after an edit without stalling the interface.
