# Render passes and multi-layer OpenEXR

> Sources (read 2026-11-07): Cinema 4D manual —
> [Render Settings ▸ Multi-Pass](https://help.maxon.net/c4d/en-us/Content/html/DRENDERSETTINGS-RDATA_GROUP_MULTIPASS.html)
> (one render saves separate layers — lighting components, depth, normals,
> object masks — to adjust in a compositor without rendering again; the
> physical passes Direct / Indirect Diffuse and Specular add up to the full
> picture; lights may get layers of their own: none, all, or selected ones)
> and the Save page (a separate multi-pass file; formats that hold layers,
> OpenEXR among them). The OpenEXR file layout follows the public OpenEXR
> specification (multi-part is not used: one part, channels named
> `layer.R` …), written by our own code (decision D35). Written in our own
> words.
>
> Covers implementation-plan step **16.6**. Builds on
> [`render-animation.md`](render-animation.md) (the single-layer OpenEXR of
> 1.0) and [`render-picture-viewer.md`](render-picture-viewer.md).

## Purpose

Handing a compositor the pieces of a picture — how much light is direct or
bounced, diffuse or shiny, from which group of lights, which pixels belong to
which object — so the look can be adjusted after rendering, from one render.

## Parameters (Render Settings ▸ Passes, a new page)

| Pass | Channels | Content |
|------|----------|---------|
| Beauty | R G B A | the picture, as today (always written) |
| Alpha | A | coverage (in PNG passes as a grey picture) |
| Depth | Z | distance from the camera along its axis, cm (32-bit float; nothing hit: the largest float) |
| Normal | X Y Z | the shading normal in world space, −1 … 1 |
| Position | X Y Z | the world position of the first hit, cm |
| Albedo | R G B | the surface's base colour (glass: its tint; metals: their colour) |
| Emission | R G B | light emitted by surfaces seen, and lights visible in render |
| Background | R G B | the Sky / Environment seen directly |
| Diffuse Direct, Diffuse Indirect | R G B | light reflected diffusely, straight from lights / after bounces |
| Specular Direct, Specular Indirect | R G B | glossy reflections and clear coat, straight from lights / after bounces |
| Transmission | R G B | light through glass |
| Object ID | ID | the object hit by most of the pixel's samples (below) |
| Material ID | ID | its material, likewise |
| Light groups | R G B each | one pass per group in use, and Default ([`render-lights-area.md`](render-lights-area.md)); the first 16 group names met get passes, lights of further groups count as Default |

| Setting | Values | Default | Effect |
|---------|--------|---------|--------|
| (each pass) | on / off | off (Beauty on) | written or not |
| PNG Passes | Picture Only, One File per Pass | Picture Only | with Format PNG: only the picture, or each pass next to it as `<name>_<pass>.png` (data passes mapped: depth over the range found from near white to far dark, normals × 0.5 + 0.5, positions over their range, IDs as distinct colours, Alpha as grey) |

## Behaviour

- **Lighting passes add up**: Emission + Background + Diffuse Direct +
  Diffuse Indirect + Specular Direct + Specular Indirect + Transmission =
  Beauty (RGB) within rounding, and the light-group passes add up to the
  same. Lighting passes are premultiplied by coverage like the beauty.
- **Data passes** (Depth, Normal, Position, Albedo) are averaged over the
  pixel's samples at the first surface seen (through cut-outs; glass shows
  its own surface). **IDs** are not averaged: each pixel takes the ID of a
  running majority vote over its samples (the ID more than half of them
  met, whenever there is one; 0 for none), so edges never blend two IDs
  into a third. The object ID is the object's place in the Object
  Manager's order + 1 at the time of the render; the file names them
  (below). Clone instances take their Cloner's ID. The material ID is the
  top Material tag's material's place in the Material Manager + 1 (0: no
  material).
- **Clamp Indirect** applies to the indirect passes as to the beauty.
- **OpenEXR**: with Format OpenEXR every pass chosen goes into the same
  file, one part, scanlines, ZIP compression: the beauty as `R G B A` (so
  every reader shows it first), each other pass as `<Pass>.<channel>`
  (`Depth.Z`, `Normal.X`, `DiffuseDirect.R`, `Group.Key.R` …). Colour
  passes are half floats, data and ID passes 32-bit floats. A string
  attribute `grid4d/objects` (and `grid4d/materials`) lists `id = name`
  lines. Our writer only; files are checked by the `exr` crate in tests.
- **Animations** write one file per frame as now (numbered), passes inside
  each, or for PNG one file per pass per frame.
- **Picture Viewer**: a Pass menu lists the passes of the shown render
  (rendered passes stay in memory for the history entry); data passes are
  shown mapped as in PNG; Save As… saves the shown pass as PNG.
- **Render View** shows the beauty only (our choice).
- **Cost**: each chosen pass costs its memory (a 4K picture: about 130 MB
  for all of them) and a little time; nothing is gathered for passes that
  are off.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `RenderSettings::passes` (a set of `Pass`), `png_passes` | `grid4d-scene` (`render.rs`) |
| Accumulating passes per pixel beside the beauty, IDs by vote | `grid4d-raytrace` (`passes.rs`: a `Layout` of the chosen passes' floats per pixel, `Split` per sample; `Progressive::with_passes`; `Image::passes`, `PassLayer`) |
| Light categories | `grid4d-raytrace` (`shade.rs`: each `Bounce` names its lobe; `trace.rs`: direct light split into its diffuse share) |
| Multi-channel OpenEXR (half and float channels, string attributes) | `grid4d-io` (`picture.rs`: `save_exr_layers`, `ExrChannel`; `save_exr` writes through it, byte for byte as before) |
| Passes in the history, the Pass menu | `grid4d-ui` (`rendering.rs`, `panels/picture_viewer.rs`) |
| The Passes page | `grid4d-ui` (`panels/render_settings.rs`) |

Deliberate differences: no blend channels, shadow or ambient-occlusion passes
(our light passes add up instead), no motion vectors, no Cryptomatte-style ID
mattes (Phase 16's "not in"), no separate multi-pass file path (the passes
live in the one EXR), no Photoshop or TIFF layers.

## Acceptance criteria

- [x] The lighting passes, and the group passes, each add up to the beauty
      within 1e-3 of its value on test scenes (glass, coat, emission, sky).
- [x] A multi-layer EXR from Grid4D is read back by the `exr` crate with
      every channel, the right types and the name attributes.
- [x] Depth, Normal and Position match the geometry of a test scene; IDs
      separate objects and materials, clones take their Cloner's.
- [x] PNG One File per Pass writes the expected names, also per frame.
- [x] The Picture Viewer shows each pass; a render without passes costs
      what it cost in 1.1 (time measured).
- [x] Settings are undoable, saved in v5, stripped from a v4 file.
