# Lens and shutter: depth of field and motion blur

> Sources (read 2026-11-07): Cinema 4D manual — Camera object,
> [Physical tab](https://help.maxon.net/c4d/2027/en-us/Content/html/OCAMERA-ID_GROUP_LENS.html)
> (F-Stop: a smaller number gives a shallower depth of field; Shutter Speed
> and, for a movie camera, Shutter Angle — 180° usual — and Shutter Offset,
> which shifts the blur towards the next frame; Diaphragm Shape with Blades
> (3 – 32) and Angle shaping the out-of-focus highlights; the blur only
> appears when the render settings' Motion Blur is on) and the Object tab
> (Focus Distance, or a Focus Object whose distance is used). Written in our
> own words. The model is the standard thin-lens camera (decision D34).
>
> Covers implementation-plan step **16.5**. Builds on
> [`object-camera.md`](object-camera.md) and [`render-engine.md`](render-engine.md).

## Purpose

Pictures that look photographed: a subject in focus with a softly blurred
background and foreground, and movement that smears the way it does on film —
for spinning wheels, passing cars, a fast camera move.

## Parameters

### Camera object ▸ Lens (new group)

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Depth of Field | on / off | off | renders through a lens with an aperture; off: a pinhole, everything sharp (as in 1.1) |
| Focus Distance | 1 … 1 000 000 cm | 500 cm (new cameras: the distance from the view to its orbit centre) | the distance along the camera's axis that is sharp |
| Focus Object | an object | none | when set, the focus distance is the distance from the camera to that object's origin, along the axis (followed through animation) |
| F-Stop | 0.5 … 64 | 2.8 | the aperture: diameter = focal length ÷ F-Stop; smaller is blurrier |
| Blades | 0 (round), 3 … 16 | 0 | the aperture's shape: a polygon of that many blades; out-of-focus highlights take its shape |
| Blade Rotation | −360 … 360° | 0° | turns the polygon |
| Focal Length | read only, mm | from the field of view | shown for orientation: a 36 × 24 mm sensor's height against the vertical field of view, f = 12 mm ÷ tan(fov ÷ 2) |
| Shutter | 0 … 2 frames | 0.5 (a 180° shutter) | how long the shutter stays open, as a fraction of a frame; 0: no blur from this camera |
| Shutter Position | Centred, Opening, Closing | Centred | the open time centred on the frame, starting at it, or ending at it |

- The aperture is real-sized: at the 36 mm convention a 50 mm lens at f/2.8
  opens 1.8 cm, in the scene's centimetres, so depth of field depends on the
  scene's scale as a real camera's does.
- Parallel (orthographic) cameras have no depth of field (the group is
  greyed); motion blur still works.

### Render Settings ▸ Sampling (added)

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Motion Blur | on / off | off | renders over the shutter interval of the camera (the view's own camera: 0.5 frame, Centred) |
| Segments | 2 … 8 | 3 | how many moments of the shutter interval the scene is evaluated at; motion between them is a straight line, so curved paths (a spinning wheel) want more |

## Behaviour

### Depth of field

- Each camera sample starts at a random point on the aperture (a disc, or
  the bladed polygon, turned by Blade Rotation) and aims at its pixel's
  point on the focus plane: points on that plane are sharp, others blur in
  proportion to their distance from it.
- **In the viewport**: no blur; the selected camera's frustum shows its
  focus plane as a rectangle, and a Focus Object is linked by a dashed line.
  The Render View and the Picture Viewer show the blur.
- Focus Distance, F-Stop and Blade Rotation animate.

### Motion blur

- With Motion Blur on, the scene is **evaluated at Segments moments** spread
  over the camera's shutter interval (animation, deformers, generators,
  Cloners, effectors and the simulation cache at sub-frame times; the
  simulation interpolates between its cached frames). Every triangle keeps a
  position per moment; the BVH's boxes cover the whole interval.
- Each camera sample gets a **time** in the interval; the ray meets the
  triangles where they were at that time (linear between moments), and
  shadow rays use the same time. So moving objects, moving lights and a
  moving camera all blur, and so does a deforming or simulated surface.
- Objects whose topology changes during the interval (a generator whose
  point count changes, a Cloner whose count changes) are not blurred: the
  moment at the frame is used throughout (a note in the render's
  information line: "n objects not motion blurred").
- Render time grows with Segments (each moment is a full evaluation and its
  triangles) but not with how far things move.
- The Render View blurs too; the viewport does not.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `ObjectKind::Camera` gains a `Lens` (depth of field, focus distance / object, f-stop, blades, rotation, shutter, position); `ObjectParam`s for the animatable ones | `grid4d-scene` |
| `RenderSettings::{motion_blur, segments}` | `grid4d-scene` (`render.rs`) |
| Sub-frame evaluation (`EvaluatedScene::set_time(f64)`; whole frames unchanged) | `grid4d-eval`, `grid4d-sim` (interpolated cache) |
| A `RenderCamera` with a thin lens; per-sample time; triangles with positions per moment; BVH boxes over the interval | `grid4d-raytrace` (`camera.rs`, `scene.rs`, `grid4d_geom::Bvh` building from boxes) |
| The Lens fields; focus plane in the frustum | `ui/panels/attributes.rs`, `viewport/overlay.rs` |
| Renders evaluate the moments on their worker | `grid4d-ui` (`rendering.rs`) |

Deliberate differences: no exposure (F-Stop changes only the blur, never the
brightness), ISO, gain, lens distortion, vignetting, chromatic aberration,
bokeh bias or anisotropy, no custom aperture picture; motion blur through
sub-frame evaluations only (no motion vectors pass; that is
[`render-passes.md`](render-passes.md)'s business).

## Acceptance criteria

- [ ] A point on the focus plane renders sharp; the blur circle of a point
      off it matches the thin-lens formula within a pixel (a test with a
      small bright sphere); Blades give a polygonal highlight.
- [ ] Depth of field off renders exactly as 1.1 (same samples, same picture).
- [ ] A moving cube blurs over exactly its shutter path (its smear's length
      measured); Shutter 0 or Motion Blur off gives the frame's picture;
      Centred / Opening / Closing move the smear.
- [ ] A deformed, a simulated and a cloned object blur; a camera move blurs
      the whole picture.
- [ ] Renders stay repeatable (the time of a sample is seeded like the rest).
- [ ] Every new field is undoable, animatable where listed, saved in v5,
      stripped from a v4 file.
- [ ] Reference pictures: depth of field with a bladed aperture, a
      motion-blurred spinning wheel.
