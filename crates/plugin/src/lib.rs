//! # grid4d-plugin
//!
//! The API plugins are written against (see `docs/reference/extend-plugins.md`).
//! A plugin is a crate built into Grid4D: it describes itself and its
//! parameters ([`Plugin`], [`ParamSpec`]) and does one job —
//! [`GeneratorPlugin`] builds a mesh from them, [`DeformerPlugin`] reshapes the
//! objects below its parent, [`CommandPlugin`] changes the document once when
//! chosen (returning [`Changes`]). The binary adds every plugin to a [`Registry`]
//! and [`install`]s it once at start; evaluation and the interface find
//! plugins there by their type id.
//!
//! Documents keep only a plugin object's type id and values
//! ([`grid4d_scene::PluginObject`]); a plugin reads them through [`Values`],
//! which supplies defaults for missing values and keeps them in range.

use std::sync::OnceLock;

use glam::{Mat4, Vec3};
use grid4d_geom::{PolyMesh, spline::Polyline};
use grid4d_scene::{Document, NodeData, NodeId, PluginObject, PluginValue, SceneEdit};

/// What a number means, for how the interface shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// A plain number.
    None,
    /// A length in centimetres (shown in the user's length unit).
    Length,
    /// An angle in radians (shown in degrees).
    Angle,
    /// A fraction 0 … 1 (shown as 0 … 100 %).
    Percent,
}

/// A parameter's type and range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParamKind {
    Float {
        min: f32,
        max: f32,
        unit: Unit,
    },
    Int {
        min: i32,
        max: i32,
    },
    Bool,
    /// One of these (English names, translated where shown).
    Choice(&'static [&'static str]),
    Vector {
        unit: Unit,
    },
    /// Linear RGB.
    Color,
}

/// One parameter of a plugin: its name (shown in the Attribute Manager and
/// the key of its value in documents), type, default and a hint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamSpec {
    pub name: &'static str,
    pub kind: ParamKind,
    pub default: PluginValue,
    /// A short explanation (the field's tooltip); may be empty.
    pub hint: &'static str,
}

impl ParamSpec {
    /// `value` brought to this parameter's type and range; a value of
    /// another type gives the default.
    #[must_use]
    pub fn checked(&self, value: PluginValue) -> PluginValue {
        match (self.kind, value) {
            (ParamKind::Float { min, max, .. }, PluginValue::Float(v)) if v.is_finite() => {
                PluginValue::Float(v.clamp(min, max))
            }
            (ParamKind::Int { min, max }, PluginValue::Int(v)) => PluginValue::Int(v.clamp(min, max)),
            (ParamKind::Bool, PluginValue::Bool(v)) => PluginValue::Bool(v),
            (ParamKind::Choice(choices), PluginValue::Choice(v)) => {
                let last = u32::try_from(choices.len().saturating_sub(1)).unwrap_or(0);
                PluginValue::Choice(v.min(last))
            }
            (ParamKind::Vector { .. }, PluginValue::Vector(v)) if v.is_finite() => PluginValue::Vector(v),
            (ParamKind::Color, PluginValue::Color(c)) if c.iter().all(|x| x.is_finite()) => {
                PluginValue::Color(c.map(|x| x.clamp(0.0, 1.0)))
            }
            _ => self.default,
        }
    }
}

/// A plugin object's values as its plugin reads them: by name, with the
/// defaults of its description for missing ones and ranges applied.
#[derive(Clone, Copy, Debug)]
pub struct Values<'a> {
    params: &'a [ParamSpec],
    object: &'a PluginObject,
}

impl<'a> Values<'a> {
    #[must_use]
    pub const fn new(params: &'a [ParamSpec], object: &'a PluginObject) -> Self {
        Self { params, object }
    }

    /// The value of the parameter named `name`, checked; `None` if the
    /// description has no such parameter.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<PluginValue> {
        let spec = self.params.iter().find(|p| p.name == name)?;
        Some(self.object.get(name).map_or(spec.default, |v| spec.checked(v)))
    }

    /// A float parameter (0 if there is none of that name and type).
    #[must_use]
    pub fn float(&self, name: &str) -> f32 {
        match self.get(name) {
            Some(PluginValue::Float(v)) => v,
            _ => 0.0,
        }
    }

    /// An integer parameter (0 if there is none of that name and type).
    #[must_use]
    pub fn int(&self, name: &str) -> i32 {
        match self.get(name) {
            Some(PluginValue::Int(v)) => v,
            _ => 0,
        }
    }

    #[must_use]
    pub fn bool(&self, name: &str) -> bool {
        matches!(self.get(name), Some(PluginValue::Bool(true)))
    }

    /// A choice's index.
    #[must_use]
    pub fn choice(&self, name: &str) -> u32 {
        match self.get(name) {
            Some(PluginValue::Choice(v)) => v,
            _ => 0,
        }
    }

    #[must_use]
    pub fn vector(&self, name: &str) -> Vec3 {
        match self.get(name) {
            Some(PluginValue::Vector(v)) => v,
            _ => Vec3::ZERO,
        }
    }

    #[must_use]
    pub fn color(&self, name: &str) -> [f32; 3] {
        match self.get(name) {
            Some(PluginValue::Color(c)) => c,
            _ => [0.0; 3],
        }
    }
}

/// What every plugin says about itself.
pub trait Plugin: Send + Sync {
    /// Its id, unique and stable (it is saved in documents):
    /// `"<author or project>.<name>"`, e.g. `"grid4d.gear"`.
    fn type_id(&self) -> &'static str;
    /// Its name in English (menus, new objects' names).
    fn name(&self) -> &'static str;
    /// One sentence on what it does (the menu's tooltip).
    fn description(&self) -> &'static str;
    /// Its parameters, in Attribute Manager order. Float parameters can be
    /// animated (the first [`grid4d_scene::ObjectParam::PLUGIN_PARAMS`]).
    fn params(&self) -> &'static [ParamSpec];
}

/// A plugin that makes a kind of object from its parameters (like a
/// primitive): Create ▸ Plugins lists it, the Attribute Manager shows its
/// parameters, evaluation calls [`GeneratorPlugin::build`].
pub trait GeneratorPlugin: Plugin {
    /// The mesh for `values`, in the object's own space (centimetres, Y up,
    /// faces counter-clockwise seen from outside). An empty mesh draws
    /// nothing. Must not panic: return what you can.
    fn build(&self, values: &Values<'_>) -> PolyMesh;
}

/// A plugin that reshapes, like the built-in deformers, its parent and
/// everything below it (Create ▸ Plugins; Shift: under the selected object,
/// fitted to it). Its box is drawn from [`DeformerPlugin::size_param`].
pub trait DeformerPlugin: Plugin {
    /// The name of its Vector parameter (a length) that is its box's size,
    /// centred on its origin: drawn in the views and set by Fit to Parent.
    fn size_param(&self) -> &'static str;
    /// The name of the Float parameter its handle sets, if it has one: the
    /// handle sits above the top of the box, as high as the value (a length
    /// as it is; a fraction or plain number × half the box's height; an angle
    /// so that 90° is half the box's height) and is dragged up and down.
    fn handle_param(&self) -> Option<&'static str> {
        None
    }
    /// Moves `points`, given in the deformer's own space (centimetres, its
    /// origin at the centre of its box). Points that are not finite numbers
    /// afterwards leave the object undeformed. Must not panic.
    fn deform(&self, values: &Values<'_>, points: &mut [Vec3]);
}

/// What a command sees when it runs: the document as it is, the selection
/// and the current frame, and the shapes of objects as Grid4D evaluates them.
pub struct Context<'a> {
    pub document: &'a Document,
    /// The selected objects, in the order they were selected.
    pub selection: &'a [NodeId],
    /// The active object (usually the one selected last).
    pub active: Option<NodeId>,
    pub frame: f64,
    shapes: &'a dyn Fn(NodeId) -> Option<PolyMesh>,
}

impl std::fmt::Debug for Context<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Context")
            .field("selection", &self.selection)
            .field("active", &self.active)
            .field("frame", &self.frame)
            .finish_non_exhaustive()
    }
}

impl<'a> Context<'a> {
    /// `shapes` gives an object's shape in its own space at `frame` (Grid4D
    /// passes its evaluation; tests their own).
    #[must_use]
    pub fn new(
        document: &'a Document,
        selection: &'a [NodeId],
        active: Option<NodeId>,
        frame: f64,
        shapes: &'a dyn Fn(NodeId) -> Option<PolyMesh>,
    ) -> Self {
        Self {
            document,
            selection,
            active,
            frame,
            shapes,
        }
    }

    /// `id`'s shape in its own space at the frame — a primitive's, a polygon
    /// object's, a generator's result or a plugin's mesh (not deformed);
    /// `None` for objects without one.
    #[must_use]
    pub fn mesh(&self, id: NodeId) -> Option<PolyMesh> {
        (self.shapes)(id)
    }

    /// `id`'s placement in the world at the frame.
    #[must_use]
    pub fn world_matrix(&self, id: NodeId) -> Mat4 {
        self.document.scene.world_matrix_at(id, self.frame)
    }
}

/// An object a command adds, before it exists: see [`Changes::insert`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NewObject(usize);

impl NewObject {
    /// The order it was added in among the command's new objects.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

/// Where a new object goes: last among the children of its parent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parent {
    /// The top level.
    Root,
    /// An object of the document.
    Object(NodeId),
    /// An object the same command added before.
    New(NewObject),
}

/// One change a command asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// A new object.
    Insert { parent: Parent, data: NodeData },
    /// Any edit of the document (an object's data, a move, a deletion, the
    /// materials …).
    Edit(SceneEdit),
}

/// What a command does to the document, in order. Grid4D applies it as one
/// undo step, all or nothing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Changes {
    changes: Vec<Change>,
    inserted: usize,
}

impl Changes {
    /// Adds an object under `parent`; returns how to refer to it (as a
    /// parent of later objects).
    pub fn insert(&mut self, parent: Parent, data: NodeData) -> NewObject {
        self.changes.push(Change::Insert { parent, data });
        self.inserted += 1;
        NewObject(self.inserted - 1)
    }

    /// Adds an edit.
    pub fn edit(&mut self, edit: SceneEdit) {
        self.changes.push(Change::Edit(edit));
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// The changes, in order.
    #[must_use]
    pub fn list(&self) -> &[Change] {
        &self.changes
    }

    #[must_use]
    pub fn into_list(self) -> Vec<Change> {
        self.changes
    }
}

/// A plugin that changes the document once, when chosen in the Plugins
/// menu; its parameters are options asked for in a dialog first.
pub trait CommandPlugin: Plugin {
    /// What to change, from the document, the selection and the options; or,
    /// when it cannot run, why — an English sentence (`Err("…".into())`),
    /// translated where it is shown. Must not panic.
    ///
    /// # Errors
    ///
    /// The message to show when the command cannot do its work.
    fn run(&self, context: &Context<'_>, options: &Values<'_>) -> Result<Changes, String>;
}

/// The installed plugins.
#[derive(Default)]
pub struct Registry {
    generators: Vec<Box<dyn GeneratorPlugin>>,
    deformers: Vec<Box<dyn DeformerPlugin>>,
    commands: Vec<Box<dyn CommandPlugin>>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("plugins", &self.plugins().map(Plugin::type_id).collect::<Vec<_>>())
            .finish()
    }
}

impl Registry {
    /// `false` (and a warning) when `type_id` is taken: the first one stays.
    fn is_free(&self, type_id: &str) -> bool {
        let free = self.plugin(type_id).is_none();
        if !free {
            log::warn!("plugin {type_id} registered twice; the first one stays");
        }
        free
    }

    /// Adds a generator plugin; one whose type id is already taken is left out
    /// (the first one stays) and reported in the log.
    pub fn add_generator(&mut self, plugin: impl GeneratorPlugin + 'static) -> &mut Self {
        if self.is_free(plugin.type_id()) {
            self.generators.push(Box::new(plugin));
        }
        self
    }

    /// Adds a deformer plugin (type ids as [`Registry::add_generator`]).
    pub fn add_deformer(&mut self, plugin: impl DeformerPlugin + 'static) -> &mut Self {
        if self.is_free(plugin.type_id()) {
            self.deformers.push(Box::new(plugin));
        }
        self
    }

    /// Adds a command plugin (type ids as [`Registry::add_generator`]).
    pub fn add_command(&mut self, plugin: impl CommandPlugin + 'static) -> &mut Self {
        if self.is_free(plugin.type_id()) {
            self.commands.push(Box::new(plugin));
        }
        self
    }

    /// The command plugin with this type id.
    #[must_use]
    pub fn command(&self, type_id: &str) -> Option<&dyn CommandPlugin> {
        self.commands.iter().find(|c| c.type_id() == type_id).map(AsRef::as_ref)
    }

    /// Every command plugin, in the order they were added.
    pub fn commands(&self) -> impl Iterator<Item = &dyn CommandPlugin> {
        self.commands.iter().map(AsRef::as_ref)
    }

    /// The plugin with this type id, of any kind.
    #[must_use]
    pub fn plugin(&self, type_id: &str) -> Option<&dyn Plugin> {
        self.plugins().find(|p| p.type_id() == type_id)
    }

    /// Every plugin: the generators, the deformers, then the commands, in
    /// the order they were added.
    pub fn plugins(&self) -> impl Iterator<Item = &dyn Plugin> {
        self.generators
            .iter()
            .map(|g| g.as_ref() as &dyn Plugin)
            .chain(self.deformers.iter().map(|d| d.as_ref() as &dyn Plugin))
            .chain(self.commands.iter().map(|c| c.as_ref() as &dyn Plugin))
    }

    /// The deformer plugin with this type id.
    #[must_use]
    pub fn deformer(&self, type_id: &str) -> Option<&dyn DeformerPlugin> {
        self.deformers
            .iter()
            .find(|d| d.type_id() == type_id)
            .map(AsRef::as_ref)
    }

    /// Every deformer plugin, in the order they were added.
    pub fn deformers(&self) -> impl Iterator<Item = &dyn DeformerPlugin> {
        self.deformers.iter().map(AsRef::as_ref)
    }

    /// The generator plugin with this type id.
    #[must_use]
    pub fn generator(&self, type_id: &str) -> Option<&dyn GeneratorPlugin> {
        self.generators
            .iter()
            .find(|g| g.type_id() == type_id)
            .map(AsRef::as_ref)
    }

    /// Every generator plugin, in the order they were added.
    pub fn generators(&self) -> impl Iterator<Item = &dyn GeneratorPlugin> {
        self.generators.iter().map(AsRef::as_ref)
    }

    /// A new object of `plugin`'s with every value at its default.
    #[must_use]
    pub fn new_object(plugin: &dyn Plugin) -> PluginObject {
        PluginObject {
            type_id: plugin.type_id().to_owned(),
            values: plugin.params().iter().map(|p| (p.name.to_owned(), p.default)).collect(),
        }
    }

    /// The mesh `object`'s plugin builds, checked: `None` when the plugin is
    /// missing or its mesh has points that are not finite.
    #[must_use]
    pub fn build(&self, object: &PluginObject) -> Option<PolyMesh> {
        let plugin = self.generator(&object.type_id)?;
        let mesh = plugin.build(&Values::new(plugin.params(), object));
        if mesh.points().iter().all(|p| p.is_finite()) {
            Some(mesh)
        } else {
            log::warn!("plugin {} built points that are not finite numbers", object.type_id);
            None
        }
    }
}

/// How far above the top of its box a deformer's handle sits for `value`
/// (see [`DeformerPlugin::handle_param`]).
fn handle_height(spec: &ParamSpec, value: f32, half_height: f32) -> f32 {
    match spec.kind {
        ParamKind::Float { unit: Unit::Length, .. } => value,
        ParamKind::Float { unit: Unit::Angle, .. } => value / std::f32::consts::FRAC_PI_2 * half_height,
        _ => value * half_height,
    }
}

impl dyn DeformerPlugin + '_ {
    /// Its box's size for `object` (never negative).
    #[must_use]
    pub fn size(&self, object: &PluginObject) -> Vec3 {
        Values::new(self.params(), object).vector(self.size_param()).abs()
    }

    /// `object` with its box's size set to `size` (Fit to Parent).
    pub fn set_size(&self, object: &mut PluginObject, size: Vec3) {
        object.set(self.size_param(), PluginValue::Vector(size));
    }

    /// `points` (deformer space) deformed for `object`; `false`, with the
    /// points as they were, if the plugin made numbers that are not finite.
    pub fn deform_checked(&self, object: &PluginObject, points: &mut [Vec3]) -> bool {
        let before = points.to_vec();
        self.deform(&Values::new(self.params(), object), points);
        let finite = points.iter().all(|p| p.is_finite());
        if !finite {
            log::warn!("plugin {} moved points to numbers that are not finite", object.type_id);
            points.copy_from_slice(&before);
        }
        finite
    }

    /// Its box as drawn: twelve edges, each in pieces, deformed by itself.
    #[must_use]
    pub fn box_lines(&self, object: &PluginObject) -> Polyline {
        let mut lines = grid4d_geom::deform::box_lines(self.size(object), |p| p);
        self.deform_checked(object, &mut lines.points);
        lines
    }

    /// Where its handle sits (deformer space), if it has one: above the
    /// centre of the box's top.
    #[must_use]
    pub fn handle(&self, object: &PluginObject) -> Option<Vec3> {
        let spec = self.params().iter().find(|p| Some(p.name) == self.handle_param())?;
        let half = self.size(object).y * 0.5;
        let value = Values::new(self.params(), object).float(spec.name);
        Some(Vec3::new(0.0, half + handle_height(spec, value, half), 0.0))
    }

    /// `object` with the handle's value that puts the handle nearest `target`
    /// (deformer space); `step` rounds it (5°, 5 %, or a whole number).
    #[must_use]
    pub fn with_handle(&self, object: &PluginObject, target: Vec3, step: bool) -> PluginObject {
        let mut moved = object.clone();
        let Some(spec) = self.params().iter().find(|p| Some(p.name) == self.handle_param()) else {
            return moved;
        };
        let half = self.size(object).y * 0.5;
        let per_unit = handle_height(spec, 1.0, half);
        if per_unit.abs() <= f32::EPSILON {
            return moved;
        }
        let mut value = (target.y - half) / per_unit;
        if step {
            value = match spec.kind {
                ParamKind::Float { unit: Unit::Angle, .. } => (value.to_degrees() / 5.0).round() * 5.0_f32.to_radians(),
                ParamKind::Float {
                    unit: Unit::Percent, ..
                } => (value * 20.0).round() / 20.0,
                _ => value.round(),
            };
        }
        moved.set(spec.name, spec.checked(PluginValue::Float(value)));
        moved
    }
}

static REGISTRY: OnceLock<Registry> = OnceLock::new();

/// Installs the plugins (once, at start). Returns `false` if a registry was
/// installed already (the first one stays).
pub fn install(registry: Registry) -> bool {
    REGISTRY.set(registry).is_ok()
}

/// The installed plugins (none before [`install`]).
#[must_use]
pub fn registry() -> &'static Registry {
    static EMPTY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get().unwrap_or_else(|| EMPTY.get_or_init(Registry::default))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Slab;

    const SLAB: &[ParamSpec] = &[
        ParamSpec {
            name: "Width",
            kind: ParamKind::Float {
                min: 1.0,
                max: 1000.0,
                unit: Unit::Length,
            },
            default: PluginValue::Float(100.0),
            hint: "",
        },
        ParamSpec {
            name: "Steps",
            kind: ParamKind::Int { min: 1, max: 8 },
            default: PluginValue::Int(2),
            hint: "",
        },
        ParamSpec {
            name: "Shape",
            kind: ParamKind::Choice(&["Flat", "Round"]),
            default: PluginValue::Choice(0),
            hint: "",
        },
    ];

    impl Plugin for Slab {
        fn type_id(&self) -> &'static str {
            "test.slab"
        }
        fn name(&self) -> &'static str {
            "Slab"
        }
        fn description(&self) -> &'static str {
            "A square"
        }
        fn params(&self) -> &'static [ParamSpec] {
            SLAB
        }
    }

    impl GeneratorPlugin for Slab {
        fn build(&self, values: &Values<'_>) -> PolyMesh {
            let w = values.float("Width") * 0.5;
            let points = vec![
                Vec3::new(-w, 0.0, w),
                Vec3::new(w, 0.0, w),
                Vec3::new(w, 0.0, -w),
                Vec3::new(-w, if values.int("Steps") > 7 { f32::NAN } else { 0.0 }, -w),
            ];
            PolyMesh::from_faces(points, [[0, 1, 2, 3]]).unwrap_or_default()
        }
    }

    #[test]
    fn values_take_defaults_and_ranges() {
        let mut object = PluginObject::new("test.slab");
        let values = |o: &PluginObject| {
            let v = Values::new(SLAB, o);
            (v.float("Width"), v.int("Steps"), v.choice("Shape"))
        };
        assert_eq!(values(&object), (100.0, 2, 0), "missing: the defaults");
        object.set("Width", PluginValue::Float(5000.0));
        object.set("Steps", PluginValue::Bool(true));
        object.set("Shape", PluginValue::Choice(9));
        object.set("Unknown", PluginValue::Int(1));
        assert_eq!(values(&object), (1000.0, 2, 1), "clamped; a wrong type is the default");
        assert_eq!(Values::new(SLAB, &object).get("Unknown"), None);
        object.set("Width", PluginValue::Float(f32::NAN));
        assert_eq!(values(&object).0, 100.0);
    }

    /// Lifts points by its Lift (a length); Lift 1000 makes NaNs.
    struct Lift;

    const LIFT: &[ParamSpec] = &[
        ParamSpec {
            name: "Size",
            kind: ParamKind::Vector { unit: Unit::Length },
            default: PluginValue::Vector(Vec3::new(100.0, 100.0, 100.0)),
            hint: "",
        },
        ParamSpec {
            name: "Lift",
            kind: ParamKind::Float {
                min: -1000.0,
                max: 1000.0,
                unit: Unit::Length,
            },
            default: PluginValue::Float(10.0),
            hint: "",
        },
    ];

    impl Plugin for Lift {
        fn type_id(&self) -> &'static str {
            "test.lift"
        }
        fn name(&self) -> &'static str {
            "Lift"
        }
        fn description(&self) -> &'static str {
            "Lifts"
        }
        fn params(&self) -> &'static [ParamSpec] {
            LIFT
        }
    }

    impl DeformerPlugin for Lift {
        fn size_param(&self) -> &'static str {
            "Size"
        }
        fn handle_param(&self) -> Option<&'static str> {
            Some("Lift")
        }
        fn deform(&self, values: &Values<'_>, points: &mut [Vec3]) {
            let lift = values.float("Lift");
            for p in points {
                p.y += if lift >= 1000.0 { f32::NAN } else { lift };
            }
        }
    }

    #[test]
    fn deformer_plugins_deform_draw_their_box_and_move_their_handle() {
        let mut registry = Registry::default();
        registry.add_deformer(Lift).add_generator(Slab).add_deformer(Lift);
        assert_eq!(registry.plugins().count(), 2, "one per type id, of any kind");
        assert!(registry.generator("test.lift").is_none() && registry.deformer("test.slab").is_none());
        let lift = registry.deformer("test.lift").unwrap();
        let mut object = Registry::new_object(lift);
        let mut points = [Vec3::ZERO, Vec3::X];
        assert!(lift.deform_checked(&object, &mut points));
        assert_eq!(points, [Vec3::new(0.0, 10.0, 0.0), Vec3::new(1.0, 10.0, 0.0)]);
        let lines = lift.box_lines(&object);
        assert_eq!(lines.segments.len(), 12);
        assert!(lines.points.iter().all(|p| (p.y - 10.0).abs() <= 50.0 + 1e-3));

        // The handle: 10 above the top (50), dragged to 80 → a lift of 30.
        assert_eq!(lift.handle(&object), Some(Vec3::new(0.0, 60.0, 0.0)));
        let moved = lift.with_handle(&object, Vec3::new(5.0, 80.4, 0.0), true);
        assert_eq!(moved.get("Lift"), Some(PluginValue::Float(30.0)));
        lift.set_size(&mut object, Vec3::new(10.0, 20.0, 30.0));
        assert_eq!(lift.size(&object), Vec3::new(10.0, 20.0, 30.0));

        object.set("Lift", PluginValue::Float(1000.0));
        let mut points = [Vec3::ONE];
        assert!(!lift.deform_checked(&object, &mut points));
        assert_eq!(points, [Vec3::ONE], "kept as they were");
    }

    /// Adds a Null with a child, or refuses without a selection.
    struct Pair;

    impl Plugin for Pair {
        fn type_id(&self) -> &'static str {
            "test.pair"
        }
        fn name(&self) -> &'static str {
            "Pair"
        }
        fn description(&self) -> &'static str {
            "Two objects"
        }
        fn params(&self) -> &'static [ParamSpec] {
            &[]
        }
    }

    impl CommandPlugin for Pair {
        fn run(&self, context: &Context<'_>, _: &Values<'_>) -> Result<Changes, String> {
            if context.selection.is_empty() {
                return Err("Select something".into());
            }
            let mut changes = Changes::default();
            let parent = changes.insert(Parent::Root, NodeData::new("A", grid4d_scene::ObjectKind::Null));
            changes.insert(Parent::New(parent), NodeData::new("B", grid4d_scene::ObjectKind::Null));
            Ok(changes)
        }
    }

    #[test]
    fn command_plugins_return_changes_that_refer_to_new_objects() {
        let mut registry = Registry::default();
        registry.add_command(Pair).add_generator(Slab).add_command(Pair);
        assert_eq!(registry.commands().count(), 1);
        assert_eq!(registry.plugins().count(), 2);
        let pair = registry.command("test.pair").unwrap();
        let document = Document::new("d");
        let shapes = |_: NodeId| None;
        let options = PluginObject::new("test.pair");
        let values = Values::new(pair.params(), &options);
        let nothing = Context::new(&document, &[], None, 0.0, &shapes);
        assert_eq!(pair.run(&nothing, &values), Err("Select something".to_owned()));
        let some = [NodeId::default()];
        let context = Context::new(&document, &some, None, 0.0, &shapes);
        let changes = pair.run(&context, &values).unwrap();
        assert_eq!(changes.list().len(), 2);
        assert!(matches!(
            changes.list()[1],
            Change::Insert { parent: Parent::New(first), .. } if first.index() == 0
        ));
        assert!(context.mesh(some[0]).is_none());
    }

    #[test]
    fn the_registry_finds_builds_and_rejects() {
        let mut registry = Registry::default();
        registry.add_generator(Slab).add_generator(Slab);
        assert_eq!(registry.generators().count(), 1, "one per type id");
        let plugin = registry.generator("test.slab").unwrap();
        let mut object = Registry::new_object(plugin);
        assert_eq!(object.values.len(), 3);
        assert_eq!(registry.build(&object).unwrap().face_count(), 1);
        object.set("Steps", PluginValue::Int(8));
        assert!(registry.build(&object).is_none(), "points that are not finite");
        assert!(registry.build(&PluginObject::new("test.missing")).is_none());
        assert!(registry.generator("test.missing").is_none());
    }
}
