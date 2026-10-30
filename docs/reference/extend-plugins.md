# Plugins

> Sources (read 2026-10-30): the SDK documentation — object plugins (a
> generator object builds its geometry from its own parameters; a parameter
> description lists each parameter's type, range, unit and default, from
> which the Attribute Manager is laid out; plugins register under a unique
> id) and the manual's plugin menu. Written in our own words; Grid4D's API is
> its own design, not a port of any SDK.
>
> Covers implementation-plan steps **13.1** (generators), 13.2 (deformers)
> and 13.3 (commands).

## Purpose

Add new kinds of objects (and, later, deformers and commands) to Grid4D
without changing its core, so they look and behave like the built-in ones.

## Behaviour (13.1)

- **Create ▸ Plugins** lists the installed generator plugins by name, each
  with a short description as its tooltip. Choosing one creates its object
  (named after the plugin) at the top of the Object Manager, selected, as
  one undo step.
- **Its parameters** appear on the Attribute Manager's Object tab, laid out
  from the plugin's description: lengths in the length unit of the
  Preferences, angles in degrees, percentages, whole numbers, check boxes,
  choices, vectors and colours, each within its range. Every change is an
  undo step (one per drag), recorded in the active take like any parameter.
- **It is an object like any other:** it draws, picks, frames, renders and
  exports from the mesh the plugin builds; deformers bend it; materials,
  layers, tags and the hierarchy apply; **Make Editable** (C) turns it into a
  polygon object with that mesh; Current State to Object copies it.
- **A missing plugin.** A document using a plugin this Grid4D does not have
  still opens. The object stays where it was with its values, draws nothing
  (it acts as a group), and its Object tab says which plugin is missing and
  lists the values it keeps. Saving writes the object back unchanged, so the
  file still works where the plugin is installed. Values the plugin does not
  (or no longer) describe are kept too; described values a file lacks take
  their defaults.
- **A plugin that fails** — a mesh with points that are not finite numbers,
  or faces pointing past its points — draws nothing; Grid4D keeps running
  and writes a warning to its log.
- **The Gear** (an example plugin that ships with Grid4D): a toothed wheel —
  Teeth, Radius, Tooth Depth, Tooth Width (percent of the pitch), Thickness,
  Hole Radius (0: no hole).

## Behaviour (13.2)

- **Deformer plugins** reshape, like the built-in deformers, the object they
  are under and its subtree: they take their place in the same chain
  (deepest first, siblings in order), inside generators' inputs too, and
  switch off with the object or its layer's Deformers switch. Create ▸
  Plugins lists them after the generators, below a line; with **Shift** one
  goes under the active object, its box fitted to it (as Fit to Parent,
  which their Object tab also has).
- **Their box** is drawn and picked like a built-in deformer's, deformed by
  the deformer itself; its size is the plugin's own Size parameter.
- **A handle**, when the plugin names a parameter for it, sits above the
  centre of the box's top, as high as the value (a length as it is; a
  fraction or plain number times half the box's height; 90° at half the
  box's height). Dragging it up and down sets the value (Shift: 5°, 5 % or
  whole numbers), one undo step per drag, Esc restores.
- **A deformer that fails** (points that are not finite numbers) leaves the
  object as it was; Grid4D writes a warning to its log.
- **Animated plugin parameters:** every number of a plugin object is keyed
  like a built-in parameter — key buttons on the Object tab (Autokeying,
  Record), a curve in the F-Curve editor named by the plugin's name for it
  and shown in its unit (degrees, percent). Files keep tracks by that name.
  Choices, check boxes, whole numbers, vectors and colours are not keyed.
- **Takes** override plugin parameters like any object parameter
  (Parameters group).
- **The Wave** (an example deformer): moves points up and down along a sine
  wave — Size, Amplitude (the handle), Wavelength, Phase, Direction (Along
  X, Along Z, Radial), Within Box (only points inside the box move).

## Behaviour (13.3)

- **Command plugins** do something to the document once, when chosen —
  like a menu command. The **Plugins** menu (in the menu bar, before
  Window) lists them by name, the description as the tooltip; "No commands
  installed" when there are none.
- **Options:** a command with parameters opens a small dialog laid out from
  its description, like the Object tab (lengths in the length unit,
  angles, percent, whole numbers, check boxes, choices, vectors, colours;
  hints as tooltips), with **OK** and **Cancel**. The values are remembered
  for the next time in this session. A command without parameters runs at
  once.
- **What it may do:** read the document (objects, their placement and
  shapes at the current frame, the selection and the active object) and
  return changes: new objects (also under other new ones), and any edit an
  undo step can hold. They become **one undo step** named after the
  command, all or nothing; the new top-level objects are selected.
- **When it cannot run** it says why in a message (translated like
  everything else) and changes nothing — e.g. nothing selected.
- **The Scatter** (an example command): copies of the selected objects (each
  with everything under it) spread at random over the surface of the
  **active** object (select it last), as many as Count, under a new Null
  named "Scatter". Each copy goes to a point chosen evenly over the
  surface's area, picks one of the objects at random, stands up along the
  surface (Align to Surface), turns about its up axis by up to Random
  Rotation and is scaled by up to ± Scale Variation. The same Seed always
  gives the same result. The surface is the object as its own shape at the
  current frame (a primitive, a polygon object, a generator's result, a
  plugin's mesh), placed where it is; deformers are not taken into account.
  Without a surface with polygons, or with only the surface selected, it
  says what to select.

## Grid4D mapping

| Piece | Where |
|-------|-------|
| `PluginObject` (type id, named `PluginValue`s), `ObjectKind::Plugin`, `ObjectKindTag::Plugin` | `grid4d-scene::plugin` |
| The plugin API: `ParamSpec`, `ParamKind`, `Unit`, `Values` (typed reads with defaults and ranges), `GeneratorPlugin`, `Registry`, `install` / `registry` (installed once) | `grid4d-plugin` (depends on geom and scene) |
| Generator plugins evaluated, cached per object and values, validated | `grid4d-eval` |
| `KindDto::Plugin` (type id, values by name; unknown types and values kept) | `grid4d-io::v3` |
| The Object tab from a description; the missing-plugin note; **Create ▸ Plugins**; `EditorCommand::CreatePlugin` | `grid4d-ui` |
| The registration list | `src/plugins.rs` (the binary) |
| The Gear | `plugins/gear` (`grid4d-plugin-gear`) |
| `Plugin` (what every plugin says), `DeformerPlugin` (size and handle parameters, `deform` on points in its space), `Registry::{add_deformer, deformer, plugin, plugins}`; the box, handle and checked deformation | `grid4d-plugin` |
| `ObjectParam::Plugin(index)` (a plugin object's value by its place, the first 64), `ObjectKind::param_label`, `NodeData::param_label` | `grid4d-scene::param`, `anim` |
| `Deformer::Plugin` in the chain (whole meshes at once), `is_deformer` | `grid4d-eval::deform` |
| `ParamDto::Plugin(name)` | `grid4d-io::v3` |
| The deformer section of Create ▸ Plugins, `CreatePlugin { child }`, Fit to Parent, key buttons, the handle, F-Curve names and units | `grid4d-ui` |
| The Wave | `plugins/wave` (`grid4d-plugin-wave`) |
| `CommandPlugin` (`run(Context, Values) → Result<Changes, message>`), `Context` (document, selection, active object, frame, shapes), `Changes` (inserts under existing or new objects, `SceneEdit`s) | `grid4d-plugin` |
| The Plugins menu, the options dialog (`panels/plugin_command.rs`), `EditorCommand::RunPlugin`, applying `Changes` as one step | `grid4d-ui` |
| The Scatter | `plugins/scatter` (`grid4d-plugin-scatter`) |

## Decisions (2026-10-30)

1. **Built in, listed explicitly** (plan D16): a plugin is a crate the binary
   lists; no libraries are loaded at run time.
2. **Values by name**, so a plugin can add or reorder parameters and old
   files still load; ranges are applied when values are read.
3. **Plugin texts** (names, descriptions, parameter names, hints) are English
   keys translated through the interface's table like Grid4D's own.
4. **Deformer plugins move whole lists of points** in their own space, so a
   plugin reads its values once per object, not once per point.
5. **Keys follow a value's place** in the object's list (names in files):
   values are only ever added at the end, so the place never changes.
6. **Plugin objects share the Generator entry of the views' Filter** (the
   filter goes by kind of object; a plugin's kind is not known there).
7. **Commands return changes, never touch the document:** Grid4D applies
   them, so undo, takes (Auto Take) and the all-or-nothing rule hold for
   plugins as for its own commands. New objects are referred to by the
   order they were added, since they have no ids before they exist.
8. **Shapes come from Grid4D** (`Context::mesh`): the API crate cannot
   evaluate the scene, so the interface hands commands the shapes it
   evaluates.
9. Not planned: plugins reading child objects as inputs, commands that
   take pointer input in the views.

## Acceptance criteria

- [x] The Gear is created from Create ▸ Plugins; its parameters edit with
      undo and change the mesh; it renders and exports.
- [x] A document with a plugin Grid4D lacks opens, shows the object as
      missing, and saves back unchanged.
- [x] A plugin's invalid mesh draws nothing and nothing breaks.
- [x] The Wave, under a plane, ripples it; its box and handle show; keys on
      its Phase move the ripples; undo removes it; a take overrides its
      Amplitude.
- [x] Plugin numbers have key buttons and F-Curves under their own names;
      their keys are saved and load back.
- [x] Plugins ▸ Scatter… opens its options; OK spreads copies over the
      active object as one undo step; the same seed gives the same result.
- [x] With nothing (or only the surface) selected, it says what to select
      and changes nothing.
