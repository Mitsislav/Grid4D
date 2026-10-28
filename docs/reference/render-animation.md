# Rendering animations, OpenEXR

> Sources (read 2026-10-28): Cinema 4D manual —
> [Render Settings ▸ Output](https://help.maxon.net/c4d/r21/us/html/DRENDERSETTINGS-RDATA_GROUP_OUTPUT.html)
> (Frame Range, From, To, Frame Step) and
> [Render Settings ▸ Save](https://help.maxon.net/c4d/r21/us/html/DRENDERSETTINGS-RDATA_GROUP_SAVE.html)
> (Format, including OpenEXR for high dynamic range; Name: how frame numbers are
> written into file names, e.g. a name, four digits and the extension, or the
> number after a dot), and the Picture Viewer page for renders of several
> frames. Written in our own words.
>
> Covers implementation-plan step **8.5**, the last of Phase 8. Builds on
> [`render-settings.md`](render-settings.md) (8.2) and
> [`render-picture-viewer.md`](render-picture-viewer.md) (8.3).

## Purpose

Rendering an animation — every frame of a range, each saved as its own
numbered file — and keeping the full range of light in OpenEXR files for
compositing.

## Parameters (Render Settings ▸ Save)

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Format | PNG (8 bit) / OpenEXR (16-bit float) | PNG | PNG: sRGB, 8 bits per channel. OpenEXR: linear light, half floats (values above 1 kept), ZIP-compressed, alpha premultiplied |
| Name | `Name0000` / `Name.0000` / `Name_0000` | `Name0000` | how frame numbers join the file name: `shot0012.png`, `shot.0012.png`, `shot_0012.png`; four digits at least, more when needed, a minus sign for negative frames |

Frame Range, From, To and Frame Step (Output page, 8.2) say which frames.

## Behaviour

- **Render to Picture Viewer** (Shift+R) renders every frame of the Render
  Settings' range in order, at the Frame Step. With **Current Frame** (or a
  range of one frame) it is one picture, as before.
- **Numbered files**: when the range gives more than one picture, each finished
  frame is saved at once as `<File><number>.<extension>` by the Name setting.
  A single picture is saved without a number. A typed `.png` or `.exr` on File
  is replaced by the Format's extension.
- **In the Picture Viewer**: one History entry for the whole animation; it
  shows the frame being rendered as it sharpens, then the last frame. The
  progress bar covers all frames; the information line counts them ("frame 12,
  13 of 91"). Stop ends the animation; frames already finished stay saved.
- **OpenEXR**: the renderer's own linear values, before the conversion to
  screen colours, so bright highlights keep their strength (half floats keep
  values up to 65504). Alpha Channel on: channels R, G, B, A (premultiplied);
  off: R, G, B over black.
- **Save As…** keeps saving the shown picture as PNG (our choice: the viewer
  keeps 8-bit pictures; OpenEXR comes from the render itself).
- The Render View stays a single frame at the current time.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `ImageFormat`, `FrameNaming` in `RenderSettings`; `output_file(document, frame, numbered)` | `grid4d-scene` (`render.rs`) |
| `format` and `naming` in the `render` entry (additive, omitted while default) | `grid4d-io` (`v3.rs`) |
| `save_exr` / `exr_bytes`: scanline OpenEXR, HALF, ZIP (16 lines) | `grid4d-io` (`picture.rs`); `flate2`, `half` |
| Frames in order in one worker; each finished frame sent for saving | `grid4d-ui` (`rendering.rs`, `file.rs`) |
| Format and Name on the Save page; Frames note on the Output page | `grid4d-ui` (`panels/render_settings.rs`) |

Deliberate differences: no movie formats, no multi-pass or layered files, no
16/32-bit choice for EXR (half floats only), no RAM player.

## Acceptance criteria

- [x] A range renders every frame at its step and saves each as a numbered file.
- [x] One frame is saved without a number; Name and Format pick the file name.
- [x] OpenEXR files hold the linear values (a decoder reads them back), with or
      without alpha.
- [x] Stop keeps the frames already saved; the History shows one entry.
- [x] Files from 8.2 (without Format and Name) still load with the defaults.
