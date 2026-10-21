# Viewport effects: shadows and ambient occlusion

> Sources (read 2026-10-21): Cinema 4D manual —
> [Options menu](https://help.maxon.net/c4d/r21/us/html/45030.html) (Shadows and SSAO
> are viewport options, set per view, among the enhanced display options),
> [Viewport effects settings](https://help.maxon.net/c4d/s22/us/html/DBASEDRAW-BASEDRAW_GROUP_EFFECTS.html)
> (viewport shadows always come from a shadow map, whatever shadow type the light
> has; whether a light's shadow is hard or soft is set on the light — Shadow Maps
> and Area both show soft in the viewport; a bigger shadow map gives sharper,
> more precise shadows at a cost; SSAO is a quick approximation of ambient
> occlusion that darkens inner corners, edges and holes, with Radius, Depth
> Range, Power, Samples and Blur; it differs from the rendered result) and
> [Light object, General tab](https://help.maxon.net/c4d/r21/us/html/OLIGHT-LIGHT_GROUP_GENERAL.html)
> (Shadow: None casts no shadow, then soft, hard and area shadows). Written in our
> own words.
>
> Covers implementation-plan step **7.6**.

## Purpose

Judging how objects sit in the scene while working: where they touch the floor,
what shades what, and the contact darkening in corners — without rendering.

## Behaviour

### The light's Shadow

| Value | In the viewport |
|-------|-----------------|
| None (default) | no shadow |
| Shadow Maps (Soft) | a soft-edged shadow |
| Raytraced (Hard) | a sharp-edged shadow |
| Area | a soft-edged shadow (like Shadow Maps) |

The final renderer (Phase 8) will tell them apart further.

### Options ▸ Shadows (per view, off by default)

- The **key light** casts shadows: the first visible light in the Object Manager
  whose Shadow is not None. Other lights light without shadows (our limitation:
  one shadow-casting light per view).
- It works in **Gouraud Shading** (with or without lines) — the modes lit by the
  scene's lights; Quick and Constant Shading and the line modes have none.
- Every type casts: a **Spot** within its cone, an **Infinite** light over the
  whole scene, an **Omni** light in every direction.
- Light and camera markers cast no shadows.
- Shadows follow the light and the objects as they move or animate.

### Options ▸ SSAO (per view, off by default)

- Darkens the shading where surfaces are close to each other: inner corners,
  creases, where an object meets the floor, holes. It darkens the soft light
  fully and the lights' direct light in part (60 %), so it shows in scenes lit
  mostly by their lights (our choice; the reference's version darkens the
  picture like a post effect).
- Works in Gouraud and Quick Shading (with or without lines).
- Fixed settings for now: a radius of about 30 cm, sixteen samples, a light blur.
  The grid is not treated as geometry.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `LightShadow` in `ObjectKind::Light` (file: omitted when None) | `grid4d-scene::light`, `grid4d-io::v3` |
| `EvaluatedLight::shadow` | `grid4d-eval` |
| `ViewEffects { shadows, ssao }` in `FrameData`; the scene's bounds for fitting the shadow map | `grid4d-render::frame` |
| Shadow map (2048², up to six layers), its pass and sampling | `grid4d-render` (`gpu.rs`, `shaders/shadow.wgsl`, `mesh.wgsl`) |
| SSAO textures per view target; normal/depth pre-pass, occlusion, blur | `grid4d-render` (`target.rs`, `shaders/ssao.wgsl`) |
| Options ▸ Shadows / SSAO, Shadow in the Attribute Manager | `viewport/mod.rs`, `panels/attributes.rs` |

Deliberate differences: one shadow-casting light; no shadow map size, shadow
attenuation, transparency in shadows or SSAO settings yet; no "Enhanced OpenGL"
switch (the effects have their own).

## Acceptance criteria

- [x] A light with a Shadow casts it in a view with Shadows on (Spot, Infinite,
      Omni), soft or hard as set; markers cast none.
- [x] SSAO darkens contact areas and corners in a view with SSAO on.
- [x] Both are per view and off by default; drawing stays allocation-free in
      steady state; the Shadow parameter is saved, undoable and older files open
      unchanged.
