# The simulation cache: playback, baking, renders

> Sources (read 2026-10-30): manual — the Dynamics overview (the start frame
> as the start state; scrubbing backwards; caching) and the project Dynamics
> settings' Cache tab (Bake, Clear Cache, memory). Written in our own words —
> no copied text or images.
>
> Covers implementation-plan steps **11.3** (playback, the cache, renders) and
> **11.5** (saved caches, memory). The bodies themselves are in
> [`simulation-rigid-body.md`](simulation-rigid-body.md).

## Purpose

Show the simulation wherever the scene is shown — scrubbing, playback, the
Render View, final renders, Current State to Object, export — the same motion
every time, without waiting for it more than once.

## Behaviour

- **The start state:** at and before the document's start frame (Minimum
  Time) every body is where the scene puts it.
- **Simulating as you go:** moving to a later frame simulates the frames up to
  it, a little each screen refresh (the viewport stays responsive); the
  result is **cached**, so going back, replaying or scrubbing shows cached
  frames at once. A frame not simulated yet shows the latest simulated one
  until it is.
- **When the cache goes stale:** any change that can alter the motion — a
  body's tag, shape, place, size, animation or children, the Simulation
  settings, the frame rate or the start frame — drops the cache and the
  simulation starts again from the start state. Selecting, renaming, hiding
  or editing objects that are not bodies keeps it.
- **The timeline** shows the cached frames as a bar under its ruler.
- **Simulate ▸ Bake Simulation** simulates the whole range (to Maximum Time)
  now; the status bar shows its progress and **Esc** stops it (what is done
  stays cached). **Simulate ▸ Clear Simulation Cache** empties the cache (it
  refills as frames are visited).
- **Renders** (Render View and Render to Picture Viewer, every frame of an
  animation) show the simulation at their frame: they take a copy of the
  cache and simulate any frame it lacks themselves — the same motion, since
  a run is deterministic.
- **Current State to Object and export** at a frame bake the simulated poses.
- **Editing a body** while the time slider is past the start frame edits its
  start state (as in the reference: set scenes up at the start frame).

## Saved frames and memory (11.5)

- **Saving** a document saves its simulated frames with it; opening the file
  shows them at once. They are kept only if the bodies still start the way the
  saved frames do (the same bodies, scales and start poses as a fresh
  simulation of the opened document) — a file edited elsewhere simulates again.
- **Cache Memory** (Project settings ▸ Simulation, 1 – 65 536 MB, 256 by
  default) caps the simulated frames' memory: at the cap no more frames are
  simulated and the status bar says so. Changing it changes no motion, so it
  keeps the cache.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `Overrides` (world matrices by node, applied after animation and Target tags, children following) and `EvaluatedScene::set_overrides` | `grid4d-eval` |
| `SimCache::overrides(frame, …)`; `Replay` (a cache copy that simulates what it lacks) | `grid4d-sim` |
| `ui::simulation::Sim` in `EditorState` — the inputs key, invalidation, budgeted filling, baking, the cached range | `grid4d-ui` |
| Viewport overrides, the timeline bar, Simulate menu, status bar progress, Esc | `grid4d-ui` panels |
| Renders with a `Replay`; Current State to Object and export with the editor's overrides | `grid4d-ui::rendering`, `current_state`, `file` |
| `SimulationCache` / `CachedBody` (scene data); `SimCache::{to_saved, from_saved, agrees}`; `.g4d` v3 `simulation_cache` (bodies by object index, frame count) and `simulation.bin` (frames × bodies × 7 little-endian `f32`, size checked against the claim and capped at 2 GiB); `grid4d_io::{save_with_cache, load_with_cache}`; `Sim::{to_save, adopt, is_full}`; `SimulationSettings::memory_limit_mb` | `grid4d-scene`, `grid4d-sim`, `grid4d-io`, `grid4d-ui` |
| A `criterion` benchmark: building 1 000 bodies, a frame of 1 000 falling cubes | `crates/sim/benches/falling_cubes.rs` |

## Decisions (2026-10-30)

1. **The cache lives in the editor** (plan D12); saved caches come in 11.5.
2. **A budget per refresh** (8 ms; 40 ms while baking) instead of a worker
   thread: the engine is fast, and a run on the UI thread needs no locking.
3. **One simulation for the viewport and renders,** made from the viewport's
   evaluation (objects visible in the editor) so both show the same motion.

## Acceptance criteria

- [x] Scrubbing forward simulates, backward shows cached frames; the timeline
      shows the cached range.
- [x] Moving a body, changing a tag or the settings drops the cache;
      selecting, renaming or editing a non-body keeps it.
- [x] Bake fills the range with progress and Esc; Clear empties it.
- [x] The Render View, final renders, Current State to Object and export show
      the simulated poses.

11.5:

- [x] A baked simulation saved with the document plays at once when reopened;
      a file whose bodies changed elsewhere simulates again.
- [x] Cache Memory stops the cache with a warning and keeps it when changed.
- [x] A benchmark of 1 000 falling cubes.
