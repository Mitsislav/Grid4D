# Rigid Body and Collider Body

> Sources (read 2026-10-30): manual — the Rigid Body tag (simulation tags), the
> Dynamics Body tag's Collision tab (Inherit Tag, Shape, Bounce, Friction),
> Mass tab (World / Custom Density, Custom Mass, Rotational Mass) and Force
> tab (damping); the Dynamics overview (frame 0 as the start state, clones as
> bodies) and project Dynamics settings (gravity, steps per frame). Written in
> our own words — no copied text or images. Defaults the manual leaves open are
> ours and marked.
>
> Covers implementation-plan steps **11.1** (the data, the settings, the
> physics crate) and **11.2** (the simulation itself). The cache, playback and
> renders are in `simulation-cache.md` (11.3); clones in
> `simulation-clones.md` (11.4).

## Purpose

Makes objects solid: a **Rigid Body** falls under gravity, collides, bounces,
slides and comes to rest; a **Collider Body** is something rigid bodies hit — a
floor, a bowl, a moving paddle — and is not pushed itself.

## The Dynamics tag

One tag kind, **Dynamics**, with two entries in the Tags menus: *Rigid Body*
(Dynamic) and *Collider Body* (Collider). One per object.

| Name | Type / range | Default (Rigid / Collider) | Effect |
|------|--------------|----------------------------|--------|
| Enabled | on / off | on | Off: the object takes no part in the simulation |
| Dynamic | Dynamic, Collider | Dynamic / Collider | Dynamic: moved by the simulation. Collider: moves only by its own animation (if any) and pushes dynamic bodies, never pushed back |
| Inherit Tag | None, Compound | None | Compound: the object and its visible children are one body (their shapes joined) |
| Shape | Automatic, Box, Sphere, Convex Hull, Static Mesh | Automatic | The collision shape, from the object's shape as shown (deformed, generated). Automatic: a convex hull for dynamic bodies, the mesh itself for colliders. Box: its bounding box; Sphere: the sphere around its bounding box's centre reaching its largest half-size; Static Mesh: the exact mesh (colliders only — a dynamic body falls back to a convex hull) |
| Bounce | ≥ 0 % | 50 % (ours) | How much speed survives a hit (the higher of two bodies' counts) |
| Friction | ≥ 0 % | 30 % (ours) | How much sliding is resisted (the two bodies' values multiplied) |
| Mass | Global Density, Custom Density, Custom Mass | Global Density | Global: density 1 — a 100 cm cube weighs 1 (kg); Custom Density: the Density below; Custom Mass: the Mass below, whatever the size |
| Density | ≥ 0 | 1 | Custom Density only |
| Mass | ≥ 0 kg | 1 kg | Custom Mass only |
| Rotational Mass | 1 – 10 000 % | 100 % | How hard it is to set spinning (moving and colliding unchanged) |
| Linear / Angular Damping | 0 – 100 % | 0 % | Speed lost per second, moving / spinning (air resistance) |
| Initial Velocity | cm/s, X / Y / Z | 0 | The body's speed at the start frame (world axes) |
| Initial Spin | °/s, X / Y / Z | 0 | Its spin at the start frame |

Collider bodies ignore mass, damping and velocities.

## Project settings: Simulation

| Name | Range | Default | Effect |
|------|-------|---------|--------|
| Enabled | on / off | on | Off: nothing is simulated (objects stay where their animation puts them) |
| Gravity | cm/s², X / Y / Z | 0, −981, 0 (ours: Earth) | Pulls every dynamic body |
| Steps per Frame | 1 – 100 | 5 | Sub-steps per frame: more for fast or small bodies (fewer missed collisions), slower |
| Solver Iterations | 1 – 100 | 4 (ours) | Accuracy of resting contacts and stacks |
| Time Scale | 0.01 – 100 | 1 | Slow motion below 1 |

## Behaviour

- **The start state** is the scene at the document's **start frame**: every
  body starts where it is (after animation, Target tags, deformers) with its
  initial velocity. Dynamic bodies keep their own animation only until then.
- **Frame by frame** the simulation advances by `1 / fps × Time Scale`,
  in Steps per Frame sub-steps; collider bodies move to their animated place
  each sub-step (so an animated collider sweeps others along).
- **Shapes** come from what the viewport shows, scaled by the object's scale;
  a body without a surface (a Null without Compound children, a light) takes
  no part.
- **Deterministic:** the same scene gives the same motion every time.
- **Units:** the engine works in metres internally (its tolerances assume
  them); the user sees centimetres, kilograms, seconds.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `Body` (enabled, `Motion`, `Inherit`, `CollisionShape`, bounce, friction, `MassMode`, density, mass, rotational mass, damping, initial velocity and spin), `Body::rigid` / `Body::collider`; `Tag::Dynamics(Body)`, `TagKind::Dynamics` | `scene/src/dynamics.rs`, `scene/src/tag.rs` |
| `SimulationSettings` (`validated`), `Document::simulation`, `SceneEdit::SetSimulation` | `scene/src/dynamics.rs`, `document.rs`, `edit.rs` |
| `.g4d` v3 `dynamics` tag (defaults omitted), `simulation` block (omitted while default) | `grid4d-io::v3` |
| New crate `grid4d-sim`: `rapier3d` 0.36 single-threaded, conversions at its edge (`convert.rs`), metres inside; `World` (gravity, bodies, sub-steps, kinematic moves) | `crates/sim` |
| `Simulation::new(doc, evaluated start frame)`: bodies from tagged visible objects (Compound: their visible children too), shapes from the shown meshes (`shape.rs`: box, sphere, convex hull, trimesh; the world scale baked in), mass properties computed per collider (Custom Mass spread by volume, Rotational Mass dividing the inverse inertia), bounce combined by maximum and friction by product; `step_to(frame)`: sub-steps, animated colliders moved to their keys each sub-step; `SimCache` (frames × bodies of position and rotation, the start scale kept, `world(frame, body)`) | `crates/sim` |
| Tags menus (Rigid Body, Collider Body), Attribute Manager tag rows, Project settings' Simulation section, tag icon | `grid4d-ui` |

## Decisions (2026-10-30)

1. **Two motions, not three:** C4D's collider body follows its animation, so
   "Kinematic" and "Static" are one *Collider* (animated or not). Ghost
   bodies are later.
2. **Shapes:** no ellipsoid or cylinder primitives in the engine (later), a
   Sphere uses the largest half-size; Moving Mesh (a dynamic exact mesh) is
   later — a dynamic body never uses a concave mesh.
3. **Units:** metres inside the engine, centimetres outside (plan D11).
4. **Later:** Apply Tag to Children, Individual Elements, Size Increment and
   Margin, Collision Noise, Custom Center, Follow Position / Rotation,
   triggers (At Velocity Peak, On Collision), Ghost.

## Acceptance criteria

11.1:

- [x] Rigid Body and Collider Body tags added from the Tags menus, edited in
      the Attribute Manager, undoable, saved and loaded.
- [x] Simulation settings edited in the Project settings, validated,
      undoable, saved (omitted while default).
- [x] `grid4d-sim` builds with `rapier3d`; a ball dropped in its world falls
      as gravity says (½ g t²).

11.2:

- [x] A cube falls onto a collider plane and rests on it; a ball bounces lower
      each time; a box slides further on less friction; an animated collider
      pushes a dynamic body; identical runs give identical results.
