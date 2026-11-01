# Writing a Grid4D plugin

A plugin adds something Grid4D does not have — a kind of object, a deformer
or a command — and makes it look and behave like the built-in ones: it has a
menu entry, its parameters appear in the Attribute Manager, its changes undo,
its objects save and load. This guide shows how to write one. The behaviour
it relies on is specified in
[`reference/extend-plugins.md`](reference/extend-plugins.md).

Grid4D ships three example plugins, each a complete crate to copy from:

| Example | Kind | Crate |
|---------|------|-------|
| Gear | generator: a toothed wheel from its parameters | [`plugins/gear`](../plugins/gear/src/lib.rs) |
| Wave | deformer: ripples the object it is under | [`plugins/wave`](../plugins/wave/src/lib.rs) |
| Scatter | command: copies spread over a surface | [`plugins/scatter`](../plugins/scatter/src/lib.rs) |

## How plugins are added

A plugin is a **Rust crate built into Grid4D**: the application lists its
plugins in [`src/plugins.rs`](../src/plugins.rs) and installs them when it
starts. Nothing is loaded at run time — Grid4D forbids `unsafe` code and Rust
has no stable binary interface between separately built libraries — so
adding a plugin means building Grid4D with it.

1. **Make the crate** in `plugins/<name>/` (copy `plugins/gear/Cargo.toml`
   and change the name and description). A plugin depends only on
   `grid4d-plugin`, and on `grid4d-geom`, `grid4d-scene` and `glam` for the
   types it uses:

   ```toml
   [package]
   name = "grid4d-plugin-pyramid"
   description = "A Grid4D plugin: a pyramid generator."
   version.workspace = true
   edition.workspace = true
   rust-version.workspace = true
   license.workspace = true
   publish.workspace = true

   [dependencies]
   grid4d-plugin.workspace = true
   grid4d-geom.workspace = true
   grid4d-scene.workspace = true
   glam.workspace = true

   [lints]
   workspace = true
   ```

2. **Add it to the workspace** (the root `Cargo.toml`): to `members`, to
   `[workspace.dependencies]` (`grid4d-plugin-pyramid = { path =
   "plugins/pyramid" }`) and to the binary's `[dependencies]`
   (`grid4d-plugin-pyramid.workspace = true`).

3. **Register it**: one line in `src/plugins.rs`, with the method for its
   kind — `add_generator`, `add_deformer` or `add_command`:

   ```rust
   registry.add_generator(grid4d_plugin_pyramid::Pyramid);
   ```

4. **Translate its texts** (below) and run `cargo fmt --all`,
   `cargo clippy --workspace --all-targets -- -D warnings` and
   `cargo test --workspace`.

## What every plugin says: `Plugin`

```rust
use grid4d_plugin::{ParamKind, ParamSpec, Plugin, Unit};
use grid4d_scene::PluginValue;

pub struct Pyramid;

const PARAMS: &[ParamSpec] = &[
    ParamSpec {
        name: "Width",
        kind: ParamKind::Float { min: 0.0, max: 100_000.0, unit: Unit::Length },
        default: PluginValue::Float(200.0),
        hint: "The side of its square base",
    },
    ParamSpec {
        name: "Height",
        kind: ParamKind::Float { min: 0.0, max: 100_000.0, unit: Unit::Length },
        default: PluginValue::Float(150.0),
        hint: "From the base to the tip",
    },
];

impl Plugin for Pyramid {
    fn type_id(&self) -> &'static str { "example.pyramid" }
    fn name(&self) -> &'static str { "Pyramid" }
    fn description(&self) -> &'static str { "A pyramid on a square base" }
    fn params(&self) -> &'static [ParamSpec] { PARAMS }
}
```

- **`type_id`** is saved in every document that uses the plugin, so it must
  be unique and **never change**: `"<author or project>.<name>"`. Grid4D's
  own plugins use `grid4d.`. A second plugin with a type id already taken is
  left out (with a warning in the log).
- **`name`** names the menu entry and new objects ("Pyramid", "Pyramid.1",
  …); **`description`** is the menu entry's tooltip.
- **`params`** lists the parameters in the order the Attribute Manager
  (or, for a command, its options dialog) shows them.

### Parameters

A `ParamSpec` has a `name` (shown, and the key its value is saved under), a
`kind`, a `default` of the matching `PluginValue` and a `hint` (the field's
tooltip; may be empty).

| `ParamKind` | `PluginValue` | Shown as |
|-------------|---------------|----------|
| `Float { min, max, unit }` | `Float(f32)` | `Unit::Length`: in the user's length unit (stored in cm); `Unit::Angle`: degrees (stored in radians); `Unit::Percent`: 0 – 100 % (stored 0 – 1); `Unit::None`: as it is |
| `Int { min, max }` | `Int(i32)` | a whole number |
| `Bool` | `Bool(bool)` | a check box |
| `Choice(&["A", "B"])` | `Choice(u32)` (an index) | a drop-down list |
| `Vector { unit }` | `Vector(Vec3)` | three fields |
| `Color` | `Color([f32; 3])` (linear RGB, 0 – 1) | a colour swatch |

The document keeps an object's values **by name**. A plugin reads them
through `Values`, which always gives a value in range: a missing value (an
object saved before the parameter existed) reads as its default, a value of
the wrong type as the default, a number outside the range is clamped. So:

- you may **add** parameters freely — old files read the defaults;
- **renaming** a parameter loses what users set for it (the old value stays
  in the file, unused): keep names stable;
- values a plugin no longer describes are kept and saved back.

```rust
fn read(values: &grid4d_plugin::Values<'_>) {
    let width = values.float("Width");   // f32, in range
    let teeth = values.int("Teeth");     // i32
    let holes = values.bool("Hole");     // bool
    let shape = values.choice("Shape");  // u32 index
    let size = values.vector("Size");    // Vec3
    let tint = values.color("Tint");     // [f32; 3]
}
```

Every `Float` parameter can be **animated** (a key button on the Object tab,
a curve in the F-Curve editor, the first 64 values of an object) and
**overridden in a take**; nothing more is needed from the plugin. Choices,
check boxes, whole numbers, vectors and colours are not keyed.

## Generators: `GeneratorPlugin`

A generator makes a kind of object from its parameters, like a primitive.
**Create ▸ Plugins** lists it; evaluation calls `build`:

```rust
use glam::Vec3;
use grid4d_geom::PolyMesh;
use grid4d_plugin::{GeneratorPlugin, Values};

impl GeneratorPlugin for Pyramid {
    fn build(&self, values: &Values<'_>) -> PolyMesh {
        let w = values.float("Width") * 0.5;
        let h = values.float("Height");
        let points = vec![
            Vec3::new(-w, 0.0, w),
            Vec3::new(w, 0.0, w),
            Vec3::new(w, 0.0, -w),
            Vec3::new(-w, 0.0, -w),
            Vec3::new(0.0, h, 0.0),
        ];
        // Faces counter-clockwise seen from outside: the base, then the sides.
        PolyMesh::from_faces(points, [&[3, 2, 1, 0][..], &[0, 1, 4], &[1, 2, 4], &[2, 3, 4], &[3, 0, 4]])
            .unwrap_or_default()
    }
}
```

- The mesh is in the **object's own space**: centimetres, Y up, faces
  counter-clockwise seen from outside (their normals point out). Polygons
  may have any number of corners.
- **`build` must not panic**: return what you can (`unwrap_or_default()`
  gives an empty mesh, which draws nothing). Points that are not finite
  numbers make Grid4D ignore the mesh and log a warning.
- Grid4D **caches** the mesh per object and builds it again only when a
  value changes, so `build` may take its time — but it runs again on every
  change while a field is dragged.
- The result is drawn, picked, deformed, rendered and exported like a polygon
  object's mesh; **Make Editable** (C) turns the object into one.

## Deformers: `DeformerPlugin`

A deformer reshapes, like the built-in deformers, the object it is under and
everything below that object. Create ▸ Plugins lists it after the
generators; with Shift it goes under the selected object, its box fitted.

```rust
use glam::Vec3;
use grid4d_plugin::{DeformerPlugin, Values};

impl DeformerPlugin for Lift {
    /// Its Vector parameter (a length) that is its box's size.
    fn size_param(&self) -> &'static str { "Size" }
    /// The Float parameter its handle sets (optional).
    fn handle_param(&self) -> Option<&'static str> { Some("Lift") }
    fn deform(&self, values: &Values<'_>, points: &mut [Vec3]) {
        let lift = values.float("Lift");
        for p in points {
            p.y += lift;
        }
    }
}
```

- `deform` gets **all the points of an object at once**, in the deformer's
  own space (its origin at the centre of its box), and moves them in place.
  Read the values once, before the loop.
- **`size_param`** names a `Vector { unit: Unit::Length }` parameter: the
  box drawn around the deformer (deformed by itself) and the size Fit to
  Parent sets.
- **`handle_param`** (optional) names a `Float` parameter. Its handle sits
  above the centre of the box's top, as high as the value — a length as it
  is, a fraction or plain number times half the box's height, an angle so
  that 90° is half the height — and dragging it up and down sets the value.
- Points that are not finite numbers after `deform` leave the object
  undeformed (and a warning in the log). `deform` must not panic.

## Commands: `CommandPlugin`

A command changes the document once, when chosen in the **Plugins** menu.
Its parameters are options, asked for in a dialog first (none: it runs at
once). It reads the document and returns **`Changes`**; Grid4D applies them
as **one undo step**, all or nothing, and selects the new top-level objects.

```rust
use grid4d_plugin::{Changes, CommandPlugin, Context, Parent, Values};
use grid4d_scene::{NodeData, ObjectKind};

impl CommandPlugin for Pair {
    fn run(&self, context: &Context<'_>, options: &Values<'_>) -> Result<Changes, String> {
        if context.selection.is_empty() {
            return Err("Select something first".into());
        }
        let mut changes = Changes::default();
        let group = changes.insert(Parent::Root, NodeData::new("Pair", ObjectKind::Null));
        changes.insert(Parent::New(group), NodeData::new("Inside", ObjectKind::Null));
        Ok(changes)
    }
}
```

What a command can read, through `Context`:

| | |
|-|-|
| `context.document` | the whole document (`document.scene` for objects) |
| `context.selection` | the selected objects, in the order they were selected |
| `context.active` | the active object (usually the one selected last) |
| `context.frame` | the current frame |
| `context.mesh(id)` | an object's shape in its own space at the frame — a primitive's, a polygon object's, a generator's result or a plugin's mesh (not deformed) |
| `context.world_matrix(id)` | an object's placement in the world at the frame |

What it can return, in `Changes` (applied in order):

- `insert(parent, data)` adds an object — under `Parent::Root`,
  `Parent::Object(id)` (an object of the document) or `Parent::New(new)`,
  an object the command added before (new objects have no ids yet, so
  `insert` returns a `NewObject` to refer to them);
- `edit(scene_edit)` any other edit of the document: a `SceneEdit::SetData`
  for an object's new data, `Move`, `Detach` (delete), materials, layers….

When it **cannot run**, it returns `Err` with an English sentence saying
why; Grid4D shows it (translated) and changes nothing. A command never
changes the document itself — which is what keeps undo, takes (Auto Take)
and the all-or-nothing rule working for plugins as for Grid4D's own
commands. Scripts can run commands too (`run_command`, see
[`scripting.md`](scripting.md)).

## Objects whose plugin is missing

A document that uses a plugin the running Grid4D does not have still
opens: the object keeps its place, its values and its keys, shows nothing
(it acts as a group: its children are drawn) and its Object tab names the
missing plugin. Saving writes it back unchanged, so the file works again
where the plugin is installed. This is why a type id must never change.

## Translations

Grid4D's interface is in English or Greek, and a plugin's texts are
translated like Grid4D's own: the English text is the key, and its Greek is
an entry in [`crates/ui/src/i18n/el.rs`](../crates/ui/src/i18n/el.rs)
(sorted by the English, byte order). A test reads the plugin crates and
lists every text without Greek (it writes the list to
`grid4d-missing-greek.txt` in the temporary folder). It finds:

- the bodies of `fn name()` and `fn description()`;
- `name: "…"` and `hint: "…"` in parameter descriptions;
- choices written in the `Choice(&[…])` itself (not in a separate constant);
- a command's messages written in `Err(…)` (not in a separate constant).

## Testing a plugin

Test a plugin directly, without the interface: make an object with its
defaults and call it.

```rust
#[cfg(test)]
mod tests {
    use grid4d_plugin::{GeneratorPlugin, Registry, Values};
    use grid4d_scene::PluginValue;

    use super::*;

    #[test]
    fn a_taller_pyramid_reaches_higher() {
        let mut object = Registry::new_object(&Pyramid);
        object.set("Height", PluginValue::Float(300.0));
        let mesh = Pyramid.build(&Values::new(Pyramid.params(), &object));
        assert_eq!(mesh.face_count(), 5);
        let (_, max) = mesh.bounds().unwrap();
        assert!((max.y - 300.0).abs() < 1e-3);
    }
}
```

Worth testing, as the examples do: the faces point outwards
(`plugins/gear`), values out of range or missing still give a usable result,
and a command's messages and its result for a fixed seed (`plugins/scatter`
builds a small document and a `Context` with its own `shapes` function).

A test that goes through Grid4D (evaluation, commands) needs the plugin
installed. The registry is installed once per test program, so tests share
one installation:

```rust
fn plugins() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let mut registry = grid4d_plugin::Registry::default();
        registry.add_generator(grid4d_plugin_pyramid::Pyramid);
        grid4d_plugin::install(registry);
    });
}
```

## Checklist

- [ ] A crate in `plugins/`, in the workspace, a dependency of the binary,
      registered in `src/plugins.rs`.
- [ ] A stable, namespaced type id; parameter names you will keep.
- [ ] Every parameter has a sensible range and default.
- [ ] `build` / `deform` / `run` never panic.
- [ ] Greek for its name, description, parameter names, hints, choices and
      messages.
- [ ] Tests; `cargo fmt`, `cargo clippy -- -D warnings` and `cargo test`
      pass (the workspace's lints apply: no `unwrap()` outside tests, no
      `unsafe`).
