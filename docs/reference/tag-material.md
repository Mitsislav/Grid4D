# The Material tag: projections and layers

> Sources: the reference's [Material Tag](https://help.maxon.net/c4d/en-us/Content/html/TTEXTURE.html)
> page (read 2026-11-06: the tag links a material and holds how it is
> projected, its tiling and a selection limit; an object may carry several,
> later tags layered over earlier ones, a smaller or partly transparent top
> layer showing those below; children without a tag use their parent's;
> dropping a material on a tag replaces only the material). The projections
> are the common set of 3D suites. Written in our own words.
>
> The implementation plan calls this the "Texture tag"; it is Grid4D's
> existing Material tag ([`manager-material.md`](manager-material.md)) grown
> by these settings, so an object never has two tags that link materials in
> different ways. Covers step **15.4** (evaluation of projections in
> `grid4d-eval`; the viewport and path tracer read the result in 15.5 – 15.6).

## Purpose

Placing a material on an object: with the object's own UVs, or projected
from a simple shape (a box for a crate, a sphere for a planet, a plane for a
decal) without unwrapping anything. Several tags layer, so a label can sit
on top of a painted surface.

## Parameters

| Name | Values | Default | Effect |
|------|--------|---------|--------|
| Material | a material, or none | none | what the tag applies |
| Projection | UV, Box, Spherical, Cylindrical, Flat, Camera | **UV** | how surface points get texture coordinates |
| Offset U, V | any | 0, 0 | shift of the coordinates |
| Tiling U, V | > 0 | 1, 1 | repeats |
| Rotation | degrees | 0 | turns the coordinates about the texture's centre |
| Projection placement | position, rotation, size | the object's bounding box | where the projection shape sits, in the object's space (not used by UV) |
| Camera | a camera object, or the active view | none | Camera projection only |

Projections:

- **UV:** the object's own UVs (every primitive and generator makes them
  from step 15.2; a polygon object keeps its own). An object without UVs
  falls back to Box.
- **Box:** each polygon takes the side of a box its normal faces most and
  is projected flat from it; the six sides share the image.
- **Spherical:** longitude and latitude around the placement's centre.
- **Cylindrical:** around the placement's Y axis; caps take the angle and
  the distance from the axis.
- **Flat:** straight along the placement's −Z onto its XY square.
- **Camera:** the image as the camera sees it (screen position), for
  matching a backplate.

## The UVs objects make (step 15.2)

`u` runs to the right of the picture, `v` down it; seen from the front, no
face shows the picture mirrored (a mirrored copy, scale −1, does — as a
mirror would). UVs never depend on an object's size.

| Object | Layout |
|--------|--------|
| Sphere, Cylinder, Cone, Capsule | `u` once around the axis (starting at +Z, towards +X), `v` from top to bottom along the profile, by length; poles take the middle of their step; caps a plan of the disc seen from outside |
| Cube | the whole picture on each side, upright (top and bottom read as a plan, −Z at the top of the picture) |
| Plane, Disc | a plan seen from above (−Z at the top of the picture) |
| Torus | `u` around the axis, `v` around the tube |
| Extrude | walls: `u` along each spline segment (by length, all the way round), `v` along the movement (its end at the top); caps: a plan of each end, unstretched |
| Lathe | `u` along the turn, `v` along the profile (by length); caps as for the Extrude |
| Sweep | `u` along the profile, `v` along the swept part of the path (its end at the top); caps as for the Extrude |
| Loft | `u` around the sections, `v` across them (the last at the top); caps as for the Extrude |
| Subdivision Surface | its cage's UVs, smoothed with it; a cage part without UVs gets a Box projection first |
| Cloner | each clone shows its source's UVs |
| Plugin generators | the plugin's UVs, else a Box projection |

Make Editable and Current State to Object keep these UVs; glTF and OBJ
export them.

## Behaviour

- **Several tags:** an object may carry any number of Material tags; they
  are applied in their order, each later one over the earlier ones wherever
  its material is opaque (Opacity channel). The last tag's material is the
  one Material ▸ Apply replaces, and the one children inherit when they
  have none of their own (they inherit all the layers).
- **Dropping a material on a tag** replaces only the material, keeping the
  projection; on an object it gives the last Material tag the material
  (keeping its projection), or adds a tag if there is none; **Shift** adds
  a new tag on top. (Revised from the draft, which added a tag by default:
  replacing keeps 1.0's behaviour for the usual one-tag object.)
- **Fit to Object** (a switch on the tag): on, the shape is the bounding
  box of each object the tag reaches, following it as it changes; off, the
  placement is the tag's own (set to the object's current box when it is
  switched off).
- **Bake to UVs** (Mesh ▸ UV, or the UV editor's *Project*): writes the
  tag's projection into a polygon object's UVs and switches the tag to UV,
  one undo step.
- Edits are ordinary tag edits (`SetData`), one undo step each, live while
  dragging.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `Tag::Material(MaterialTag)` with `Projection`, offset, tiling, rotation, placement | `grid4d-scene` `tag.rs`; Material tags exempt from "one tag of each kind" |
| Which layers reach each object | `grid4d-eval` `material_layer.rs`: `MaterialLayer` per tag (material, projection, the tag's 2D transform, placement), a `MaterialLayers` run on every evaluated node and instance |
| Coordinates per corner | `MaterialLayer::corners` (the viewport caches them per mesh and layer from 15.5) |
| Projection maths (pure) | `grid4d-geom` `uv.rs` (`Projection`, `project`, `fit`; shared with Bake / Project) |
| File | `.g4d` v4: the `material` tag's new optional fields; several per object |

Deliberate differences: no per-polygon-selection limit yet (Grid4D has no
selection tags), no "seamless" or decal-side switches; the projection set is
ours. A placement is in the space of each object the tag reaches (its own
object's, or an inheriting child's), not always the tag's object's.

## As built (15.4b)

- The projections work in the shape's own space: the cube −1 … 1 (Box;
  Flat reads its XY square), the sphere and the cylinder of radius 1
  (Cylindrical: faces looking more than 45° up or down get a plan, like the
  Cylinder's caps). Seen from outside the shape no face is mirrored; faces
  across the seam stay whole (their `u` may pass 1) and corners on the axis
  take their face's other corners' `u`.
- Camera projects through the camera's clip space; the viewport and the path
  tracer give the matrix (15.5 – 15.6). Without a camera matrix the object's
  UVs are used.
- The tag's coordinates: turned about the picture's centre, tiled, then
  offset; a channel's own tiling and offset come after.
- Evaluation keeps no per-corner coordinates: they depend on the drawn mesh
  (a primitive's is unit size), so whoever draws computes them with
  `MaterialLayer::corners` and caches them.
- Bake to UVs is a button on the tag's page (polygon objects; Box,
  Spherical, Cylindrical, Flat); it clears the placement once the tag is UV.

## As built (15.5a, the viewport)

- A shape projection's coordinates are made per corner on the CPU, once per
  GPU mesh: the batch key holds the projection and its matrix (the
  placement, or the fitted box, times the drawn mesh's scale), so moving or
  turning an object reuses them; a change of size or placement makes new
  ones (dropped as soon as they are not drawn).
- The tag's turn, tiling and offset travel per instance and apply in the
  shader, so they cost no new mesh.
- Camera projects through the view's camera (a linked camera object is used
  by the path tracer, 15.6, and by the viewport later).

## As built (15.5b, layers in the viewport)

- An object's Material tags are drawn from the last one that covers the
  whole surface (no Opacity picture, opacity at or above its threshold) up:
  that one as the surface, each later one as a layer — the same mesh drawn
  again on its own depth (an invariant position, depth test less-or-equal,
  no depth writes), lowest layer first, its cut-out leaving what is below.
  Tags under a covering one are not drawn at all.
- Layers are batched like surfaces (by mesh, coordinates, material and
  level); shadows, ambient occlusion and edge lines use the surface only.

## As built (15.6, the path tracer)

- Every layer is read at each hit, from the top down: the first one solid
  there is the surface; where all are cut out, rays and shadows pass.
- Camera projections project the hit point through the tag's camera object
  (where it is at the rendered frame), else through the render's camera.

## Acceptance criteria

- [ ] A cube with a photo on a Box projection shows it on all six sides,
      undistorted, in the viewport and the render.
- [x] Two tags on one object: the later one's opaque parts cover the earlier
      (viewport, 15.5b; render, 15.6).
- [x] A 1.0 document's Material tags read as UV projection, tiling 1 —
      unchanged rendering.
- [x] Bake to UVs gives the same picture as the projection it replaces.
- [x] Each edit is one undo step; save and load keep every setting.
