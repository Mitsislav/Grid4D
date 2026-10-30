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

## Decisions (2026-10-30)

1. **Built in, listed explicitly** (plan D16): a plugin is a crate the binary
   lists; no libraries are loaded at run time.
2. **Values by name**, so a plugin can add or reorder parameters and old
   files still load; ranges are applied when values are read.
3. **Plugin texts** (names, descriptions, parameter names, hints) are English
   keys translated through the interface's table like Grid4D's own.
4. **Later (13.2–13.3):** deformer and command plugins; animated plugin
   parameters. Not planned: plugins reading child objects as inputs.

## Acceptance criteria

- [x] The Gear is created from Create ▸ Plugins; its parameters edit with
      undo and change the mesh; it renders and exports.
- [x] A document with a plugin Grid4D lacks opens, shows the object as
      missing, and saves back unchanged.
- [x] A plugin's invalid mesh draws nothing and nothing breaks.
