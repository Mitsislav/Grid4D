# Deformers 2: Wrap, Follow Spline, Displace, Project to Surface, Smooth, Explode, Point Offsets

> Sources: the reference's pages for its wrapping, spline-following,
> displacing, shrink-wrapping, smoothing, exploding and correcting deformers
> were not read for this spec; each is described by what it does to points,
> in plain terms, with options of our own. Smoothing that keeps volume by
> Taubin, *A signal processing approach to fair surface design* (1995),
> public. Written 2026-11-09 in our own words.
>
> Covers implementation-plan step **17.9**. Decision D44; risk R5 (Point
> Offsets may move to 1.4). Names (R8): **Wrap**, **Follow Spline**,
> **Displace**, **Project to Surface**, **Smooth**, **Explode**, **Point
> Offsets** — plain and descriptive, none the reference's own.

## Common rules (as `deformer-basics.md`)

- A deformer reshapes its **parent and the parent's subtree**, in the
  deformer stack while walking, deepest first, cached per node; generator
  inputs carry the deformers inside them. **Strength** (0 – 100 %, where it
  has one) blends the deformed points with the undeformed. Enabled tick and
  layers as for every deformer. Each draws its own guide (a box, a cylinder,
  a sphere) as a curve, picked like a spline; strength handles where noted.
- Parameters marked *keyed* animate as `ObjectParam`s.

## Wrap

| Name | Range | Default | Effect |
|------|-------|---------|--------|
| Shape | Cylinder · Sphere | Cylinder | What the flat input is wrapped onto. |
| Width, Height | length | 400, 200 cm | The flat region (in the deformer's X and Y) that maps onto the shape. |
| Radius | length | 100 cm | *Keyed*. |
| Angle Start, Angle End | angle | −180°, 180° | How far round the width wraps (*keyed*). |
| Strength | % | 100 % | *Keyed*; handle. |

- A point at (x, y, z) in the deformer's space: x across Width maps to the
  angle between Start and End, y stays (Cylinder) or maps to latitude
  (Sphere), z adds to the radius. A label wrapped round a bottle.

## Follow Spline

| Name | Range | Default | Effect |
|------|-------|---------|--------|
| Spline | link | none | The path. |
| Axis | +X · −X · +Y · −Y · +Z · −Z | +Y | The object's axis laid along the path. |
| Fit | Keep Length · Fit to Spline | Keep Length | Keep the object's length along the path, or stretch it over the whole path. |
| Offset | % of the path | 0 % | Where along the path it starts (*keyed*). |
| Strength | % | 100 % | *Keyed*. |

- Uses the path's evaluated curve with parallel-transport frames (as the
  Sweep); a closed path wraps around.

## Displace

| Name | Range | Default | Effect |
|------|-------|---------|--------|
| Source | Texture · Noise | Noise | — |
| Texture | a document texture | none | Its brightness (the picture's luminance). |
| Mapping | UV · Flat (the deformer's Z) | UV | Where the picture is read. |
| Noise Scale, Octaves, Seed | length, 1 – 8, int | 50 cm, 3, 0 | Fractal noise in the deformer's space. |
| Height | length | 10 cm | *Keyed*. |
| Mid Level | 0 – 1 | 0.5 | The value that does not move. |
| Direction | Normal · X · Y · Z | Normal | — |
| Strength | % | 100 % | *Keyed*. |

- **Pictures in evaluation (D44)**: the editor hands evaluation the decoded
  pictures it already keeps (`TextureCache`), keyed by texture and its
  version; a missing or still-loading picture displaces nothing and the
  deformer rebuilds when it arrives. Renders and Current State use the same
  pictures.

## Project to Surface

| Name | Range | Default | Effect |
|------|-------|---------|--------|
| Target | link to an object | none | What the points are pulled onto (its evaluated mesh). |
| Mode | Nearest Point · Along Direction · Along Normal | Nearest Point | Along Direction casts along the deformer's −Z; Along Normal along each point's normal (both ways, the nearer hit). |
| Offset | length | 0 cm | Kept off the surface along its normal. |
| Strength | % | 100 % | *Keyed*. |

- Uses the target's cached BVH (built once per target mesh); a point with no
  hit stays. A decal projected onto a shell.

## Smooth

| Name | Range | Default | Effect |
|------|-------|---------|--------|
| Iterations | 1 – 200 | 10 | — |
| Keep Volume | bool | on | Alternating shrink and inflate steps, so the shape does not shrink. |
| Keep Borders | bool | on | Open borders do not move. |
| Strength | % | 100 % | *Keyed*. |

## Explode

| Name | Range | Default | Effect |
|------|-------|---------|--------|
| Strength | % | 0 % | How far the explosion has gone (*keyed*: animated from 0 to 100). |
| Distance | length | 300 cm | How far polygons fly at 100 %, outward from the deformer's origin. |
| Variation | % | 50 % | Random spread of each polygon's distance. |
| Spin | angle | 180° | Random turn of each polygon about its centre at 100 %. |
| End Size | % | 100 % | Each polygon's size at 100 %. |
| Seed | int | 0 | — |

- Polygons are split apart (each gets its own points); with Strength 0 the
  mesh is unchanged (still welded).

## Point Offsets (if time allows; else 1.4, R5)

- Stores **per-point offsets** made on the *deformed* shape: in Points mode,
  the gizmo on an object under this deformer edits the deformed points and
  the offsets are saved on the deformer (by point index), applied last in
  the stack. Freeze clears them. Its file data is an array of offsets.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `ObjectKind::{Wrap, FollowSpline, Displace, ProjectToSurface, Smooth, Explode, PointOffsets}` in `DEFORMERS`, labels, icons, pages, `ObjectParam`s for keyed values | `scene/node.rs`, `scene/param.rs` |
| The point functions (wrap mapping, path frames, noise, Laplacian/Taubin, explode per polygon) | `grid4d-geom::deformers` (pure) |
| Stack entries, links (spline, target, texture), pictures handed in (`EvaluatedScene::set_pictures`) | `grid4d-eval::deform` |
| File | `.g4d` v6 kinds `wrap`, `follow_spline`, `displace`, `project_to_surface`, `smooth`, `explode` (`point_offsets`); links by index; new parameter names (17.9) |

## Decisions made while implementing (2026-11-25)

1. **Point Offsets moved to 1.4** (R5): editing the deformed shape needs
   the gizmo and Points mode to work through a deformer stack, which is a
   step of its own. The other six are in 17.9.
2. **Whole-mesh deformers**: Displace needs normals and UVs, Smooth edges,
   Explode polygons, so the stack now hands each deformer the whole mesh
   (in its own space) rather than its points; the box deformers and plugins
   still see points only. The maths is in `grid4d_geom::deformers` (beside
   `deform`, which keeps the box deformers).
3. **Links are resolved while walking**, placed at the evaluated frame:
   - The path is the linked spline's own curve: a spline primitive, a Text
     or an editable spline. A Spline Boolean's curve is not followed.
     Only the path's first segment is used. Past its ends, points carry on
     along the end segments.
   - The target is a polygon object's mesh or a primitive's (its BVH is
     built once per mesh). The target is not deformed, and generators are
     not targets.
   - A dangling or unsuitable link does nothing.
4. **Pictures (D44)**:
   - The editor turns its decoded pictures into **grey pictures** (luminance,
     at most 1024 wide) for the textures a Displace reads. Only an unchanged
     picture is kept, and the conversion repeats only when a picture changes.
   - It hands them to evaluation (`EvaluatedScene::set_pictures`). Different
     pictures mean a new evaluation, so the deformer updates when the
     picture arrives.
   - Renders make the grey pictures once per render from the pictures they
     take. Make Editable and Current State use the editor's.
5. **Follow Spline**: the object is measured along its Axis from its lowest
   point. With Keep Length that point sits at Path Offset, and with Fit to
   Spline the object spans the whole path. Frames are carried along the
   path by parallel transport, starting from the turn that takes the Axis
   to the path's first direction.
6. **Smooth** is Taubin's λ | μ smoothing with λ = 0.5 and μ = −0.53. Keep
   Volume off is plain Laplacian smoothing. Keep Borders holds the points on
   edges with one polygon.
7. **Explode** moves each polygon outward from the deformer's origin, through
   its centre (along its normal when the centre is at the origin). Distance,
   axis and turn are random per polygon and seed. Polygons keep their order
   and UVs. At Strength 0 the mesh is untouched and still welded.
8. **Displace**:
   - Flat mapping repeats the picture every 100 cm, centred on the deformer.
   - Height is not negative: the parameter is a length like the others.
     Mid Level gives the push inwards.
   - Noise is value noise, smoothly interpolated, in 1 – 8 octaves.
9. **Guides**: the Wrap draws its cylinder (two circles) or sphere (three
   circles). The others draw a 50 cm cross with a diamond, picked like a
   spline. None has a strength handle, and Fit to Parent does nothing for
   them (they have no box).
10. **Keyed values**:
    - Wrap's Radius, Start Angle, End Angle and Strength.
    - Follow Spline's **Path Offset** (a new parameter, `path_offset`, refused
      in a version 5 file) and Strength.
    - Displace's Height and Strength.
    - Project to Surface's Offset and Strength.
    - Smooth's and Explode's Strength.
11. **Project to Surface, Along Direction** casts forward only, down the
    deformer's −Z axis.

## Acceptance criteria (all met in 17.9; Point Offsets moved to 1.4)

- [x] Wrap of a 400 × 200 plane onto a cylinder of radius 400 / 2π closes
      the seam within 1e-3 cm.
- [x] Follow Spline of a cylinder along a quarter circle bends it with
      constant cross-section (Keep Length).
- [x] Displace by a known picture moves each point by height × (value − mid)
      along its normal; a missing picture moves nothing.
- [x] Project to Surface (Nearest Point) puts every point on a sphere target
      within 1e-3 cm; Offset keeps them off by it.
- [x] Smooth with Keep Volume keeps a noisy sphere's volume within 2 %; Keep
      Borders keeps a plane's border points.
- [x] Explode at 0 % is the input; at 100 % each polygon is at its distance.
- [x] Every new deformer: strength 0 % is the input; idle frames do not
      recompute; one undo step per edit (the Attribute Manager's live edits,
      as for every object); saved as v6 and reopened the same; a v5 file
      naming one is refused.
