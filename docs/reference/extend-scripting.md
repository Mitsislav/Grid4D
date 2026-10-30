# Scripting

> Sources (read 2026-10-30): the manual's Script Manager and the scripting
> SDK overview (a script is a small program run once on the open document,
> reading and changing its objects through an object model; changes it
> makes can be undone; output goes to a console). Written in our own words;
> Grid4D's script API is its own design, in Rhai, not a port of any SDK.
>
> Covers implementation-plan steps **13.4** (the core) and 13.5 (the Script
> Manager).

## Purpose

Automate repetitive work — build, arrange, rename, recolour many objects —
with a short program instead of many clicks; each run is one undo step.

## Behaviour (13.4)

- **A script** is text in [Rhai](https://rhai.rs) (a small language with
  Rust-like syntax: `let`, `for x in 0..10`, `if`, functions, arrays,
  maps). It runs once on the open document, from top to bottom.
- **One undo step.** Whatever a script changes becomes one step named "Run
  Script": Edit ▸ Undo takes all of it back. A script that stops with an
  error changes nothing at all — not even what it did before the error.
  Changes are recorded in the active take like any edit (Auto Take).
- **Errors** say what went wrong and where: "Line 3, column 9: …".
- **Endless loops** cannot hang Grid4D: a script is stopped after a large
  number of operations (100 million) with a message saying so, and changes
  nothing.
- **No access to files or the network**, nor to anything outside the
  document.
- **Output:** `print(…)` lines are collected and shown when the script ends
  (in 13.5, in the Script Manager's console).
- **Running a script file** (13.4): Script ▸ Run Script File… picks a
  `.rhai` file and runs it; its output, or its error, is then shown in a
  message.

### The document API

Values are as the Attribute Manager shows them: lengths in centimetres
(whatever the length unit of the Preferences), angles in degrees,
percentages as 0 – 100; positions, rotations (H, P, B), scales and colours
are arrays of three numbers; colours are 0 – 1 (linear). Objects are values
of type `Object`; an object that has been deleted gives an error when used.

| Function | Gives / does |
|----------|--------------|
| `objects()` | every object, in Object Manager order (parents before children) |
| `roots()` | the top-level objects |
| `find(name)` | the first object with that name, or `()` when there is none |
| `selection()` | the selected objects, in selection order |
| `select(object)`, `select([objects])` | selects them (the last one active) after the script |
| `frame()` | the current frame (a whole number) |
| `create(kind)`, `create(kind, parent)` | a new object of a kind by its name as in the Create menu ("Cube", "Sphere", "Null", "Bend", "Cloner", …) or of a plugin ("Gear", or its id "grid4d.gear"), named like the menu's, at its default; at the top or last under `parent` |
| `delete(object)` | deletes it with everything under it |
| `run_command(name)`, `run_command(name, #{ option: value, … })` | runs a plugin command ("Scatter" or its id) on the script's selection with these options (the rest at their defaults); gives the new top-level objects; its message stops the script |
| `print(value)` | a line of output |

| Property / method of an `Object` | |
|-------------------------------|-|
| `name` | get / set |
| `kind` | its kind's name ("Cube", "Gear", …), read only |
| `position`, `rotation`, `scale` | get / set, in its parent's space |
| `color` | display colour, get / set |
| `visible`, `visible_in_render`, `enabled` | get / set |
| `parent` | get (`()` at the top) / set (an object or `()`: moved, keeping its place in the world) |
| `children` | the objects directly under it |
| `material` | the name of its material, or `""`; set by a material's name (`""`: none) |
| `layer` | the name of its layer, or `""`; set by a layer's name (`""`: none) |
| `get(name)`, `set(name, value)` | a parameter by its name on the Object tab: the numbers that can be keyed ("Radius", "Size . X", …) and every value of a plugin object ("Teeth": numbers, true / false, a choice's index or name, [x, y, z]); names are matched without regard to case; one an object lacks is an error |

Animated values: setting a value that has keys changes the value under them
(as without Autokeying); the keys stay.

## Behaviour (13.5)

- **The script library** is a folder of `.rhai` files in the user data
  folder (beside the recovery copies: `…/grid4d/data/scripts`). On the first
  start it is made with a few example scripts: Grid of Cubes, Number
  Selected Objects, Spread Selected Objects, Random Colours, Scatter on the
  Last Selected. Scripts are listed by file name, without `.rhai`, in
  alphabetical order.
- **Script ▸** lists the library's scripts (a click runs one), then
  **Script Manager…** and **Run Script File…** (any `.rhai` file).
- **The Script Manager** (a window) edits one script at a time:
  - its name and toolbar: **New** (an empty script with a hint), **Open**
    (a list of the library's scripts), **Save** (into the library under the
    name typed in the Name field; a name may not contain `\ / : * ? " < > |`),
    **Run**; the title shows the name and `*` while there are unsaved
    changes;
  - the editor: monospace text with line numbers beside it;
  - **Ctrl+Enter** runs the script while the editor has the keyboard;
  - the **console** below: what scripts printed and their errors (in the
    warning colour, with line and column), the newest at the bottom;
    **Clear** empties it.
- **While the Script Manager is open**, output and errors of every script
  run (from it or from Script ▸) go to its console; while it is closed they
  are shown in a message, as in 13.4.
- Saving, opening or listing the library never changes the document.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| The engine (limits, no `eval`), the API, `run(document, selection, frame, shapes, source) → Result<Outcome, ScriptError>`: works on a copy of the document, applying `SceneEdit`s and keeping their inverses | `grid4d-script` (new; depends on geom, scene, plugin; `rhai` 1.27) |
| `Changes::apply` (a command's changes on a document, all or nothing), shared by scripts and the interface | `grid4d-plugin` |
| `EditorCommand::RunScript` (the outcome becomes the document, one undo step, the selection), Script ▸ Run Script File… (`FileDialogs::pick_script`), the output or error shown | `grid4d-ui` |
| `Scripts` (the library folder, its listing, the examples installed when it is made; the edited script; the console), `EditorState::scripts`; `EditorCommand::{NewScript, OpenScript, SaveScript, RunLibraryScript}` | `grid4d-ui::scripts`, `commands` |
| The Script Manager window | `grid4d-ui::panels::script_manager` |
| The example scripts | `crates/ui/scripts/*.rhai` (built in) |
| The library's folder | the binary (`<storage dir>/scripts`) |

## Decisions (2026-10-30)

1. **Rhai** (plan D17): pure Rust, sandboxed, no installation.
2. **A copy, then a swap** (plan D18): a script works on a copy of the
   document; only when it finishes does the copy become the document, with
   the inverses of its edits as the undo step. An error leaves the
   document untouched.
3. **Display units, not stored ones,** so a script uses the numbers the user
   sees (degrees, percent, centimetres).
4. **Not planned in Phase 13:** scripts that run every frame; keys and
   animation from scripts; creating materials or layers; syntax colouring and
   completion in the editor; sub-folders in the library.

## Acceptance criteria

- [x] A script that builds and colours a grid of cubes undoes in one step.
- [x] A failing script changes nothing and reports line and column.
- [x] An endless loop is stopped.
- [x] A script calls a plugin command (Scatter).
- [x] The library is made with the examples on the first start; every
      example runs without error on the demo scene.
- [x] The Script Manager runs its script (also with Ctrl+Enter), shows output
      and errors in its console, saves into and opens from the library.
