# Boolean generator

> Sources (read 2026-11-09, Cinema 4D 2027.0): manual — the legacy Boole
> object's [Object properties](https://help.maxon.net/c4d/2027/en-us/Content/html/OBOOLE-ID_OBJECTPROPERTIES.html)
> (the four modes, single object, hidden new edges, breaks and selection at
> the cut, point merging); Python SDK — the newer Boolean generator's
> parameter list (operation, resolving self-intersections, normals, cut-edge
> selection). Methods from public papers, our own code: Shewchuk, *Adaptive
> precision floating-point arithmetic and fast robust geometric predicates*
> (1997); Jacobson, Kavan and Sorkine-Hornung, *Robust inside-outside
> segmentation using generalized winding numbers* (2013); the plane-based
> representation as in Campen and Kobbelt, *Exact and robust (self-)
> intersections for polygonal meshes* (2010). Written in our own words — no
> copied text or images.
>
> Covers implementation-plan steps **17.2** (the method, decided by a spike)
> and **17.3** (the generator). Decisions D39, D40; risks R6, R9.

## Purpose

Combines two shapes as solids — one with the other added, cut away or
intersected — **live**: the inputs stay ordinary objects the user moves and
edits, and the result follows at interactive speed. The everyday hard-surface
tool for holes, slots, cut-outs and merged parts.

## Inputs

- **A** is the generator's **first child**, **B** its **second**; each with
  its own visible children merged into one mesh in the generator's space (as
  the Subdivision Surface merges its cage). Further children are drawn as
  themselves, unchanged.
- An input may be anything that evaluates to a mesh: a primitive, a polygon
  object, a deformed object, **another generator's result** (a Boolean of a
  Boolean, a Subdivision Surface, an Extrude). Generators nest: inner ones are
  built first.
- Splines, cameras, lights and empty groups give no mesh; with A or B
  missing the generator shows A alone (or nothing).
- Hidden (View off) and disabled objects are left out, as everywhere.

## Parameters

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Mode | Union · Subtract · Intersect · Without | Subtract | Union: everything inside either. Subtract: A with B's volume cut away, closed where B cut. Intersect: only what is inside both. Without: A's surface outside B, **left open** where B cut it (no new faces from B). |
| Merge Coplanar Faces | bool | on | The pieces one source polygon was cut into are merged back into n-gons (quads where they can be), so the result has the source's edges and the cut's, not a triangle soup. Off: triangles (for debugging and export to tools that want them). |
| Hard Cut Edges | bool | on | The edges along the cut become **hard edges** (step 17.8): the result's smoothing breaks there even within the Phong angle. |
| Select Cut Edges | bool | on | Make Editable leaves the cut edges selected (Edges mode), ready for a bevel. |
| Weld Tolerance | length ≥ 0 | 0.01 cm | Points of the result closer than this are welded; cuts that pass within it of an existing point use that point. |

None of these is animatable in 1.3 (switches and a tolerance). The mode is a
closed set of four.

## Behaviour

- **Shown as one object**: the result is drawn and picked as the generator;
  A and B are hidden while it is enabled (Object Manager tick, `Q`) and drawn
  as themselves when it is off. In component modes the active input's points,
  edges and polygons stay editable (drawn as a wire cage, like a Subdivision
  Surface's), so a cutter can be reshaped while the result updates.
- **Closed in, closed out**: closed, consistently oriented inputs give a
  closed result (no holes, no flipped faces) in Union, Subtract and
  Intersect. Slightly open inputs (a missing cap, small gaps) still work: a
  point is inside an input when its **generalised winding number** is above
  one half, so a nearly closed mesh behaves as the solid it nearly is.
- **Hard cases** — faces lying in each other's plane, shared edges or
  points, inputs that only touch, a cutter flush with a face — give a
  correct result, never a crash: decided by **exact predicates** (the spike
  chooses how, 17.2), never by a tolerance alone.
- **Origins**: every face of the result records where it came from — input
  (A or B) and that input's face. The result's UVs are each source corner's
  UVs carried by interpolation inside its source polygon (B's faces in
  Subtract are seen from inside: their UVs are kept, their winding turned).
  Origins make later per-part materials and polygon selections possible
  (not in 1.3, D39).
- **Materials**: in 1.3 the result is one object and takes the Boolean's own
  Material tags (or what it inherits), like any generator; the inputs' tags
  do not carry over (per-polygon materials are later).
- **Bad inputs never panic**: when an input cannot be read as a solid (no
  faces, non-finite points, a result the method refuses), the generator
  shows **A unchanged** and says why in its Attribute Manager page (for
  example "The second object has no closed volume"). Fixing the input
  clears the message.
- **Live under a drag** (R9): the result is cached by its inputs' key (their
  mesh ids or primitive parameters, their matrices relative to the
  generator, the parameters) and rebuilt only when one changes. If a rebuild
  takes longer than the frame can afford (more than 33 ms, below 30 fps),
  a drag of an input shows the **last result** and rebuilds when the drag
  ends; the status bar says so ("Boolean updates when you release"). Undo
  steps are not affected (one per drag, as always).
- **Make Editable** replaces the generator with one polygon object holding
  the result (the cut edges selected when Select Cut Edges is on, its hard
  edges kept) and removes A and B; **Current State to Object** makes a copy;
  export bakes the result. One undo step each.
- **Creating**: Create ▸ Generators ▸ Boolean; Alt + create wraps the
  selected object (it becomes A).
- **Renders** use the same result (the generator at render levels: an input
  that is a Subdivision Surface uses its render level).

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `ObjectKind::Boolean(Boolean)`, `ObjectKindTag::Boolean` in `GENERATORS`, label, icon, Attribute Manager page with the message row | `scene/node.rs`, `ui/icons.rs`, `ui/panels/attributes.rs` |
| `boolean(a, b, mode, method) -> Result<BooleanResult { mesh, origins, stats }, BooleanError>` (17.2; options and merging back into n-gons in 17.3) | `grid4d-geom::boolean` (pure: `kernel.rs` planes, points and predicates; `mod.rs` the pipeline) on `grid4d-geom::exact` (intervals, expansions) |
| Inputs, nesting, cache, the error kept for the Attribute Manager | `grid4d-eval::generator` (built deepest first; `generator_mesh` bakes it) |
| The R9 fallback (last result while dragging) | `grid4d-eval` (the rebuild's time kept per generator) + the viewport's drag state; status-bar text through `i18n::tr` |
| File | `.g4d` v6 kind `boolean` (17.3) |

## Budget (`modeling_budget` example)

| Case | Target | The spike (17.2, exact method) |
|------|--------|--------------------------------|
| A product shell: a bevelled box (≈ 2 000 triangles) minus two cylinders, a cutter dragged | ≤ 8 ms a rebuild: the drag stays at 60 fps (a live Subdivision Surface over 1 152 faces costs 2.3 ms a frame) | 29 – 38 ms (a box of 1 728 triangles minus two cylinders of 124): not met yet |
| Two spheres of 50 000 triangles each | ≤ 150 ms a rebuild (the R9 fallback applies while dragging); triangulating them and building each one's BVH and topology alone costs 36 ms | 0.47 – 0.67 s: not met yet |
| A million triangles against a cutter | finishes (no time promised), no panic | 10 – 11 s, closed |

The spike's code was written to be right, not fast: every stage still
allocates per piece, keys edges in hash maps and visits every triangle of
both inputs, even those far from the other. Step 17.3 works only near the
cut (untouched triangles pass straight through), groups pieces through the
inputs' own topology, and keeps its buffers between rebuilds; the targets
stand.

## The spike (17.2): how the method was chosen

Both candidates were built as **one pipeline** that differs only in how it
holds the points a cut makes, so the comparison measures that alone:

- **Exact (plane-based)**: a made point stays the meeting point of three
  input planes; its side of any plane is decided exactly (an interval filter
  first, exact expansion arithmetic when the interval cannot tell: 8 471
  times for the product shell, 105 149 for the million triangles).
- **Rounded**: a made point is rounded to `f64` when made and treated as an
  input point from then on; tests on it are exact, but it no longer lies on
  the planes that made it, so joining equal points and telling whether a
  piece lies on the other surface need a tolerance (10⁻⁹ of the scene).

| | Exact | Rounded |
|-|-------|---------|
| The corpus (14 hard cases: overlapping, coplanar faces, a shared edge, a shared point, face to face, identical, nearly touching, a hole flush with two faces, a cube turned by 10⁻⁶ rad, touching from within, spheres, a sphere and a cylinder, a plate and nine cylinders, non-manifold A) | all pass (closed, volumes exact to 10⁻⁵) | 4 fail (open results: the turned cube, both sphere cases, the plate) |
| 1 000 random pairs (boxes, spheres, cylinders; half on a coarse grid) | 0 wrong | 185 wrong |
| The same 1 000, time | 3.0 s | 1.4 s |
| The three budget cases | closed; 29 – 38 ms, 0.47 – 0.67 s, 10 – 11 s | open; 37 – 46 ms, 1.3 – 1.7 s, 120 s |
| 20 000 random pairs; 5 000 meshes of any points and faces | closed and right; no panic | — |

**Outcome (D39): the exact, plane-based method.** It is the only one that
passes everything, and on real sizes it is also the faster (the rounded
method's near-misses make more pieces and send classification into long
searches). The triangle arrangement with a constrained triangulation per
triangle, D39's other candidate, was not built: its weak point is the same
as the rounded method's (points made at intersections must be rounded to be
triangulated), and the plane-based cut of convex pieces needs no
triangulation at all.

What the spike found on the way (each now a test):

- Points a hair apart (10⁻¹⁶) are real: a line crossing the two
  nearly-coplanar halves of a quad turned in `f32` meets them at two
  points. Inserting points into an edge in order of their *approximate*
  position put such a pair in opposite orders on the edge's two sides; the
  order is now decided exactly, by the coordinate along the edge's main
  axis.
- The quick test that finds candidates for an edge must allow the
  tolerance past both ends: a point 10⁻¹⁶ from an end rounded to just
  beyond it.
- A group's place is asked of its largest piece first: a sliver's centre
  can lie a hair outside the solid it belongs to, and must never outvote it.

## Decisions (2026-11-09)

1. Default mode **Subtract** — the most common use (holes and cut-outs).
2. Inputs may be generators (nesting) — a chain of Booleans is how such
   models are built; the Subdivision Surface's "no nesting yet" rule is
   lifted for every generator in 17.3.
3. The method is chosen by the spike (17.2) and recorded here and in D39:
   **exact, plane-based** (see *The spike* above).
4. Edge planes run along the axis a triangle's normal leans on least, so
   they are never parallel to it and need no square roots.
5. Equal points are found near each other in a grid of 10⁻⁶ of the scene
   and joined only when exactly equal; nothing is welded by distance in the
   exact method (the Weld Tolerance parameter, 17.3, is a separate pass).

## Acceptance criteria

- [ ] Each mode on two overlapping cubes gives the expected closed result
      (point count, volume within 1e-4, every edge between two faces).
- [x] The boolean corpus (17.2: coplanar faces, shared edges and points,
      touching, nearly touching, non-manifold, slightly open, a million
      triangles) passes, and the fuzzer finds no panic (property tests in
      `cargo test`; the `boolean` cargo-fuzz target in CI and weekly).
- [ ] Merge Coplanar Faces: a cube minus a smaller cube through one face
      gives n-gons on that face, quads where possible, no slivers.
- [ ] UVs on the result equal the source's UVs at every source point kept.
- [ ] Every face's origin names the right input and face.
- [ ] A bad input shows A and a message; fixing it clears both.
- [ ] Moving B updates the result in the same frame; idle frames do not
      rebuild and allocate nothing.
- [ ] The R9 fallback: a slow rebuild shows the last result during a drag
      and the new one after release.
- [ ] Make Editable gives one polygon object with the cut edges selected and
      hard; undo restores the generator and both inputs exactly.
- [ ] Saved as v6 and reopened, the parameters are the same; a v5 file
      naming `boolean` is refused.
