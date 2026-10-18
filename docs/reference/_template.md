# <Feature name>

> Sources: <manual page URL>, <resource-index / SDK URL>. Read on <YYYY-MM-DD>,
> Cinema 4D <version>. Written in our own words — no copied text or images.

## Purpose

What the user achieves with it, in one or two sentences.

## Parameters

| Name | Type / range | Default | Effect |
|------|--------------|---------|--------|
|      |              |         |        |

## Behaviour

- Interaction: how it is invoked (menu, shortcut, viewport handles), what the user
  sees while using it.
- Result: what changes in the scene (new object, edited mesh, hierarchy, selection).
- Edge cases: empty selection, degenerate geometry, extreme values, undo.

## Grid4D mapping

Which crate / type / `EditorCommand` / `SceneEdit` it touches, and anything we
deliberately do differently (and why).

## Acceptance criteria

- [ ] Testable statement …
- [ ] Undo/redo restores the exact previous state.
