# Workspace layouts

> Sources (read 2026-10-30): manual — the interface overview (managers docked
> around the viewport, tabbed managers, dragging a manager's tab to dock it
> elsewhere, the Layout menu with its preset layouts, resetting a layout).
> Written in our own words — no copied text or images. The presets are our own
> arrangements, not copies of the reference's.
>
> Covers implementation-plan step **12.1**.

## Purpose

Arrange the window to suit the work: put managers side by side or in tabs,
make the viewport larger, bring the curves up for animation — and get a known
arrangement back with one click.

## Behaviour

- **Panes.** The viewport and the managers are panes: Viewport, Objects,
  Takes, Attributes, Coordinates, Layers, Timeline, F-Curve, Materials. Every
  pane except the Viewport sits in a group of tabs; its tab shows its name.
  The menu bar, the command palette, the mode palette and the status bar stay
  where they are.
- **Rearranging.** Drag a pane's tab:
  - onto another group's tab bar — it joins that group as a tab;
  - onto the edge of a pane — the pane splits and the dragged one takes that
    side.
  Drag the line between two panes to resize them.
- **Closing.** A tab's **×** closes that pane (not the Viewport). **Window ▸**
  *pane name* opens it again — as a tab beside the panes it usually sits with —
  or, if it is open, brings its tab to the front.
- **Layouts.** The **Layout** menu at the right of the menu bar switches the
  whole arrangement:
  - **Standard** — the Object Manager over the Attribute Manager on the right,
    the timeline under the viewport (as Grid4D has always looked);
  - **Modeling** — a wider viewport, the Attribute and Coordinate Managers
    side by side under the Object Manager, the Material Manager as a tab of the
    Object Manager, a short timeline;
  - **Animate** — the F-Curve editor tall under the viewport, with the timeline
    and Material Manager as its tabs.
  **Window ▸ Reset Layout** puts the current layout back as it was before any
  dragging.
- **Remembered.** The arrangement (and which layout it came from) is saved
  with the window and comes back on the next start; closed panes stay closed.
  A saved arrangement that holds a pane twice or nothing at all (from another
  version) gives the layout's preset back; one without the Viewport gets it
  back.
- **Shortcuts that show a manager** — Shift+F2 (Material Manager), Shift+F5
  (Attribute Manager), Shift+F7 (Coordinate Manager), Tab over the timeline or
  the F-Curve editor (the other one) — bring its tab to the front, opening it
  if it was closed. Keys meant for a manager (Delete in the F-Curve editor, the
  timeline's keys, the Material Manager; Ctrl+N there; H / S over the curves)
  act on the pane under the pointer or the one last clicked, if it is showing.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `Pane`, `PaneState` (panes shown this frame, the one under the pointer, a request to show one, a layout request, a reset); `EditorState::panes` | `ui::workspace`, `ui::editor` |
| `Workspace` (an `egui_tiles::Tree<Pane>` and its `Layout`): presets, `show_pane`, `use_layout`, `repaired`; a `Behavior` that draws each pane's panel and styles the tabs | `ui::workspace` |
| `Panels` (the docked panels together) | `ui::workspace` |
| Saved by the binary in eframe storage (`Workspace` is serialisable) | `src/app.rs` |
| The Layout menu, Window ▸ panes and Reset Layout | `ui::panels::menu_bar` |

## Decisions (2026-10-30)

1. **`egui_tiles`** does the docking (plan D13); its tab bars are styled with
   our palette.
2. **Every pane is in exactly one place;** presets contain all nine (closed
   panes are simply not in the tree).
3. **The viewport keeps no tab bar** while it is alone in its group, as in
   the reference; it cannot be closed.
4. **Later:** floating panes in their own windows, user-saved layouts.

## Acceptance criteria

- [x] Panes dock as tabs or splits by dragging their tabs, resize by their
      dividing lines, close by their **×** (not the Viewport) and reopen from
      the Window menu.
- [x] Standard, Modeling and Animate each hold every pane once; Reset Layout
      restores the current one.
- [x] The arrangement survives a restart; a broken saved arrangement is
      repaired.
- [x] Shift+F2 / F5 / F7 and Tab bring their panes to the front; Delete,
      Ctrl+N, H and S still reach the right manager.
