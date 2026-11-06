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

## Behaviour

- **Several tags:** an object may carry any number of Material tags; they
  are applied in their order, each later one over the earlier ones wherever
  its material is opaque (Opacity channel). The last tag's material is the
  one Material ▸ Apply replaces, and the one children inherit when they
  have none of their own.
- **Dropping a material on a tag** replaces only the material, keeping the
  projection; on an object it adds a new tag at the end (Shift: replaces the
  last tag's material instead).
- **Fit to Object** (tag's context menu): the placement becomes the
  object's bounding box again.
- **Bake to UVs** (Mesh ▸ UV, or the UV editor's *Project*): writes the
  tag's projection into a polygon object's UVs and switches the tag to UV,
  one undo step.
- Edits are ordinary tag edits (`SetData`), one undo step each, live while
  dragging.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `Tag::Material(MaterialTag)` with `Projection`, offset, tiling, rotation, placement | `grid4d-scene` `tag.rs`; Material tags exempt from "one tag of each kind" |
| Coordinates per corner | `grid4d-eval`: per drawable, cached with the mesh key, rebuilt when the tag or the mesh changes |
| Projection maths (pure) | `grid4d-geom` `uv.rs` (shared with Bake / Project) |
| File | `.g4d` v4: the `material` tag's new optional fields; several per object |

Deliberate differences: no per-polygon-selection limit yet (Grid4D has no
selection tags), no "seamless" or decal-side switches; the projection set is
ours.

## Acceptance criteria

- [ ] A cube with a photo on a Box projection shows it on all six sides,
      undistorted, in the viewport and the render.
- [ ] Two tags on one object: the later one's opaque parts cover the earlier.
- [ ] A 1.0 document's Material tags read as UV projection, tiling 1 —
      unchanged rendering.
- [ ] Bake to UVs gives the same picture as the projection it replaces.
- [ ] Each edit is one undo step; save and load keep every setting.
