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
| The point functions (wrap mapping, path frames, noise, Laplacian/Taubin, explode per polygon) | `grid4d-geom::deform` (pure) |
| Stack entries, links (spline, target, texture), pictures handed in (`EvaluatedScene::set_pictures`) | `grid4d-eval::deform` |
| File | `.g4d` v6 kinds `wrap`, `follow_spline`, `displace`, `project_to_surface`, `smooth`, `explode` (`point_offsets`); links by index; new parameter names (17.9) |

## Acceptance criteria

- [ ] Wrap of a 400 × 200 plane onto a cylinder of radius 400 / 2π closes
      the seam within 1e-3 cm.
- [ ] Follow Spline of a cylinder along a quarter circle bends it with
      constant cross-section (Keep Length).
- [ ] Displace by a known picture moves each point by height × (value − mid)
      along its normal; a missing picture moves nothing.
- [ ] Project to Surface (Nearest Point) puts every point on a sphere target
      within 1e-3 cm; Offset keeps them off by it.
- [ ] Smooth with Keep Volume keeps a noisy sphere's volume within 2 %; Keep
      Borders keeps a plane's border points.
- [ ] Explode at 0 % is the input; at 100 % each polygon is at its distance.
- [ ] Every new deformer: strength 0 % is the input; idle frames do not
      recompute; one undo step per edit; saved as v6 and reopened the same; a
      v5 file naming one is refused.
