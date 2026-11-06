# The `.g4d` file format

The specification of Grid4D's document files, so that other programs can read
and write them. This describes **format version 4**, which Grid4D 1.1 writes.
Version 3 (Grid4D 1.0, **frozen**) is the same format without version 4's
additions — see [Version 4 and version 3](#version-4-and-version-3) and
[Versions](#versions).

> Version 4 is being completed during the development of Grid4D 1.1: until
> 1.1 is released it may still gain optional fields and new types (each
> listed under [Version 4 and version 3](#version-4-and-version-3)); from
> the release on it never changes.

## The container

A `.g4d` file is a **zip archive** (deflate compression, no encryption) with
these entries:

| Entry | Content |
|-------|---------|
| `document.json` | The document: UTF-8 JSON, described below. Always present |
| `meshes/<n>.bin` | The mesh with index *n* (0, 1, 2, …) of `meshes`, little-endian binary, described below. One per mesh; meshes shared by several objects are stored once |
| `simulation.bin` | The saved simulation frames, when `document.simulation_cache` is present |
| `textures/<n>.<ext>` | The picture of texture *n* of `document.textures`, when it is embedded: the original file's bytes, unchanged; `<ext>` is the extension of its `embedded` name, lower case (`bin` when it has none or an unusual one) |

Grid4D writes every entry with the zip format's earliest timestamp, so saving
the same document twice gives identical bytes; readers must not rely on
timestamps or on the order of entries. Entries larger than 4 GiB use zip64.

## `document.json`

The top level:

```json
{
  "format": "grid4d",
  "version": 4,
  "document": { … },
  "meshes": [ { "points": 8, "faces": 6, "corners": 24, "uvs": false } ]
}
```

| Field | Type | Meaning |
|-------|------|---------|
| `format` | string | Always `"grid4d"` |
| `version` | integer | `4` (`3` for a version 3 file) |
| `document` | object | The document (below) |
| `meshes` | array | One entry per `meshes/<n>.bin`, in order: its numbers of `points`, `faces`, `corners` and whether it has `uvs` |

### Conventions

- **Units:** lengths in centimetres; world space right-handed with Y up.
- **Angles** in radians, except fields whose name ends in `_deg`.
- **Rotations** are H, P, B (`rotation_hpb`): heading about Y, then pitch
  about X, then bank about Z.
- **Colours** are linear RGB, each 0 – 1.
- **Vectors** are arrays of three numbers `[x, y, z]`.
- **References** between objects, to materials and to layers are **indices**
  into the corresponding arrays (`document.objects`, `document.materials`,
  `document.layers`).
- **Optional fields** are left out while they have their default; a reader
  must accept them missing. The defaults are given below.
- Every number must be finite (no NaN or infinity); Grid4D refuses files with
  others. Numbers outside an object's valid range (a segment count of 0, say)
  are clamped to the range when read.
- Unknown fields are ignored; an unknown `type` of object or tag is an error.

### `document`

| Field | Type | Default | Meaning |
|-------|------|---------|---------|
| `name` | string | — | The document's name |
| `time` | object | — | `fps` (integer), `start` and `end` (frames, integers), `looping` (bool) |
| `objects` | array | — | Every object, parents before their children (below) |
| `materials` | array | `[]` | Materials: `name`, `color`, `roughness` (0 – 1), `metallic` (0 – 1), `emission` (linear RGB, may exceed 1) |
| `layers` | array | `[]` | Layers: `name`, `color`, and the switches `solo`, `view`, `render`, `manager`, `locked`, `animation`, `generators`, `deformers` (bools) |
| `takes` | object | none | Takes (below) |
| `render` | object | the defaults | Render settings (below) |
| `simulation` | object | the defaults | `enabled`, `gravity` (cm/s²), `steps_per_frame`, `solver_iterations`, `time_scale`, `memory_limit_mb` |
| `simulation_cache` | object | none | Saved simulation frames: `start` (frame), `frames` (count), `bodies` — each `object` (index), `clone` (a clone's number, for a Cloner's clones; optional) and `scale` |
| `textures` | array | `[]` | The document's textures (below) |

### Objects

Each entry of `objects`:

| Field | Type | Default | Meaning |
|-------|------|---------|---------|
| `name` | string | — | Its name (names need not be unique) |
| `parent` | integer | none (top level) | Its parent's index, always **smaller** than its own; siblings keep the order of the array |
| `kind` | object | — | What it is: a `type` and its parameters (below) |
| `position`, `rotation_hpb`, `scale` | vectors | — | Its transform relative to its parent |
| `display_color` | colour | — | Its colour in the views |
| `visible_in_editor`, `visible_in_render`, `enabled` | bools | — | Its switches |
| `tags` | array | `[]` | Its tags (below), at most one of each type |
| `tracks` | array | `[]` | Its animation (below) |
| `layer` | integer | none | Its own layer's index (children without one inherit it) |

The `kind` of an object, by its `type`:

| `type` | Parameters |
|--------|------------|
| `null` | — |
| `cube` | `size` (vector), `segments` ([x, y, z] integers) |
| `sphere` | `radius`, `segments` |
| `cylinder` | `radius`, `height`, `rotation_segments`, `height_segments` |
| `plane` | `width`, `depth`, `width_segments`, `depth_segments` |
| `cone` | `bottom_radius`, `top_radius`, `height`, `rotation_segments`, `height_segments` |
| `torus` | `ring_radius`, `pipe_radius`, `ring_segments`, `pipe_segments` |
| `disc` | `inner_radius`, `outer_radius`, `rotation_segments`, `disc_segments` |
| `capsule` | `radius`, `height`, `rotation_segments`, `height_segments`, `cap_segments` |
| `camera` | `fov_y_deg` (vertical field of view, degrees) |
| `light` | `color`, `intensity` (1 = 100 %); `light_type`: `omni` (default), `spot`, `infinite`; `details` (default: none): `use_inner`, `inner_angle_deg`, `outer_angle_deg`, `falloff` (`none`, `linear`, `inverse_square`), `radius`; `shadow`: `none` (default), `shadow_maps`, `raytraced`, `area` |
| `polygon` | `mesh` (index into `meshes`); `selected`, `hidden` (component sets, below; default empty) |
| `subdivision_surface` | `viewport_levels`, `render_levels`, `linear_pre` (0 – 6), `boundary` (`edge`, `edge_and_corner`) |
| `extrude` | `offset`, `movement` (vector), `subdivision`; `direction`: `auto` (default), `x`, `y`, `z`, `custom`, `absolute`; `reverse_normals` (false), `hierarchical` (false), `start_cap` (true), `end_cap` (true), `cap_type`: `n_gons` (default), `triangles` |
| `lathe` | `angle`, `subdivision`, `movement` (0), `scaling` (1), `reverse_normals` (false), `start_cap` (true), `end_cap` (true), `cap_type` |
| `sweep` | `end_scale` (1), `end_rotation` (0), `start_growth` (0), `end_growth` (1), `parallel_movement` (false), `constant_cross_section` (true), `banking` (true), `reverse_normals` (false), `start_cap` (true), `end_cap` (true), `cap_type` |
| `loft` | `subdivision_u`, `subdivision_v`, `organic`, `per_segment`, `looped`, `linear`, `reverse_normals` (all false), `start_cap`, `end_cap` (true), `cap_type` |
| `cloner` | `layout` (below), `object` (index, Object mode's surface; none), `effectors` (indices; `[]`), `clones`: `iterate` (default), `random`; `seed`; `reset_coordinates` (true) |
| `plain_effector`, `random_effector` | `strength` (fraction); `position`, `scale` (fraction), `rotation`, `color` (each optional: off when absent); `uniform_scale` (false); `seed`; `falloff` (default Infinite): `shape` (`infinite`, `sphere`, `box`, `linear`), `size`, `soft` |
| `bend` | `size`, `strength`, `angle` (0), `keep_length` (false), `origin`: `bottom` (default), `center_c`, `center_s`, `top`; `mode` |
| `twist` | `size`, `angle`, `mode` |
| `taper`, `bulge` | `size`, `strength` (fraction), `curvature` (0), `fillet` (false), `mode` |
| `ffd` | `size`, `grid` ([x, y, z] point counts), `mode`, `lattice` (the points; default: the regular grid), `selected`, `hidden` (point ranges) |
| `spline` | a spline primitive: `spline` (its shape, below), `plane`: `xy` (default), `zy`, `xz`; `reverse` (false); `intermediate_points` (default adaptive, 5°) |
| `text` | `text`, `height`, `align`: `left` (default), `center`, `right`; `horizontal_spacing`, `vertical_spacing` (0); `plane`, `reverse`, `intermediate_points` |
| `editable_spline` | `spline_type` (`linear`, `cubic`, `akima`, `b_spline`, `bezier`), `points` (vectors), `tangents` (per point `[in, out]`; default all zero), `segments` (each `count` points, `closed`), `intermediate_points`, `selected`, `hidden` |
| `plugin` | `plugin` (the plugin's type id, e.g. `"grid4d.gear"`), `values` (below) |

A deformer's `mode` is `limited` (default), `within_box` or `unlimited`.

A Cloner's `layout` has a `mode`:

| `mode` | Fields |
|--------|--------|
| `linear` | `count`, `offset` (0), `step`: `per_step` (default), `end_point`; `position`, `rotation` ([0, 0, 0]), `scale` (per-step factors) |
| `radial` | `count`, `radius`, `plane`, `align`, `start_angle` (0), `end_angle`, `offset` (0) |
| `grid` | `count` ([x, y, z]), `step`, `size` |
| `object` | `distribution`: `vertex` (default), `polygon_center`; `align` |

A spline primitive's `spline` has a `shape`:

| `shape` | Fields |
|---------|--------|
| `circle` | `radius`, `ellipse` (the second radius; none), `ring` (the inner radius; none) |
| `rectangle` | `width`, `height`, `rounding` (none) |
| `arc` | `arc_type` (`arc`, `sector`, `segment`, `ring`), `radius`, `inner_radius`, `start`, `end` |
| `n_side` | `radius`, `sides`, `rounding` (none) |
| `star` | `inner_radius`, `outer_radius`, `twist`, `points` |
| `helix` | `start_radius`, `start_angle`, `end_radius`, `end_angle`, `radial_bias`, `height`, `height_bias`, `subdivision` |
| `flower` | `inner_radius`, `outer_radius`, `petals` |

`intermediate_points` has a `mode`: `none`, `natural` / `uniform` (with
`number`), `adaptive` (with `angle`), `subdivided` (with `angle` and
`max_length`).

**Component sets** (`selected`, `hidden`) hold inclusive index ranges
`[first, last]`, ascending, in up to three lists: `points`, `edges`,
`polygons`. Edges are given **by corner**: corner *c* (an index into the mesh's
corners) stands for the edge from *c* to the next corner of its face.

**Plugin values** are objects with a `name` and one of: `float`, `int`,
`bool`, `choice` (an index), `vector`, `color` — e.g.
`{"name": "Teeth", "int": 12}`. A reader that does not know the plugin must
keep the values as they are.

### Tags

Each tag has a `type`:

| `type` | Fields |
|--------|--------|
| `phong` | `angle_limit` (bool), `angle` |
| `protection` | `position`, `scale`, `rotation` (bools) |
| `target` | `target`, `up` (object indices; none), `pitch` (bool) |
| `material` | `material` (index into `materials`; none) |
| `dynamics` | `motion` (`dynamic`, `collider`), `bounce`, `friction`; `enabled` (true), `compound` (false), `shape`: `automatic` (default), `box`, `sphere`, `convex_hull`, `static_mesh`; `mass_mode`: `global_density` (default), `custom_density`, `custom_mass`; `density`, `mass`, `rotational_mass` (1); `linear_damping`, `angular_damping` (0); `initial_velocity` (cm/s), `initial_spin` (radians/s) ([0, 0, 0]) |

### Textures

Each entry of `textures` (version 4):

| Field | Type | Default | Meaning |
|-------|------|---------|---------|
| `name` | string | — | Its name in the Texture Manager |
| `file` | string | none | A **linked** picture: its path. A path inside the document's folder is stored **relative** to it, with `/` between folders (`maps/wood.jpg`); any other path as the system wrote it (absolute) |
| `embedded` | string | none | An **embedded** picture: the original file's name (`logo.png`); its bytes are the entry `textures/<n>.<ext>` |
| `color_space` | string | `srgb` | How its values are read: `srgb` (colour pictures) or `linear` (data maps, HDR / OpenEXR) |

Exactly one of `file` and `embedded` is present. A reader resolves a
relative `file` from the folder the `.g4d` file is in; a linked file that does
not exist is not an error (Grid4D shows the texture as missing). Pictures are
PNG, JPEG, TIFF, WebP, Radiance HDR or OpenEXR files, kept exactly as they were.

### Animation

Each track of `tracks`:

- `param`: what it animates — `position_x` … `position_z`, `rotation_h`,
  `rotation_p`, `rotation_b` (radians), `scale_x` … `scale_z`; an object
  parameter by meaning (`size_x`, `radius`, `height`, `width`, `top_radius`,
  `bottom_radius`, `inner_radius`, `outer_radius`, `ring_radius`,
  `pipe_radius`, `ellipse_radius`, `rounding_radius`, `start_radius`,
  `end_radius`, `start_angle`, `end_angle`, `twist`, `radial_bias`,
  `height_bias`, `horizontal_spacing`, `vertical_spacing`, `field_of_view`,
  `intensity`, `inner_angle`, `outer_angle`, `falloff_radius`, `offset`,
  `movement_x` … `movement_z`, `angle`, `rise`, `scaling`, `end_scale`,
  `end_rotation`, `start_growth`, `end_growth`, `strength`, `amount`,
  `curvature`); or a plugin value by name, `{"plugin": "Phase"}`.
- `keys`, ascending by frame: `frame` (may be fractional), `value` (in the
  parameter's stored unit), `interpolation`: `spline` (default), `linear`,
  `step`; `tangents` (default: automatic): `left` and `right`, each
  `[frames, value change]`.

### Render settings

`width`, `height` (pixels), `lock_ratio`, `frames` (`"current"`, `"all"` or
`{"manual": {"from": …, "to": …}}`), `frame_step`, `samples`, `bounces`,
`auto_light`, `save`, `output` (a path), `alpha`, `format`: `png` (default),
`exr`; `naming`: `name0000` (default), `name_dot0000`,
`name_underscore0000`.

### Takes

`takes`: `takes` (each: `name`, `camera` (object index; none), `render`
(`{"settings": …}`; none), `objects` — its overrides), `active` (the active
take's index; none: Main), `render` (the take Render Settings use; none: the
active one), `main_values` (Main's values of what the active take
overrides), `main_render`.

The document's objects hold **the active take's** values. An override lists
an `object` (index) and any of: `transform` (`[position, rotation_hpb,
scale]`), `visibility` (`[editor, render, enabled]`), `parameters` (a whole
`kind`), `tags`, `display_color`.

## `meshes/<n>.bin`

A polygon mesh with *P* points, *F* faces and *C* corners (from the matching
entry of `meshes`), all little-endian, one after the other:

| Part | Size | Content |
|------|------|---------|
| points | *P* × 12 bytes | `x, y, z` as `f32` |
| face sizes | *F* × 4 bytes | the number of corners of each face, `u32` (at least 3) |
| corners | *C* × 4 bytes | each corner's point index, `u32`, face after face |
| UVs | *C* × 8 bytes, when `uvs` | each corner's `u, v` as `f32` |

The face sizes add up to *C*; faces are counter-clockwise seen from outside
and never visit the same point twice. The entry's length must be exactly the
total.

## `simulation.bin`

The poses of `simulation_cache`: for each of its `frames`, for each of its
`bodies` in order, seven `f32` (little-endian): the position `x, y, z` (cm)
and the rotation as a quaternion `x, y, z, w`. The entry's length is
`frames × bodies × 28` bytes exactly.

## Limits

Grid4D never trusts a size a file states before allocating: `document.json`
is read up to 256 MiB, mesh entries must have exactly the length their counts
give, `simulation.bin` is read up to 2 GiB, an embedded texture up to 1 GiB
(and its picture is decoded within 16 384 pixels a side and 1 GiB of
memory). Indices out of range, a parent index not smaller than the object's
own, mesh faces pointing past their points, and numbers that are not finite
make the file invalid.

## Version 4 and version 3

Version 4 keeps everything about version 3 — the container, every field and
its meaning — and only adds to it. What it adds, as of this writing:

| Addition | Since |
|----------|-------|
| `document.textures` and the entries `textures/<n>.<ext>` | Grid4D 1.1 development (step 15.3) |

A version 3 file is therefore read as a version 4 file with the version
number changed, minus any field this table lists (version 3 readers ignore
unknown fields, so such a field in a version 3 file means nothing). A
version 4 file that uses none of the additions is a valid version 3 file once
its `version` says `3`.

## Versions

| Version | Container | Read by Grid4D |
|---------|-----------|----------------|
| 1, 2 | a single JSON file, `"format": "oc4d"` (from the project's former name OpenC4D; extension `.oc4d`) | yes, converted when opened |
| 3 | the zip archive above, `"format": "grid4d"`; Grid4D 1.0 | yes, converted when opened |
| 4 | the same archive with version 4's additions; Grid4D 1.1 | yes; written |

A version grows during development only by optional fields and new types.
**From Grid4D 1.0 a released version never changes:** new data means a new
version number, written by newer Grid4Ds and converted from every older
version when opened. A Grid4D refuses files with a version newer than it
knows, with a message naming both versions (Grid4D 1.0 refuses version 4
files this way).
