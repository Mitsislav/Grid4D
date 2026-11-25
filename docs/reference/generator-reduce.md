# Reduce generator

> Sources (read 2026-11-09, Cinema 4D 2027.0): manual — the Polygon
> Reduction generator's Object properties (strength as a share of the
> original's edges, children reduced together or apart, open borders kept, a
> calculation done once and then adjusted interactively). Method from a public
> paper, our own code: Garland and Heckbert, *Surface simplification using
> quadric error metrics* (1997), with boundary and seam constraint planes as
> that paper describes. Written in our own words — no copied text or images.
>
> Covers implementation-plan step **17.5**. Decision D40. Name (R8): the
> reference's *Polygon Reduction* is called **Reduce** here.

## Purpose

Makes a heavy mesh — a scan, a sculpt, an imported CAD model — lighter, live,
keeping its shape, outline and UVs as well as the polygon budget allows.

## Parameters

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
| Keep | 0 – 100 % | 50 % | The share of the input's **triangles** the result keeps (10 % keeps a tenth). Animatable. |
| Keep Borders | bool | on | Open borders (edges with one face) keep their shape: collapses along them are allowed only along the border, at a high cost. |
| Keep UV Seams | bool | on | Edges where the UVs split are treated like borders, so textures do not tear. |
| Keep Hard Edges | bool | on | Hard edges (17.8) are treated like borders. (Added with hard edges in 17.8.) |
| Children Together | bool | on | All children reduced as one mesh (a share of the whole); off: each child keeps the share of its own. |

## Behaviour

- **Inputs**: every visible child and its subtree, as the Join merges them
  (generators' results included), triangulated.
- **Two stages**, like the reference's calculate-then-adjust: when the input
  changes, the **collapse order** is computed once (every edge's quadric
  error, collapses applied in order of cost, each recorded); when only Keep
  changes, the result is replayed from the recorded order up to the share —
  so dragging Keep is interactive even on a large scan.
- Each collapse places the kept point where the summed quadric error is
  least (solved, else the edge's better end or middle). A collapse that would
  flip a face, or join two borders through the interior, is refused.
- UVs follow by interpolation along the collapsed edge (per side of a seam).
- The result is **triangles** (the method works on them).
- Keep 100 % gives the triangulated input unchanged; Keep 0 % stops at the
  smallest mesh the constraints allow (never an empty or broken one).
- Make Editable bakes the result; Current State to Object copies it; export
  bakes it.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `ObjectKind::Reduce(Reduce)` (`keep` animatable as `ObjectParam::Keep`), in `GENERATORS`, label, icon, page | `scene/node.rs`, `scene/param.rs`, `ui/icons.rs`, `ui/panels/attributes.rs` |
| `CollapseOrder::new(mesh, constraints)`, `CollapseOrder::replay(share) -> PolyMesh` | `grid4d-geom::reduce` (pure; a binary heap of costs, buffers reused between replays) |
| Two caches per generator: the order (by input key), the result (by order and Keep) | `grid4d-eval::generator` |
| File | `.g4d` v6 kind `reduce`; `keep` a new parameter name for tracks (17.5) |

## Budget (measured 2026-11-09; `modeling_budget` example)

| Case | Target |
|------|--------|
| A scan of a million triangles: the collapse order | ≤ 5 s, once per change of the input (today its topology and BVH alone take 0.35 s): **3.8 s** (17.5) |
| The same, Keep dragged (a replay to 10 %) | ≤ 100 ms a change (R9's fallback applies above 33 ms): **24 ms** (17.5) |

## Decisions made while implementing (2026-11-24)

1. **The kept point is an end of the edge**, the one where the summed
   quadric error is less, never a solved point in between. The result's
   points are then the input's own with their UVs exactly (no
   interpolation across seams), and a replay is only merges of point and UV
   numbers: fast enough to follow a dragged Keep.
2. **Faces' planes count equally** (the paper's unweighted quadrics):
   weighting them by area took dense parts first and left large triangles
   elsewhere (a sphere's poles).
3. **Seams.** An edge holds the point at one of its ends when the UVs split
   there; where they split only at the far end (the teeth along a sphere's
   pole) the near point is free. A collapse is refused when a UV of the
   moving point would have two places to go (it would stretch across the
   cut). A point held by other than two kept edges (a corner of a border or
   seam) never moves.
4. **Keep Hard Edges** comes with hard edges themselves (17.8); until then
   there are none to keep, and the option is not shown.
5. Keep is stored as a share (0 – 1) and shown in percent; a change of Keep
   replays even while a pointer is held (R9 defers only changes of the
   inputs).
6. Children Together (the reference's reduce-together switch, shortened
   to fit the Attribute Manager) off computes one order per child.

## Acceptance criteria (all met in 17.5)

- [x] A sphere reduced to 10 % keeps 10 % of its triangles (± 1 %) and stays
      within 2 % of its radius everywhere (a sphere of 16 000 triangles:
      1.3 % at most at the faces' centres).
- [x] A plane with a hole: the outer border and the hole keep their outline
      (border points stay on the original border).
- [x] A cube unwrapped with seams: with Keep UV Seams no UV crosses a seam.
- [x] No face flips (every result normal within 90° of the input's there).
- [x] Changing Keep replays without recomputing the order (the order is the
      same one, checked in a test); idle frames do nothing.
- [x] Keep 100 % returns the triangulated input; 0 % returns a valid mesh.
- [x] Make Editable gives the shown mesh in one undo step.
- [x] Saved as v6 (Keep keyed) and reopened the same; a v5 file naming it is
      refused.
