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
| Keep Hard Edges | bool | on | Hard edges (17.8) are treated like borders. |
| Reduce Children Together | bool | on | All children reduced as one mesh (a share of the whole); off: each child keeps the share of its own. |

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
| A scan of a million triangles: the collapse order | ≤ 5 s, once per change of the input (today its topology and BVH alone take 0.35 s) |
| The same, Keep dragged (a replay to 10 %) | ≤ 100 ms a change (R9's fallback applies above 33 ms) |

## Acceptance criteria

- [ ] A sphere reduced to 10 % keeps 10 % of its triangles (± 1 %) and stays
      within 2 % of its radius everywhere.
- [ ] A plane with a hole: the outer border and the hole keep their outline
      (border points stay on the original border).
- [ ] A cube unwrapped with seams: with Keep UV Seams no UV crosses a seam.
- [ ] No face flips (every result normal within 90° of the input's there).
- [ ] Changing Keep replays without recomputing the order (a counter in a
      test); idle frames do nothing.
- [ ] Keep 100 % returns the triangulated input; 0 % returns a valid mesh.
- [ ] Make Editable gives the shown mesh in one undo step.
- [ ] Saved as v6 (Keep keyed) and reopened the same; a v5 file naming it is
      refused.
