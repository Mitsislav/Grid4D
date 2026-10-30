# Clones as bodies

> Sources (read 2026-10-30): manual — the Dynamics overview (a Rigid Body tag
> on a Cloner makes all its clones dynamic objects; collisions between clones)
> and the Collision tab's Individual Elements. Written in our own words — no
> copied text or images.
>
> Covers implementation-plan step **11.4**. The bodies are described in
> [`simulation-rigid-body.md`](simulation-rigid-body.md), the cache in
> [`simulation-cache.md`](simulation-cache.md), the Cloner in
> [`mograph-cloner.md`](mograph-cloner.md).

## Purpose

Pour, scatter, topple many things at once: a Rigid Body tag on a Cloner makes
**every clone** a body of its own — a grid of cubes collapsing, a thousand
spheres poured into a bowl — and a Collider Body tag makes every clone an
obstacle (a field of posts).

## Behaviour

- **Each clone is one body.** Its shape is what the clone shows — its child's
  shape (generated or deformed as shown), or, for a cloned group, every part
  of the group together. The tag's settings (shape type, bounce, friction,
  mass …) apply to every clone. Clones collide with each other and with every
  other body.
- **The start state** is each clone's placement at the start frame — the
  layout, with the effectors applied — as with any body.
- **Afterwards the simulation moves the clones;** the Cloner, its children and
  its effectors still decide their shapes and colours. Changing the Cloner,
  its children, its effectors or its Object mode object drops the cache, like
  any change to a body.
- **Results:** the viewport, renders, Make Editable, Current State to Object
  and export show the clones where the simulation has them.
- A Dynamics tag on a Cloner's **child** does nothing (the child is not shown
  itself); tag the Cloner.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `Overrides::set_clone(cloner, n, delta)` — a clone moved by a matrix (its parts with it); the Cloner's bounds follow | `grid4d-eval` |
| `BodyKey::Clone(cloner, n)`; a tagged Cloner's clones as bodies (each clone's parts as one body, framed by its first part); `SimCache::overrides` gives each clone its move since the start | `grid4d-sim` |
| The inputs key also holds a tagged Cloner's effectors and Object mode object | `grid4d-ui::simulation` |
| Make Editable with the simulated clones | `grid4d-ui::commands` |

## Decisions (2026-10-30)

1. **A clone's move is applied as a change since the start,** so a clone's
   parts keep their arrangement and nothing about the layout is stored twice.
2. **Animating the Cloner's layout after the start frame** is not
   simulated: each clone's move is added to its placement at the frame, so a
   layout animated past the start shifts the simulated clones with it.
3. **Later:** Individual Elements (bodies per part or per letter), Self
   Collisions off, a Follow Position spring back to the layout.

## Acceptance criteria

- [x] A grid of cubes from a tagged Cloner falls and spreads on a
      floor; each clone is a body; cloned groups move as one.
- [x] Clones collide with each other and with other bodies.
- [x] Changing the Cloner or its effectors re-simulates.
- [x] Renders, Make Editable, Current State to Object and export show the
      simulated clones.
