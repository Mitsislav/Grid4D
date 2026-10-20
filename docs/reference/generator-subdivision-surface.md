# Subdivision Surface generator

> Sources (read 2026-10-19, Cinema 4D 2027.0):
> manual — [Subdivision Surface](https://help.maxon.net/c4d/en-us/Content/html/OSDS.html),
> [its Object properties](https://help.maxon.net/c4d/en-us/Content/html/OSDS-ID_OBJECTPROPERTIES.html),
> [Polygon Object § quadrangles](https://help.maxon.net/c4d/en-us/Content/html/OPOLYGON.html),
> [Optimize](https://help.maxon.net/c4d/en-us/Content/html/5674.html);
> Python SDK [resource index](https://developers.maxon.net/docs/py/2027_0_0/indexes/resource_index.html)
> entry `c4d.Osds` (parameter names, types, ranges).
> Algorithm: Catmull & Clark, *Recursively generated B-spline surfaces on arbitrary
> topological meshes* (1978) — public, no Maxon code involved.
> Written in our own words — no copied text or images.
>
> Covers implementation-plan step **4.7** (generator framework + Subdivision Surface).
> Depends on [`modeling-modes.md`](modeling-modes.md) (polygon objects, Make Editable).

## Purpose

Smooths a coarse polygon mesh — the **cage** — into a dense, rounded surface, live.
The user keeps modelling the few cage points; the generator shows the smooth result.
It is the backbone of organic and hard-surface modelling (hard edges come from
closely spaced edge loops or, later, edge weights).

## Generator behaviour

- **Input:** the generator's **first child** is the cage. It may be a polygon object
  or a primitive (anything that evaluates to a mesh). **Decided:** the first child,
  with its own children merged into one mesh, is the cage; further children of the
  generator are drawn unchanged and are not smoothed.
- **Cage visibility:** the cage's own surface is hidden while the generator is
  enabled; the generator draws the smoothed result in its place, at the generator's
  transform × the cage's transform. In component modes the cage's points/edges/faces
  stay visible and selectable (drawn as a wireframe cage over the smooth surface), so
  the user edits the cage while seeing the result.
- **Enabled flag** (Object Manager tick / `Q`): when off, the cage is drawn as a
  normal object and nothing is smoothed. The manual describes `Q` as toggling between
  cage display and smoothed display.
- **Creating:** Create ▸ Generators ▸ Subdivision Surface adds an empty generator.
  Holding **Alt** while creating wraps the selected object: the new generator takes
  its place in the hierarchy and the object becomes its child (one undo step).
- **Make Editable on the generator:** replaces it with a polygon object holding the
  smoothed mesh at the **Subdivision Viewport** level (decided — what you see is what
  you get) and removes the cage. *Current State To
  Object* gives the same mesh as a copy and keeps the original.
- **Picking:** clicking the smooth surface in Model mode selects the generator; the
  cage is selected through the Object Manager or in component modes.

## Parameters

Names follow the reference application's Attribute Manager; ranges come from the SDK
resource entry `Osds`. Defaults are not stated in either source — the values marked
*verify* are the commonly seen ones and must be confirmed before release.

| Name | Type / range | Default | Effect | Grid4D v0.4 |
|------|--------------|---------|--------|-------------|
| Type | Catmull-Clark · Catmull-Clark (N-Gons) · OpenSubdiv Catmull-Clark · OpenSubdiv Loop · OpenSubdiv Bilinear | verify | Subdivision scheme (see below) | Catmull-Clark with native n-gons only; enum kept open for more |
| Subdivision Viewport | int 0..6 | 2 (verify) | Levels used in the viewport | yes |
| Subdivision Renderer | int 0..6 | 3 (verify) | Levels used by the final renderer | stored now, used in Phase 8 |
| Linear Pre-Subdivision | int 0..6 | 0 | Splits every face into quads around its centre *without* smoothing, N times, before smoothing — keeps the result closer to the cage | yes (cheap: same split without the smoothing rules) |
| Boundary Interpolation | Edge · Edge and Corner | verify | *Edge*: border corners are smoothed too. *Edge and Corner*: border points that belong to a single face stay where they are | yes |
| Subdivide UVs | Standard · Boundary · Edge (classic types); Linear Boundaries · Linear All (OpenSubdiv) | Standard | How UVs follow the subdivision | later (UVs interpolated linearly until then) |
| Triangle Subdivision | Catmark · Smooth (OpenSubdiv only) | Catmark | Smoother rule for triangles of the cage | later |
| Edge Crease | Uniform · Chaikin (OpenSubdiv only) | Uniform | Transition between differently weighted edges | later (with weights) |
| R12 Compatibility | bool | off | Legacy weighting | never (no legacy files to load) |

### Types, in our terms

| Type | What it does |
|------|--------------|
| Catmull-Clark | Triangulates n-gons first, then Catmull-Clark; result is all quads |
| Catmull-Clark (N-Gons) | Catmull-Clark directly on n-gons (an n-gon becomes n quads) — usually rounder, fewer artefacts |
| OpenSubdiv Catmull-Clark | Pixar's reference implementation of the same scheme; identical to plain Catmull-Clark when no weights are used; weights behave slightly differently |
| OpenSubdiv Loop | Triangle scheme: quads are triangulated first, result is all triangles (game pipelines) |
| OpenSubdiv Bilinear | Splits without smoothing (like Linear Pre-Subdivision alone) |

Grid4D v0.4 implements **one** scheme: Catmull-Clark on n-gons natively (equivalent to
"Catmull-Clark (N-Gons)" and to OpenSubdiv Catmull-Clark without weights). Bilinear
falls out of the same code (skip the smoothing rules). Loop is out of scope.

## Algorithm (one level)

For a cage with points *P*, faces *F*, edges from `Topology`:

1. **Face point** of each face = average of its corner points.
2. **Edge point** of each edge:
   - interior edge (2 faces): average of its 2 end points and the 2 adjacent face points;
   - border edge (1 face) or non-manifold edge (> 2 faces): midpoint of its end points.
3. **New position of each original point** with *n* adjacent edges:
   - interior point: `(Q + 2R + (n − 3)·P) / n`, where *Q* = average of adjacent face
     points, *R* = average of adjacent edge midpoints;
   - border point: `(P_prev + 6·P + P_next) / 8` using its two border neighbours;
   - corner (border point used by a single face) with *Edge and Corner*: unchanged;
   - point on non-manifold edges: treated like a border point along those edges.
4. **Topology:** every face with *k* corners becomes *k* quads
   (corner point, edge point, face point, previous edge point).

Level *L* repeats this *L* times; level 0 is the cage itself. After the first level
every face is a quad, so face count = (corners of the cage) × 4^(L−1).
Smoothing of normals uses the existing `Smoothing` (Phong angle) on the result.

### Topology notes from the manual, as behaviour

- Each edge should have **at most two faces**; more tear the surface. We do not fail:
  such edges use border rules, and the Attribute Manager can show a warning.
- **Only connected faces are smoothed together**: a cube whose sides share no points
  (6 separate quads) smooths into 6 separate rounded patches, not a sphere — *Optimize*
  welds them. The welded Cube from Make Editable must give the familiar rounded box.
- Quads give the cleanest result; triangles and n-gons cause visible pinching. Not an
  error — the tool just has to work.

## Weighting (later — not in v0.4)

Points, edges and polygons can carry a **weight** (0–100 %) that pulls the smooth
surface toward the cage, giving creases without extra geometry. In the reference
application weights are set with the *Weight Subdivision Surface* tool or by holding
`.` and dragging in a component mode (Shift / Ctrl variants also weight the attached
edges or points). Planned for Grid4D as per-point and per-edge crease values on the
polygon object (semi-sharp creases, decreasing by one per level), as its own spec
`modeling-sds-weights.md`.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `ObjectKind::SubdivisionSurface { viewport_levels, render_levels, linear_pre, boundary }` + `ObjectKindTag`, label, icon, Attribute Manager fields | `scene/node.rs`, `ui/icons.rs` (original icon), `ui/panels/attributes.rs` |
| Generator framework: "generator consumes first child, hides it" | `grid4d-eval::generator` — `EvaluatedScene::update` builds (or reuses) each generator's output; the cage's nodes get no drawable or pick shape but stay evaluated, so component modes still edit them |
| `subdivide(mesh, &SubdivisionOptions) -> PolyMesh` | `grid4d-geom::subdivision` (pure, tested on `shapes.rs`) |
| Cache | keyed by (cage `MeshId` or primitive `MeshKey`, cage transform-independent params, levels, options); recomputed only when one changes, so idle frames do nothing. The result is a `MeshHandle` → its own GPU buffer |
| Alt-create wrap, Make Editable on generator, `Q` toggle | `EditorCommand`s through `commit` (one undo step each) |
| File format | new kind in `.g4d` v3 (`KindDto`) |

Performance: level 6 multiplies faces by up to 4^6 = 4096. Subdivision runs off the
hot path (only when inputs change) but must reuse buffers between levels; the
viewport level should stay interactive for a 1 k-quad cage at level 3 (≈ 64 k quads).
Add a criterion bench.

## Decisions (2026-10-19)

1. **Cage = first child**, with its own children merged into one mesh; further
   children of the generator are drawn unchanged. (Whether the reference application
   does exactly this is unconfirmed; it is our rule either way.)
2. **Make Editable on the generator** bakes the **viewport** level.

## Decisions made while implementing (2026-10-20)

3. **Defaults**: Subdivision Viewport 2, Subdivision Renderer 3, Linear
   Pre-Subdivision 0, Boundary Interpolation *Edge and Corner*. The manual does not
   state them; 2/3 is the long-standing convention.
4. **`Q`** (Toggle Parent Generator) is in Maxon's shortcut list. It switches the
   selected generator, or the nearest generator above a selected object; each
   generator is switched once even when several of its objects are selected.
5. **Alt-create**: the generator is inserted at the object's parent and position
   among its siblings, at the parent's origin; the object keeps its own transform,
   so it does not move. Without a selection it is an ordinary create.
6. **Make Editable** skips disabled generators (they show their cage as it is) and
   objects inside a cage that goes with its generator.
7. **Non-manifold edges** (more than two faces) are smoothed like borders.
8. **Hidden or disabled** objects inside the cage are left out of the input; a
   generator inside a cage is not merged (no nesting yet).

## Open questions

1. Default **Type** and **Boundary Interpolation** in 2027.0 (likely an OpenSubdiv
   type). Our single scheme makes the Type choice moot for v0.4, but the default
   boundary rule changes how open meshes (a plane) look.
2. Viewport/renderer default levels (2/3 is the long-standing convention — we use
   it; verify against the application when possible).

## Acceptance criteria

- [x] Level 0 returns the cage unchanged (same points and faces).
- [x] A welded cube (8 points, 6 quads) at level 1 gives 26 points and 24 quads; all
      points lie inside the cage's bounds; the mesh is closed and manifold.
- [x] Face count after *L* levels = (corner count of the cage) × 4^(L−1) for any
      cage (tested on cube, n-gon cylinder, torus, L-shape, star).
- [x] An n-gon with *k* corners becomes *k* quads at level 1.
- [x] Six unwelded cube faces give six separate patches (no shared points); after
      Optimize they give the rounded box (= the welded cube case).
- [x] On an open plane with *Edge and Corner*, the 4 corner points do not move; with
      *Edge* they do.
- [x] A non-manifold edge (3 faces) does not panic and gives finite points.
- [x] Linear Pre-Subdivision 1 + level 0 equals one bilinear split (each face → quads
      around its centre, no smoothing).
- [x] Moving a cage point updates the smooth result the same frame; idle frames do
      not recompute (cache hit) and allocate nothing.
- [x] Alt-create wraps the selected object in one undo step; undo restores the
      hierarchy exactly.
- [x] Disabling the generator shows the cage as a normal object; re-enabling hides it.
- [x] Make Editable on the generator yields one polygon object with the mesh at the
      viewport level and removes the cage; undo restores both.
- [x] A cage with a child of its own smooths both as one mesh; a second child of the
      generator is drawn unsmoothed.
- [x] Save/reload keeps all parameters (round-trip test; the golden demo document
      has no generator, so its fixture is unchanged).
