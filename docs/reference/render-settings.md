# Render Settings

> Sources (read 2026-10-21): Cinema 4D manual —
> [Render Settings](https://help.maxon.net/c4d/r21/us/html/DRENDERSETTINGS.html)
> (a window opened from the Render menu: pages listed on the left, the page's
> settings on the right; the settings belong to the scene),
> [Output](https://help.maxon.net/c4d/r21/us/html/DRENDERSETTINGS-RDATA_GROUP_OUTPUT.html)
> (presets, width and height, Lock Ratio, frame range: Manual, Current Frame,
> All Frames, Preview Range; From, To, Frame Step; the number of frames),
> [Save](https://help.maxon.net/c4d/r21/us/html/DRENDERSETTINGS-RDATA_GROUP_SAVE.html)
> (Save, File — a bare name or a relative path is taken from the scene's
> folder —, Format, Alpha Channel) and
> [Options](https://help.maxon.net/c4d/r21/us/html/DRENDERSETTINGS-RDATA_GROUP_OPTIONS.html)
> (Auto Light: the built-in light of a scene without lights). Written in our own
> words.
>
> Covers implementation-plan step **8.2**. The Picture Viewer (8.3) renders with
> these settings and saves the picture; frame ranges and numbered files are 8.5.

## Purpose

Saying how the scene is rendered — picture size, quality, which frames, where
the picture goes — once, with the document, instead of every time.

## Parameters

**Output**

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Preset | list | (matches the size) | sets Width and Height to a common size; *Custom* when they match none |
| Width, Height | 1…16384 px | 1280 × 720 | the picture's size |
| Lock Ratio | on/off | off | changing one side changes the other, keeping the proportions |
| Frame Range | Current Frame / All Frames / Manual | Current Frame | which frames an animation render renders |
| From, To | frames | 0, 0 | the frames of a Manual range (To never before From) |
| Frame Step | 1…10000 | 1 | renders every n-th frame of the range |
| Frames | read only | — | how many pictures the range gives |

**Save**

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Save | on/off | on | a finished render is saved to File |
| File | path | empty | where; a name without a folder, or a relative path, is taken from the document's folder; `.png` is added when missing |
| Format | PNG | PNG | 8 bits per channel (OpenEXR follows with 8.5) |
| Alpha Channel | on/off | off | saves transparency (the background, edges); off: the background is black |

**Sampling** (our page: the path tracer's own settings)

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Samples | 1…4096 per pixel | 16 | smoother edges, less noise in soft shadows and bounces; render time grows with it |
| Bounces | 0…64 | 2 | how often light bounces between surfaces (0: direct light only) |

**Options**

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Auto Light | on/off | on | a scene without lights is lit by the automatic light; off: it stays dark (only luminance shows) |

## Behaviour

- **Opening**: Render ▸ Edit Render Settings…, **Ctrl+B**, or the toolbar's
  render settings button. A window that stays open beside the workspace (not
  modal); the pages (Output, Save, Sampling, Options) are listed on its left.
  Ctrl+B or its close button closes it.
- **Undo**: every change is one undo step ("Render Settings"); a drag on a value
  is one step.
- **Saved with the document** (`.g4d`); a new document starts with the
  defaults; Merge leaves the document's settings alone.
- **Lock Ratio**: while on, typing or dragging Width sets Height (rounded) so
  the proportions from before the change stay, and the other way round.
  Presets set both sides whatever Lock Ratio says.
- **Frame Range**: *All Frames* is the document's range (Project Settings,
  Minimum to Maximum Time); *Current Frame* the frame at the time slider;
  *Manual* From…To. Typing From or To switches the range to Manual. Frames
  shows the count, Frame Step taken into account.
- **File**: the **…** button opens a save dialog. A relative path needs the
  document saved, so it has a folder; until then the render is not saved (the
  Picture Viewer says so, 8.3).
- **Edge cases**: values out of range (typed, or in a file) are clamped; a
  Manual range with To before From ends at From.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `RenderSettings` (`FrameRange`, `validated`, `frames`, `output_path`) in `Document::render` | `grid4d-scene` (`render.rs`) |
| `SceneEdit::SetRender` (whole settings, validated) | `grid4d-scene` (`edit.rs`) |
| `render` in `document.json` (version 3, omitted while default) | `grid4d-io` (`v3.rs`) |
| `RenderOptions::from(&RenderSettings)`, `RenderOptions::auto_light` | `grid4d-raytrace` |
| The window: `RenderSettingsPanel`, edits through `LiveEdit` + `CommitLive` | `grid4d-ui` (`panels/render_settings.rs`) |
| File ▸ … button: `EditorCommand::BrowseRenderOutput` → `FileDialogs::pick_render_output` | `grid4d-ui` (`file.rs`) |

Deliberate differences: one set of render settings per document (no list of
several, no inheritance between them); no separate render frame rate (the
project's is used), Preview Range (Grid4D's timeline has one range), fields,
DPI, film or pixel aspect; the numbering of animation files and other formats
come with 8.5. *Sampling* replaces the reference's anti-aliasing and renderer
pages with the path tracer's two settings.

## Acceptance criteria

- [x] Ctrl+B, the menu entry and the toolbar button open and close the window.
- [x] Every change is one undo step; undo/redo restores the exact settings.
- [x] The settings are saved and loaded with the document; a document with the
      defaults writes nothing new (earlier files and the golden file unchanged).
- [x] Out-of-range values in a file are clamped on loading.
- [x] Lock Ratio keeps the proportions; presets set both sides.
- [x] Frames counts the range with its step; relative output paths resolve
      against the document's folder and get `.png`.
- [x] Without lights and Auto Light off, a render shows only luminance.
- [x] Showing the window on idle frames changes nothing.
