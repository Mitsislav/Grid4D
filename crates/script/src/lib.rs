//! # grid4d-script
//!
//! Scripts in [Rhai](https://rhai.rs) that read and change a document (see
//! `docs/reference/extend-scripting.md`). [`run`] works on a copy of the
//! document: every change is a `SceneEdit` applied to the copy, its inverse
//! kept. When the script finishes, the copy is the new document and the
//! inverses undo it in one step; an error (or the operations limit) leaves the
//! caller's document as it was.
//!
//! Values are as the Attribute Manager shows them: centimetres, degrees,
//! percent; vectors and colours are arrays of three numbers.

use std::{cell::RefCell, rc::Rc};

use glam::Vec3;
use grid4d_geom::PolyMesh;
use grid4d_plugin::{Context, ParamKind, Plugin, Unit, Values};
use grid4d_scene::{Document, NodeData, NodeId, ObjectKind, ObjectKindTag, PluginValue, Scene, SceneEdit, Tag};
use rhai::{Array, Dynamic, Engine, EvalAltResult, Map, Position};

/// A script is stopped after this many operations (an endless loop).
pub const MAX_OPERATIONS: u64 = 100_000_000;

/// How Grid4D gives an object's shape in its own space at a frame (for
/// plugin commands a script runs).
pub type Shapes = fn(&Scene, NodeId, f64) -> Option<PolyMesh>;

/// What a finished script did.
#[derive(Debug)]
pub struct Outcome {
    /// The document as the script left it.
    pub document: Document,
    /// The inverses of its edits, in undo order; empty if it changed nothing.
    pub inverses: Vec<SceneEdit>,
    /// What it printed, one line per `print`.
    pub output: Vec<String>,
    /// The selection it asked for (`select`), if any.
    pub selection: Option<Vec<NodeId>>,
}

/// Why a script stopped, and what it printed before.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptError {
    /// What went wrong (English).
    pub message: String,
    /// Where, if known (1-based).
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub output: Vec<String>,
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.line, self.column) {
            (Some(line), Some(column)) => write!(f, "Line {line}, column {column}: {}", self.message),
            (Some(line), None) => write!(f, "Line {line}: {}", self.message),
            _ => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for ScriptError {}

/// An object of the document, as scripts see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Object(NodeId);

/// The document copy a script works on, and what it did.
struct State {
    document: Document,
    /// In the order they were applied.
    inverses: Vec<SceneEdit>,
    selection: Vec<NodeId>,
    selection_changed: bool,
    frame: f64,
    shapes: Shapes,
}

type Shared = Rc<RefCell<State>>;
type Result<T> = std::result::Result<T, Box<EvalAltResult>>;

fn error<T>(message: impl Into<String>) -> Result<T> {
    Err(Box::new(EvalAltResult::ErrorRuntime(
        Dynamic::from(message.into()),
        Position::NONE,
    )))
}

impl State {
    /// `id`'s data, or an error if it was deleted.
    fn data(&self, id: NodeId) -> Result<&NodeData> {
        match self.document.scene.get(id) {
            Some(node) => Ok(&node.data),
            None => error("the object no longer exists"),
        }
    }

    /// Applies `edit` to the copy, keeping its inverse.
    fn apply(&mut self, edit: SceneEdit) -> Result<SceneEdit> {
        match self.document.apply(edit) {
            Ok(inverse) => {
                self.inverses.push(inverse.clone());
                Ok(inverse)
            }
            Err(err) => error(err.to_string()),
        }
    }

    /// Changes `id`'s data with `change` (one edit; none if nothing changed).
    fn edit(&mut self, id: NodeId, change: impl FnOnce(&mut NodeData) -> Result<()>) -> Result<()> {
        let mut data = self.data(id)?.clone();
        change(&mut data)?;
        if &data != self.data(id)? {
            self.apply(SceneEdit::SetData { id, data })?;
        }
        Ok(())
    }
}

/// Three numbers from a script's array.
fn vector(value: &Array) -> Result<Vec3> {
    let numbers: Vec<f32> = value.iter().filter_map(number).collect();
    match numbers.as_slice() {
        &[x, y, z] => Ok(Vec3::new(x, y, z)),
        _ => error("expected an array of three numbers, like [0, 100, 0]"),
    }
}

/// A number (an integer or a float) as an `f32`.
#[expect(clippy::cast_possible_truncation, reason = "script numbers become the scene's f32s")]
#[expect(clippy::cast_precision_loss, reason = "script integers are small")]
fn number(value: &Dynamic) -> Option<f32> {
    value
        .as_float()
        .map(|v| v as f32)
        .or_else(|_| value.as_int().map(|v| v as f32))
        .ok()
        .filter(|v| v.is_finite())
}

fn array(v: Vec3) -> Array {
    v.to_array().into_iter().map(|x| Dynamic::from(f64::from(x))).collect()
}

/// The name scripts know `kind` by.
fn kind_name(kind: &ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Plugin(object) => grid4d_plugin::registry()
            .plugin(&object.type_id)
            .map_or(kind.label(), Plugin::name),
        _ => kind.label(),
    }
}

/// A new object's data: a kind from the Create menu or a generator or
/// deformer plugin, by name (or plugin id).
fn new_object(scene: &Scene, kind: &str) -> Result<NodeData> {
    if let Some(tag) = ObjectKindTag::ALL
        .into_iter()
        .find(|tag| tag.label().eq_ignore_ascii_case(kind))
    {
        return Ok(NodeData::new(scene.unique_name(tag.label()), tag.default_kind()));
    }
    let registry = grid4d_plugin::registry();
    let plugin = registry
        .plugins()
        .filter(|p| registry.command(p.type_id()).is_none())
        .find(|p| p.name().eq_ignore_ascii_case(kind) || p.type_id() == kind);
    match plugin {
        Some(plugin) => Ok(NodeData::new(
            scene.unique_name(plugin.name()),
            ObjectKind::Plugin(grid4d_plugin::Registry::new_object(plugin)),
        )),
        None => error(format!("there is no kind of object called \"{kind}\"")),
    }
}

/// A plugin value as a script sees it: numbers in display units.
fn plugin_value(spec: Option<&grid4d_plugin::ParamSpec>, value: PluginValue) -> Dynamic {
    match value {
        PluginValue::Float(v) => {
            let shown = match spec.map(|s| s.kind) {
                Some(ParamKind::Float { unit: Unit::Angle, .. }) => v.to_degrees(),
                Some(ParamKind::Float {
                    unit: Unit::Percent, ..
                }) => v * 100.0,
                _ => v,
            };
            Dynamic::from(f64::from(shown))
        }
        PluginValue::Int(v) => Dynamic::from(i64::from(v)),
        PluginValue::Bool(v) => Dynamic::from(v),
        PluginValue::Choice(v) => Dynamic::from(i64::from(v)),
        PluginValue::Vector(v) => Dynamic::from(array(v)),
        PluginValue::Color(c) => Dynamic::from(array(Vec3::from_array(c))),
    }
}

/// A script's value for a plugin parameter, brought into its type and range.
fn to_plugin_value(spec: &grid4d_plugin::ParamSpec, value: &Dynamic) -> Result<PluginValue> {
    let wrong = || error(format!("\"{}\" takes a different kind of value", spec.name));
    let raw = match spec.kind {
        ParamKind::Float { unit, .. } => {
            let Some(v) = number(value) else { return wrong() };
            PluginValue::Float(match unit {
                Unit::Angle => v.to_radians(),
                Unit::Percent => v / 100.0,
                Unit::Length | Unit::None => v,
            })
        }
        ParamKind::Int { .. } => match value.as_int().ok().and_then(|v| i32::try_from(v).ok()) {
            Some(v) => PluginValue::Int(v),
            None => return wrong(),
        },
        ParamKind::Bool => match value.as_bool() {
            Ok(v) => PluginValue::Bool(v),
            Err(_) => return wrong(),
        },
        ParamKind::Choice(choices) => {
            let index = value.as_int().ok().and_then(|v| u32::try_from(v).ok()).or_else(|| {
                let name = value.clone().into_string().ok()?;
                choices
                    .iter()
                    .position(|c| c.eq_ignore_ascii_case(&name))
                    .and_then(|i| u32::try_from(i).ok())
            });
            match index {
                Some(v) => PluginValue::Choice(v),
                None => return wrong(),
            }
        }
        ParamKind::Vector { .. } | ParamKind::Color => {
            let Some(v) = value.read_lock::<Array>().map(|a| vector(&a)) else {
                return wrong();
            };
            let v = v?;
            if spec.kind == ParamKind::Color {
                PluginValue::Color(v.to_array())
            } else {
                PluginValue::Vector(v)
            }
        }
    };
    Ok(spec.checked(raw))
}

/// The engine: limits, no `eval`, the document API on `state`, `print` into
/// `output`.
fn engine(state: &Shared, output: &Rc<RefCell<Vec<String>>>) -> Engine {
    let mut engine = Engine::new();
    engine
        .set_max_operations(MAX_OPERATIONS)
        .set_max_call_levels(64)
        .set_max_expr_depths(64, 32)
        .set_max_string_size(1 << 20)
        .set_max_array_size(1 << 20)
        .set_max_map_size(1 << 16);
    engine.disable_symbol("eval");
    let printed = Rc::clone(output);
    engine.on_print(move |line| printed.borrow_mut().push(line.to_owned()));
    let debugged = Rc::clone(output);
    engine.on_debug(move |line, _, _| debugged.borrow_mut().push(line.to_owned()));

    engine.register_type_with_name::<Object>("Object");
    let s = Rc::clone(state);
    engine.register_fn("to_string", move |o: &mut Object| -> String {
        let state = s.borrow();
        state.document.scene.get(o.0).map_or_else(
            || "(deleted)".to_owned(),
            |n| format!("{} ({})", n.data.name, kind_name(&n.data.kind)),
        )
    });
    let s = Rc::clone(state);
    engine.register_fn("to_debug", move |o: &mut Object| -> String {
        let state = s.borrow();
        state.document.scene.get(o.0).map_or_else(
            || "Object(deleted)".to_owned(),
            |n| format!("Object(\"{}\")", n.data.name),
        )
    });
    engine.register_fn("==", |a: Object, b: Object| a == b);
    engine.register_fn("!=", |a: Object, b: Object| a != b);

    register_document(&mut engine, state);
    register_object(&mut engine, state);
    register_parameters(&mut engine, state);
    engine
}

/// `objects`, `roots`, `find`, `selection`, `select`, `frame`, `create`,
/// `delete`, `run_command`.
fn register_document(engine: &mut Engine, state: &Shared) {
    fn walk(scene: &Scene, ids: &[NodeId], out: &mut Array) {
        for &id in ids {
            out.push(Dynamic::from(Object(id)));
            walk(scene, scene.children(id), out);
        }
    }
    let s = Rc::clone(state);
    engine.register_fn("objects", move || -> Array {
        let state = s.borrow();
        let scene = &state.document.scene;
        let mut out = Array::new();
        walk(scene, scene.roots(), &mut out);
        out
    });
    let s = Rc::clone(state);
    engine.register_fn("roots", move || -> Array {
        let state = s.borrow();
        state
            .document
            .scene
            .roots()
            .iter()
            .map(|&id| Dynamic::from(Object(id)))
            .collect()
    });
    let s = Rc::clone(state);
    engine.register_fn("find", move |name: &str| -> Dynamic {
        let state = s.borrow();
        let scene = &state.document.scene;
        let mut all = Array::new();
        walk(scene, scene.roots(), &mut all);
        all.into_iter()
            .find(|o| {
                o.read_lock::<Object>()
                    .and_then(|o| scene.get(o.0))
                    .is_some_and(|n| n.data.name == name)
            })
            .unwrap_or(Dynamic::UNIT)
    });
    let s = Rc::clone(state);
    engine.register_fn("selection", move || -> Array {
        let state = s.borrow();
        state
            .selection
            .iter()
            .filter(|&&id| state.document.scene.contains(id))
            .map(|&id| Dynamic::from(Object(id)))
            .collect()
    });
    let s = Rc::clone(state);
    engine.register_fn("select", move |o: Object| {
        let mut state = s.borrow_mut();
        state.selection = vec![o.0];
        state.selection_changed = true;
    });
    let s = Rc::clone(state);
    engine.register_fn("select", move |list: Array| -> Result<()> {
        let mut ids = Vec::with_capacity(list.len());
        for item in list {
            match item.try_cast::<Object>() {
                Some(o) => ids.push(o.0),
                None => return error("select takes objects"),
            }
        }
        let mut state = s.borrow_mut();
        state.selection = ids;
        state.selection_changed = true;
        Ok(())
    });
    let s = Rc::clone(state);
    #[expect(clippy::cast_possible_truncation, reason = "frames are whole numbers in range")]
    engine.register_fn("frame", move || -> i64 { s.borrow().frame.round() as i64 });

    let create = |state: &Shared, kind: &str, parent: Option<NodeId>| -> Result<Object> {
        let mut state = state.borrow_mut();
        if let Some(parent) = parent {
            state.data(parent)?;
        }
        let data = new_object(&state.document.scene, kind)?;
        let inverse = state.apply(SceneEdit::Insert {
            parent,
            index: usize::MAX,
            data,
        })?;
        inverse
            .node()
            .map_or_else(|| error("the object could not be made"), |id| Ok(Object(id)))
    };
    let s = Rc::clone(state);
    engine.register_fn("create", move |kind: &str| create(&s, kind, None));
    let s = Rc::clone(state);
    engine.register_fn("create", move |kind: &str, parent: Object| {
        create(&s, kind, Some(parent.0))
    });
    let s = Rc::clone(state);
    engine.register_fn("delete", move |o: Object| -> Result<()> {
        let mut state = s.borrow_mut();
        state.data(o.0)?;
        state.apply(SceneEdit::Detach { root: o.0 })?;
        Ok(())
    });

    let s = Rc::clone(state);
    engine.register_fn("run_command", move |name: &str| run_command(&s, name, &Map::new()));
    let s = Rc::clone(state);
    engine.register_fn("run_command", move |name: &str, options: Map| {
        run_command(&s, name, &options)
    });
}

/// Runs a plugin command on the script's selection; gives its new top-level
/// objects.
fn run_command(state: &Shared, name: &str, options: &Map) -> Result<Array> {
    let registry = grid4d_plugin::registry();
    let Some(plugin) = registry
        .commands()
        .find(|c| c.name().eq_ignore_ascii_case(name) || c.type_id() == name)
    else {
        return error(format!("there is no plugin command called \"{name}\""));
    };
    let mut values = grid4d_plugin::Registry::new_object(plugin);
    for (key, value) in options {
        let Some(spec) = plugin.params().iter().find(|p| p.name.eq_ignore_ascii_case(key)) else {
            return error(format!("{} has no option \"{key}\"", plugin.name()));
        };
        values.set(spec.name, to_plugin_value(spec, value)?);
    }
    let changes = {
        let state = state.borrow();
        let (frame, shapes) = (state.frame, state.shapes);
        let scene = &state.document.scene;
        let shape = |id: NodeId| shapes(scene, id, frame);
        let context = Context::new(
            &state.document,
            &state.selection,
            state.selection.last().copied(),
            frame,
            &shape,
        );
        match plugin.run(&context, &Values::new(plugin.params(), &values)) {
            Ok(changes) => changes,
            Err(message) => return error(format!("{}: {message}", plugin.name())),
        }
    };
    let mut state = state.borrow_mut();
    match changes.apply(&mut state.document) {
        Ok(applied) => {
            // Kept in the order they were applied.
            state.inverses.extend(applied.inverses.into_iter().rev());
            Ok(applied.tops.into_iter().map(|id| Dynamic::from(Object(id))).collect())
        }
        Err(err) => error(err),
    }
}

/// The properties of an `Object`.
fn register_object(engine: &mut Engine, state: &Shared) {
    let s = Rc::clone(state);
    engine.register_get("name", move |o: &mut Object| -> Result<String> {
        Ok(s.borrow().data(o.0)?.name.clone())
    });
    let s = Rc::clone(state);
    engine.register_set("name", move |o: &mut Object, name: &str| -> Result<()> {
        s.borrow_mut().edit(o.0, |d| {
            d.name = name.to_owned();
            Ok(())
        })
    });
    let s = Rc::clone(state);
    engine.register_get("kind", move |o: &mut Object| -> Result<String> {
        Ok(kind_name(&s.borrow().data(o.0)?.kind).to_owned())
    });

    // Position, rotation (degrees) and scale.
    for (property, degrees) in [("position", false), ("rotation", true), ("scale", false)] {
        let pick = move |d: &NodeData| match property {
            "position" => d.transform.position,
            "rotation" => d.transform.rotation,
            _ => d.transform.scale,
        };
        let s = Rc::clone(state);
        engine.register_get(property, move |o: &mut Object| -> Result<Array> {
            let v = pick(s.borrow().data(o.0)?);
            Ok(array(if degrees { v * (180.0 / std::f32::consts::PI) } else { v }))
        });
        let s = Rc::clone(state);
        engine.register_set(property, move |o: &mut Object, value: Array| -> Result<()> {
            let v = vector(&value)?;
            let v = if degrees { v * (std::f32::consts::PI / 180.0) } else { v };
            s.borrow_mut().edit(o.0, |d| {
                match property {
                    "position" => d.transform.position = v,
                    "rotation" => d.transform.rotation = v,
                    _ => d.transform.scale = v,
                }
                Ok(())
            })
        });
    }

    let s = Rc::clone(state);
    engine.register_get("color", move |o: &mut Object| -> Result<Array> {
        Ok(array(Vec3::from_array(s.borrow().data(o.0)?.display_color)))
    });
    let s = Rc::clone(state);
    engine.register_set("color", move |o: &mut Object, value: Array| -> Result<()> {
        let c = vector(&value)?.clamp(Vec3::ZERO, Vec3::ONE).to_array();
        s.borrow_mut().edit(o.0, |d| {
            d.display_color = c;
            Ok(())
        })
    });

    for property in ["visible", "visible_in_render", "enabled"] {
        let s = Rc::clone(state);
        engine.register_get(property, move |o: &mut Object| -> Result<bool> {
            let state = s.borrow();
            let d = state.data(o.0)?;
            Ok(match property {
                "visible" => d.visible_in_editor,
                "visible_in_render" => d.visible_in_render,
                _ => d.enabled,
            })
        });
        let s = Rc::clone(state);
        engine.register_set(property, move |o: &mut Object, on: bool| -> Result<()> {
            s.borrow_mut().edit(o.0, |d| {
                match property {
                    "visible" => d.visible_in_editor = on,
                    "visible_in_render" => d.visible_in_render = on,
                    _ => d.enabled = on,
                }
                Ok(())
            })
        });
    }

    let s = Rc::clone(state);
    engine.register_get("parent", move |o: &mut Object| -> Result<Dynamic> {
        let state = s.borrow();
        state.data(o.0)?;
        Ok(state
            .document
            .scene
            .get(o.0)
            .and_then(grid4d_scene::Node::parent)
            .map_or(Dynamic::UNIT, |p| Dynamic::from(Object(p))))
    });
    let s = Rc::clone(state);
    engine.register_set("parent", move |o: &mut Object, parent: Dynamic| -> Result<()> {
        let parent = if parent.is_unit() {
            None
        } else {
            match parent.try_cast::<Object>() {
                Some(p) => Some(p.0),
                None => return error("parent takes an object or ()"),
            }
        };
        let mut state = s.borrow_mut();
        let scene = &state.document.scene;
        if let Some(p) = parent {
            state.data(p)?;
            if p == o.0 || scene.is_ancestor_of(o.0, p) {
                return error("an object cannot go under itself");
            }
        }
        let Some(transform) = scene.transform_keeping_world(o.0, parent) else {
            return error("the object no longer exists");
        };
        state.apply(SceneEdit::Move {
            id: o.0,
            parent,
            index: usize::MAX,
            transform,
        })?;
        Ok(())
    });
    let s = Rc::clone(state);
    engine.register_get("children", move |o: &mut Object| -> Result<Array> {
        let state = s.borrow();
        state.data(o.0)?;
        Ok(state
            .document
            .scene
            .children(o.0)
            .iter()
            .map(|&id| Dynamic::from(Object(id)))
            .collect())
    });

    // The material and layer, by name.
    let s = Rc::clone(state);
    engine.register_get("material", move |o: &mut Object| -> Result<String> {
        use grid4d_scene::Tags;
        let state = s.borrow();
        let material = state.data(o.0)?.tags.material();
        Ok(material
            .and_then(|m| state.document.scene.materials().get(m))
            .map_or_else(String::new, |m| m.name.clone()))
    });
    let s = Rc::clone(state);
    engine.register_set("material", move |o: &mut Object, name: &str| -> Result<()> {
        let mut state = s.borrow_mut();
        let material = if name.is_empty() {
            None
        } else {
            match state.document.scene.materials().iter().find(|(_, m)| m.name == name) {
                Some((id, _)) => Some(id),
                None => return error(format!("there is no material called \"{name}\"")),
            }
        };
        state.edit(o.0, |d| {
            d.tags.retain(|t| !matches!(t, Tag::Material { .. }));
            if material.is_some() {
                d.tags.push(Tag::Material { material });
            }
            Ok(())
        })
    });
    let s = Rc::clone(state);
    engine.register_get("layer", move |o: &mut Object| -> Result<String> {
        let state = s.borrow();
        let layer = state.data(o.0)?.layer;
        Ok(layer
            .and_then(|l| state.document.scene.layers().get(l))
            .map_or_else(String::new, |l| l.name.clone()))
    });
    let s = Rc::clone(state);
    engine.register_set("layer", move |o: &mut Object, name: &str| -> Result<()> {
        let mut state = s.borrow_mut();
        let layer = if name.is_empty() {
            None
        } else {
            match state.document.scene.layers().iter().find(|(_, l)| l.name == name) {
                Some((id, _)) => Some(id),
                None => return error(format!("there is no layer called \"{name}\"")),
            }
        };
        state.edit(o.0, |d| {
            d.layer = layer;
            Ok(())
        })
    });
}

/// `get(name)` and `set(name, value)`: an object's parameters by their names
/// on the Object tab.
fn register_parameters(engine: &mut Engine, state: &Shared) {
    let s = Rc::clone(state);
    engine.register_fn("get", move |o: &mut Object, name: &str| -> Result<Dynamic> {
        let state = s.borrow();
        let kind = &state.data(o.0)?.kind;
        if let ObjectKind::Plugin(object) = kind {
            let spec = grid4d_plugin::registry()
                .plugin(&object.type_id)
                .and_then(|p| p.params().iter().find(|spec| spec.name.eq_ignore_ascii_case(name)));
            let value = match spec {
                Some(spec) => grid4d_plugin::registry()
                    .plugin(&object.type_id)
                    .and_then(|p| Values::new(p.params(), object).get(spec.name)),
                None => object
                    .values
                    .iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case(name))
                    .map(|&(_, v)| v),
            };
            return match value {
                Some(value) => Ok(plugin_value(spec, value)),
                None => error(format!("{} has no parameter \"{name}\"", kind_name(kind))),
            };
        }
        let param = kind
            .params()
            .into_iter()
            .find(|&p| kind.param_label(p).eq_ignore_ascii_case(name));
        match param.and_then(|p| kind.param(p).map(|v| (p, v))) {
            Some((p, v)) => Ok(Dynamic::from(f64::from(v) * p.unit().display_scale())),
            None => error(format!("{} has no parameter \"{name}\"", kind_name(kind))),
        }
    });
    let s = Rc::clone(state);
    engine.register_fn("set", move |o: &mut Object, name: &str, value: Dynamic| -> Result<()> {
        let mut state = s.borrow_mut();
        state.edit(o.0, |d| {
            let label = kind_name(&d.kind);
            if let ObjectKind::Plugin(object) = &mut d.kind {
                let spec = grid4d_plugin::registry()
                    .plugin(&object.type_id)
                    .and_then(|p| p.params().iter().find(|spec| spec.name.eq_ignore_ascii_case(name)));
                let Some(spec) = spec else {
                    return error(format!("{label} has no parameter \"{name}\""));
                };
                object.set(spec.name, to_plugin_value(spec, &value)?);
                return Ok(());
            }
            let param = d
                .kind
                .params()
                .into_iter()
                .find(|&p| d.kind.param_label(p).eq_ignore_ascii_case(name));
            let (Some(param), Some(v)) = (param, number(&value)) else {
                return error(format!("{label} has no number parameter \"{name}\""));
            };
            #[expect(clippy::cast_possible_truncation, reason = "parameters are f32")]
            let stored = (f64::from(v) / param.unit().display_scale()) as f32;
            d.kind.set_param(param, stored);
            Ok(())
        })
    });
}

/// Where a Rhai error happened, and its message without the position.
fn script_error(mut err: EvalAltResult, output: Vec<String>) -> ScriptError {
    let position = err.take_position();
    let message = match &err {
        EvalAltResult::ErrorTooManyOperations(_) => {
            format!("the script was stopped after {MAX_OPERATIONS} operations (an endless loop?); nothing was changed")
        }
        EvalAltResult::ErrorRuntime(value, _) => value.to_string(),
        _ => err.to_string(),
    };
    ScriptError {
        message,
        line: position.line(),
        column: position.position(),
        output,
    }
}

/// Runs `source` on a copy of `document`, with `selection` (in order) as the
/// script's selection, at `frame`.
///
/// # Errors
///
/// A [`ScriptError`] when the script does not compile or stops with an
/// error; the document is then untouched.
pub fn run(
    document: &Document,
    selection: &[NodeId],
    frame: f64,
    shapes: Shapes,
    source: &str,
) -> std::result::Result<Outcome, ScriptError> {
    let state = Rc::new(RefCell::new(State {
        document: document.clone(),
        inverses: Vec::new(),
        selection: selection.to_vec(),
        selection_changed: false,
        frame,
        shapes,
    }));
    let output = Rc::new(RefCell::new(Vec::new()));
    let engine = engine(&state, &output);
    let result = engine
        .compile(source)
        .map_err(EvalAltResult::from)
        .and_then(|ast| engine.run_ast(&ast).map_err(|err| *err));
    drop(engine);
    let output = output.take();
    if let Err(err) = result {
        return Err(script_error(err, output));
    }
    let Ok(state) = Rc::try_unwrap(state).map(RefCell::into_inner) else {
        return Err(ScriptError {
            message: "the script's document is still in use".to_owned(),
            line: None,
            column: None,
            output,
        });
    };
    let mut inverses = state.inverses;
    inverses.reverse();
    Ok(Outcome {
        document: state.document,
        inverses,
        output,
        selection: state.selection_changed.then_some(state.selection),
    })
}

#[cfg(test)]
mod tests;
