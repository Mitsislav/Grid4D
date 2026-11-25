//! Generators: objects that build their geometry from their children. The
//! Subdivision Surface smooths its first child — the *cage*, with the cage's own
//! descendants merged into one mesh — in the generator's space; the Extrude
//! pushes its first child spline (or, hierarchical, every spline below it) into
//! a solid; the Lathe turns its first child spline around its Y axis; the
//! Sweep moves its first child spline (the profile) along its second (the path);
//! the Loft stretches a skin over all its child splines; the Boolean combines
//! its first two children (each with its own children, other generators'
//! results included) as solids; the Symmetry shows its children with their
//! mirror image; the Join merges its children into one mesh; the Reduce
//! makes them lighter. The Spline Boolean combines its child splines into a
//! curve, which the spline generators read like a spline (D45).

use std::sync::Arc;

use crate::deform::Pictures;
use glam::Mat4;
use grid4d_geom::{
    PolyMesh,
    boolean::{self, BooleanError},
    ops::{OptimizeOptions, optimize},
    reduce::{CollapseOrder, Constraints},
    spline::{Interpolation, Polyline, SplinePrimitive, Text},
    spline_boolean::{self, SplineBoolean},
    subdivision::{self, SubdivisionOptions},
    surface::{self, Extrusion, Lathe, Loft, Sweep},
    symmetry::{self, Symmetry},
    uv,
};
use grid4d_scene::{Boolean, Join, MeshHandle, NodeId, ObjectKind, Reduce, Scene, SplineHandle};

use crate::{
    Purpose,
    deform::{self, Link},
    editable_mesh,
};

/// A polygon object's edge creases (percent) and hard edges, by point pair
/// (smaller first, sorted): what a generator reads from beside its mesh.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct EdgeMarks {
    creases: Vec<([u32; 2], u8)>,
    hard: Vec<[u32; 2]>,
}

impl EdgeMarks {
    /// The marks of `components` on `mesh` (an edge's crease is its corners'
    /// largest).
    fn of(mesh: &MeshHandle, components: &grid4d_scene::Components) -> Self {
        let state = components.state();
        if state.creases.is_empty() && state.hard.is_empty() {
            return Self::default();
        }
        let topology = mesh.topology();
        let pair = |corner: usize| topology.corner_edge(corner).map(|e| topology.edges()[e as usize]);
        let mut creases: std::collections::BTreeMap<[u32; 2], u8> = std::collections::BTreeMap::new();
        for (&corner, &percent) in &state.creases {
            if let Some(edge) = pair(corner as usize) {
                let entry = creases.entry(edge).or_insert(0);
                *entry = (*entry).max(percent);
            }
        }
        let mut hard: Vec<[u32; 2]> = state.hard.iter().filter_map(pair).collect();
        hard.sort_unstable();
        hard.dedup();
        Self {
            creases: creases.into_iter().collect(),
            hard,
        }
    }
}

/// What a generator makes from its inputs, for one evaluation's purpose.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Options {
    Subdivision(SubdivisionOptions),
    Extrude { extrusion: Extrusion, hierarchical: bool },
    Lathe(Lathe),
    Sweep(Sweep),
    Loft(Loft),
    Boolean(Boolean),
    Symmetry(Symmetry),
    Join(Join),
    SplineBoolean(SplineBoolean),
    Reduce(Reduce),
}

impl Options {
    /// The options that decide a Reduce's collapse order: everything but
    /// its Keep (a change of Keep only replays the order).
    pub(crate) fn order_key(&self) -> Option<Reduce> {
        match *self {
            Self::Reduce(reduce) => Some(Reduce { keep: 0.0, ..reduce }),
            _ => None,
        }
    }
}

/// Something a generator wants its user to know about its result (shown in
/// the Attribute Manager).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeneratorNote {
    /// A Boolean lacks one of its two inputs (it shows the first).
    MissingInput,
    /// An input has a point that is not a number (the first input is shown).
    NotANumber,
    /// The result would be too large for a mesh (the first input is shown).
    TooLarge,
}

/// A generator's result, built.
#[derive(Clone, Debug, Default)]
pub struct Built {
    pub mesh: PolyMesh,
    /// A generator of curves' result (the Spline Boolean's), in its space.
    pub curve: Option<Polyline>,
    /// A Boolean's edges along the cut (point pairs of `mesh`).
    pub cut_edges: Vec<[u32; 2]>,
    /// Edges shading splits along (point pairs of `mesh`): a Boolean's cut
    /// with Hard Cut Edges.
    pub hard: Vec<[u32; 2]>,
    pub note: Option<GeneratorNote>,
}

/// How deep generators inside a Boolean's inputs are followed.
const MAX_NESTING: u32 = 16;

/// One input of a generator, in the generator's space: what decides the result,
/// so an unchanged list means an unchanged result (a selection change in the
/// cage, or a move of the generator itself, changes nothing).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Input {
    source: Source,
    /// The input node's transform relative to the generator.
    matrix: Mat4,
    /// The deformers inside the input reaching this node (empty for most).
    links: Vec<Link>,
    /// Which of a Boolean's two inputs it belongs to, or which child of a
    /// Reduce that reduces its children apart (0 for others).
    operand: u32,
}

#[derive(Clone, Debug, PartialEq)]
enum Source {
    /// A polygon object's mesh (compared by id) and its edges' creases and
    /// hard flags.
    Mesh(MeshHandle, EdgeMarks),
    /// A primitive's parameters (its mesh is generated when needed).
    Primitive(ObjectKind),
    /// A spline primitive's parameters (its curve is sampled when needed).
    SplinePrimitive(SplinePrimitive),
    /// A text's parameters (its outline is made when needed).
    Text(Text),
    /// An editable spline (compared by id) and how it is sampled.
    Spline(SplineHandle, Interpolation),
    /// Another generator inside a Boolean's input, or a generator of curves
    /// inside a spline generator: its options and inputs (in its own space;
    /// its result is built when needed).
    Generated(Options, Vec<Input>),
}

impl Source {
    /// The sampled curve of a spline input; `None` for meshes.
    fn polyline(&self) -> Option<Polyline> {
        match self {
            Self::SplinePrimitive(primitive) => Some(primitive.polyline()),
            Self::Text(text) => Some(grid4d_text::text_spline(text).sample(text.interpolation)),
            Self::Spline(spline, interpolation) => Some(spline.spline().sample(*interpolation)),
            Self::Generated(options @ Options::SplineBoolean(_), inputs) => build_all(inputs, options).curve,
            Self::Mesh(..) | Self::Primitive(_) | Self::Generated(..) => None,
        }
    }
}

/// The generator's options for `purpose` (its viewport or render levels),
/// `None` if `kind` is not a generator.
pub(crate) fn options(kind: &ObjectKind, purpose: Purpose) -> Option<Options> {
    match *kind {
        ObjectKind::SubdivisionSurface {
            viewport_levels,
            render_levels,
            linear_pre,
            boundary,
        } => Some(Options::Subdivision(SubdivisionOptions {
            levels: match purpose {
                Purpose::Editor => viewport_levels,
                Purpose::Render => render_levels,
            },
            linear_pre,
            boundary,
        })),
        ObjectKind::Extrude {
            extrusion,
            hierarchical,
        } => Some(Options::Extrude {
            extrusion,
            hierarchical,
        }),
        ObjectKind::Lathe(lathe) => Some(Options::Lathe(lathe)),
        ObjectKind::Sweep(sweep) => Some(Options::Sweep(sweep)),
        ObjectKind::Loft(loft) => Some(Options::Loft(loft)),
        ObjectKind::Boolean(boolean) => Some(Options::Boolean(boolean)),
        ObjectKind::Symmetry(symmetry) => Some(Options::Symmetry(symmetry)),
        ObjectKind::Join(join) => Some(Options::Join(join)),
        ObjectKind::SplineBoolean(settings) => Some(Options::SplineBoolean(settings)),
        ObjectKind::Reduce(reduce) => Some(Options::Reduce(reduce)),
        _ => None,
    }
}

/// Fills `inputs` with the inputs of generator `id` — for a Subdivision
/// Surface its cage (the first child and that child's visible, enabled
/// descendants that have geometry), for an Extrude its spline(s) — placed as
/// they are at `frame` (animation included), and returns the colour the result
/// shows without a material. `stack` is scratch space.
#[expect(clippy::too_many_arguments, reason = "where and when it is evaluated, passed apart")]
pub(crate) fn gather(
    scene: &Scene,
    id: NodeId,
    options: &Options,
    purpose: Purpose,
    frame: f64,
    pictures: &Pictures,
    stack: &mut Vec<NodeId>,
    inputs: &mut Vec<Input>,
) -> Option<[f32; 3]> {
    inputs.clear();
    stack.clear();
    match options {
        Options::Sweep(_) => return gather_sweep(scene, id, purpose, frame, inputs),
        Options::Loft(_) => return gather_loft(scene, id, purpose, frame, inputs),
        Options::Boolean(_) => return gather_solids(scene, id, purpose, frame, pictures, Operands::Two, 0, inputs),
        Options::Symmetry(_) | Options::Join(_) => {
            return gather_solids(scene, id, purpose, frame, pictures, Operands::All, 0, inputs);
        }
        Options::Reduce(reduce) => {
            let operands = if reduce.together { Operands::All } else { Operands::Each };
            return gather_solids(scene, id, purpose, frame, pictures, operands, 0, inputs);
        }
        Options::SplineBoolean(_) => return gather_curves(scene, id, purpose, frame, 0, inputs),
        Options::Subdivision(_) | Options::Extrude { .. } | Options::Lathe(_) => {}
    }
    let &first = scene.children(id).first()?;
    let (colour, walk) = match *options {
        // The cage's colour: the smooth surface stands in for it.
        Options::Subdivision(_) => (scene.get(first)?.data.display_color, true),
        // Its own: a spline's line colour would not suit a solid.
        Options::Extrude { hierarchical, .. } => (scene.get(id)?.data.display_color, hierarchical),
        Options::Lathe(_) => (scene.get(id)?.data.display_color, false),
        Options::Sweep(_)
        | Options::Loft(_)
        | Options::Boolean(_)
        | Options::Symmetry(_)
        | Options::Join(_)
        | Options::SplineBoolean(_)
        | Options::Reduce(_) => {
            return None;
        }
    };
    let into_generator = scene.world_matrix_at(id, frame).inverse();
    stack.push(first);
    while let Some(node_id) = stack.pop() {
        let Some(node) = scene.get(node_id) else { continue };
        let data = &node.data;
        if !purpose.shows(data) || !scene.layers().shows_own(data.layer, purpose == Purpose::Render) || !data.enabled {
            continue; // hidden parts are left out
        }
        let kind = data.kind_at(frame);
        if data.kind.is_generator() {
            // A cage may be a generator of solids (a Join, a Boolean, a
            // Symmetry): its result is smoothed; other generators inside a
            // cage are left out.
            if matches!(options, Options::Subdivision(_))
                && let Some(nested) = options_of_solids(&kind, purpose)
            {
                let mut gathered = Vec::new();
                if gather_solids(scene, node_id, purpose, frame, pictures, nested.1, 1, &mut gathered).is_some() {
                    inputs.push(Input {
                        source: Source::Generated(nested.0, gathered),
                        matrix: into_generator * scene.world_matrix_at(node_id, frame),
                        links: Vec::new(),
                        operand: 0,
                    });
                }
            }
            // An Extrude's or a Lathe's spline may be a generator's curve.
            if matches!(options, Options::Extrude { .. } | Options::Lathe(_))
                && let Some(source) = curve_generator(scene, node_id, &kind, purpose, frame, 1)
            {
                inputs.push(Input {
                    source,
                    matrix: into_generator * scene.world_matrix_at(node_id, frame),
                    links: Vec::new(),
                    operand: 0,
                });
            }
            continue;
        }
        let source = match (&*kind, options) {
            (ObjectKind::Polygon { mesh, components }, Options::Subdivision(_)) => {
                Some(Source::Mesh(mesh.clone(), EdgeMarks::of(mesh, components)))
            }
            (kind, Options::Subdivision(_)) if editable_mesh_exists(kind) => Some(Source::Primitive(kind.clone())),
            (ObjectKind::SplinePrimitive(primitive), Options::Extrude { .. } | Options::Lathe(_)) => {
                Some(Source::SplinePrimitive(*primitive))
            }
            (ObjectKind::Text(text), Options::Extrude { .. } | Options::Lathe(_)) => Some(Source::Text(text.clone())),
            (
                ObjectKind::Spline {
                    spline, interpolation, ..
                },
                Options::Extrude { .. } | Options::Lathe(_),
            ) => Some(Source::Spline(spline.clone(), *interpolation)),
            _ => None,
        };
        if let Some(source) = source {
            let links = match source {
                Source::Mesh(..) | Source::Primitive(_) => deform::input_chain(scene, node_id, id, frame, pictures),
                Source::SplinePrimitive(_) | Source::Text(_) | Source::Spline(..) | Source::Generated(..) => Vec::new(),
            };
            inputs.push(Input {
                source,
                matrix: into_generator * scene.world_matrix_at(node_id, frame),
                links,
                operand: 0,
            });
        }
        if walk {
            // Children in reverse so they pop in order.
            stack.extend(node.children().iter().rev());
        }
    }
    Some(colour)
}

/// The Sweep's inputs: its profile (in the profile's own space) and its path
/// (in the generator's), both visible, enabled splines; nothing otherwise.
fn gather_sweep(scene: &Scene, id: NodeId, purpose: Purpose, frame: f64, inputs: &mut Vec<Input>) -> Option<[f32; 3]> {
    let colour = scene.get(id)?.data.display_color;
    let into_generator = scene.world_matrix_at(id, frame).inverse();
    for (k, &child) in scene.children(id).iter().take(2).enumerate() {
        let Some(data) = scene.get(child).map(|node| &node.data) else {
            break;
        };
        let kind = data.kind_at(frame);
        let Some(source) = curve_source(scene, child, &kind, purpose, frame, 1) else {
            break;
        };
        if !purpose.shows(data) || !scene.layers().shows_own(data.layer, purpose == Purpose::Render) || !data.enabled {
            break;
        }
        let matrix = if k == 0 {
            Mat4::IDENTITY
        } else {
            into_generator * scene.world_matrix_at(child, frame)
        };
        inputs.push(Input {
            source,
            matrix,
            links: Vec::new(),
            operand: 0,
        });
    }
    if inputs.len() < 2 {
        inputs.clear();
    }
    Some(colour)
}

/// The Loft's inputs: every visible, enabled spline child, in order, in the
/// generator's space.
fn gather_loft(scene: &Scene, id: NodeId, purpose: Purpose, frame: f64, inputs: &mut Vec<Input>) -> Option<[f32; 3]> {
    let colour = scene.get(id)?.data.display_color;
    let into_generator = scene.world_matrix_at(id, frame).inverse();
    for &child in scene.children(id) {
        let Some(data) = scene.get(child).map(|node| &node.data) else {
            continue;
        };
        let kind = data.kind_at(frame);
        let Some(source) = curve_source(scene, child, &kind, purpose, frame, 1) else {
            continue;
        };
        if purpose.shows(data) && scene.layers().shows_own(data.layer, purpose == Purpose::Render) && data.enabled {
            inputs.push(Input {
                source,
                matrix: into_generator * scene.world_matrix_at(child, frame),
                links: Vec::new(),
                operand: 0,
            });
        }
    }
    Some(colour)
}

/// The Spline Boolean's inputs: every visible, enabled spline child (or
/// generator of curves, up to [`MAX_NESTING`] deep), in order, in the
/// generator's space. Its colour is its own (the result is its line).
fn gather_curves(
    scene: &Scene,
    id: NodeId,
    purpose: Purpose,
    frame: f64,
    nesting: u32,
    inputs: &mut Vec<Input>,
) -> Option<[f32; 3]> {
    let colour = scene.get(id)?.data.display_color;
    let into_generator = scene.world_matrix_at(id, frame).inverse();
    for &child in scene.children(id) {
        let Some(data) = scene.get(child).map(|node| &node.data) else {
            continue;
        };
        if !purpose.shows(data) || !scene.layers().shows_own(data.layer, purpose == Purpose::Render) || !data.enabled {
            continue;
        }
        let kind = data.kind_at(frame);
        if let Some(source) = curve_source(scene, child, &kind, purpose, frame, nesting + 1) {
            inputs.push(Input {
                source,
                matrix: into_generator * scene.world_matrix_at(child, frame),
                links: Vec::new(),
                operand: 0,
            });
        }
    }
    Some(colour)
}

/// What a spline input reads from `kind` (object `id`): a spline's
/// parameters or content, or a generator of curves with its own inputs;
/// `None` for anything else.
fn curve_source(
    scene: &Scene,
    id: NodeId,
    kind: &ObjectKind,
    purpose: Purpose,
    frame: f64,
    nesting: u32,
) -> Option<Source> {
    match kind {
        ObjectKind::SplinePrimitive(primitive) => Some(Source::SplinePrimitive(*primitive)),
        ObjectKind::Text(text) => Some(Source::Text(text.clone())),
        ObjectKind::Spline {
            spline, interpolation, ..
        } => Some(Source::Spline(spline.clone(), *interpolation)),
        _ => curve_generator(scene, id, kind, purpose, frame, nesting),
    }
}

/// A generator of curves (a Spline Boolean, enabled) as a spline input: its
/// options and inputs; `None` for anything else or too deep.
fn curve_generator(
    scene: &Scene,
    id: NodeId,
    kind: &ObjectKind,
    purpose: Purpose,
    frame: f64,
    nesting: u32,
) -> Option<Source> {
    let ObjectKind::SplineBoolean(settings) = *kind else {
        return None;
    };
    let data = &scene.get(id)?.data;
    if !data.enabled || nesting > MAX_NESTING || !scene.layer_switches(id).generators {
        return None;
    }
    let mut nested = Vec::new();
    gather_curves(scene, id, purpose, frame, nesting, &mut nested)?;
    Some(Source::Generated(Options::SplineBoolean(settings), nested))
}

/// A generator of solids' options and which children it takes; `None` for
/// other kinds.
fn options_of_solids(kind: &ObjectKind, purpose: Purpose) -> Option<(Options, Operands)> {
    match options(kind, purpose)? {
        options @ Options::Boolean(_) => Some((options, Operands::Two)),
        options @ (Options::Symmetry(_) | Options::Join(_)) => Some((options, Operands::All)),
        options @ Options::Reduce(reduce) => {
            Some((options, if reduce.together { Operands::All } else { Operands::Each }))
        }
        Options::Subdivision(_)
        | Options::Extrude { .. }
        | Options::Lathe(_)
        | Options::Sweep(_)
        | Options::Loft(_)
        | Options::SplineBoolean(_) => None,
    }
}

/// Which children a generator of solids takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operands {
    /// The first two, apart (a Boolean's A and B).
    Two,
    /// All of them, as one (a Symmetry's or a Join's).
    All,
    /// All of them, each apart (a Reduce's that reduces children apart).
    Each,
}

/// The inputs of a generator of solids (a Boolean, Symmetry, Join): its
/// first two children (operand 0 and 1) or all of them (operand 0), each with
/// its visible, enabled descendants that have a surface, in the generator's
/// space. A generator among them enters with its own inputs (its children are
/// its, not this one's), up to [`MAX_NESTING`] deep. The colour returned is
/// the Boolean's own, else the first child's (the result stands in for it).
#[expect(clippy::too_many_arguments, reason = "where and when it is evaluated, passed apart")]
fn gather_solids(
    scene: &Scene,
    id: NodeId,
    purpose: Purpose,
    frame: f64,
    pictures: &Pictures,
    operands: Operands,
    nesting: u32,
    inputs: &mut Vec<Input>,
) -> Option<[f32; 3]> {
    let own = scene.get(id)?.data.display_color;
    let colour = match operands {
        Operands::Two => own,
        Operands::All | Operands::Each => scene
            .children(id)
            .first()
            .and_then(|&c| scene.get(c))
            .map_or(own, |c| c.data.display_color),
    };
    let into_generator = scene.world_matrix_at(id, frame).inverse();
    let mut stack = Vec::new();
    let count = match operands {
        Operands::Two => 2,
        Operands::All | Operands::Each => usize::MAX,
    };
    for (k, &child) in (0u32..).zip(scene.children(id).iter().take(count)) {
        let operand = match operands {
            Operands::Two | Operands::Each => k,
            Operands::All => 0,
        };
        stack.push(child);
        while let Some(node_id) = stack.pop() {
            let Some(node) = scene.get(node_id) else { continue };
            let data = &node.data;
            if !purpose.shows(data)
                || !scene.layers().shows_own(data.layer, purpose == Purpose::Render)
                || !data.enabled
            {
                continue;
            }
            let kind = data.kind_at(frame);
            let matrix = into_generator * scene.world_matrix_at(node_id, frame);
            if let Some(options) = options(&kind, purpose).filter(|_| nesting < MAX_NESTING) {
                // A generator: its result stands in for it and its children.
                let mut nested = Vec::new();
                let gathered = match options {
                    Options::Boolean(_) => gather_solids(
                        scene,
                        node_id,
                        purpose,
                        frame,
                        pictures,
                        Operands::Two,
                        nesting + 1,
                        &mut nested,
                    ),
                    Options::Symmetry(_) | Options::Join(_) => gather_solids(
                        scene,
                        node_id,
                        purpose,
                        frame,
                        pictures,
                        Operands::All,
                        nesting + 1,
                        &mut nested,
                    ),
                    Options::Reduce(reduce) => {
                        let operands = if reduce.together { Operands::All } else { Operands::Each };
                        gather_solids(
                            scene,
                            node_id,
                            purpose,
                            frame,
                            pictures,
                            operands,
                            nesting + 1,
                            &mut nested,
                        )
                    }
                    _ => gather(
                        scene,
                        node_id,
                        &options,
                        purpose,
                        frame,
                        pictures,
                        &mut Vec::new(),
                        &mut nested,
                    ),
                };
                if gathered.is_some() {
                    inputs.push(Input {
                        source: Source::Generated(options, nested),
                        matrix,
                        links: Vec::new(),
                        operand,
                    });
                }
                continue;
            }
            let source = match &*kind {
                ObjectKind::Polygon { mesh, components } => {
                    Some(Source::Mesh(mesh.clone(), EdgeMarks::of(mesh, components)))
                }
                kind if editable_mesh_exists(kind) => Some(Source::Primitive(kind.clone())),
                _ => None,
            };
            if let Some(source) = source {
                inputs.push(Input {
                    source,
                    matrix,
                    links: deform::input_chain(scene, node_id, id, frame, pictures),
                    operand,
                });
            }
            stack.extend(node.children().iter().rev());
        }
    }
    Some(colour)
}

/// Adds a mesh input (a polygon object, a primitive, another generator's
/// result) to `merged`; curves add nothing.
fn add_surface(merged: &mut Merged, input: &Input) {
    match &input.source {
        Source::Mesh(handle, marks) => {
            let offset = u32::try_from(merged.points.len()).unwrap_or(u32::MAX);
            if input.links.is_empty() {
                merged.add(handle.mesh(), input.matrix);
            } else {
                merged.add(&deform::deform_mesh(handle.mesh(), &input.links), input.matrix);
            }
            merged.mark(marks, offset);
        }
        Source::Primitive(kind) => {
            if let Some(mesh) = editable_mesh(kind) {
                merged.add(&deform::deform_mesh(&mesh, &input.links), input.matrix);
            }
        }
        Source::Generated(options, inputs) => merged.add(&build_all(inputs, options).mesh, input.matrix),
        Source::SplinePrimitive(_) | Source::Text(_) | Source::Spline(..) => {}
    }
}

/// The generator's result from `inputs`, in its space — the Subdivision
/// Surface's merged and smoothed cage, the Extrude's solids merged, the
/// Lathe's body, the Boolean's combination — with what else it has to say
/// (a Boolean's cut edges and note).
pub(crate) fn build_all(inputs: &[Input], options: &Options) -> Built {
    let mut merged = Merged::default();
    let mesh = match options {
        Options::Boolean(settings) => return build_boolean(inputs, *settings),
        Options::SplineBoolean(settings) => {
            let shapes: Vec<Polyline> = inputs
                .iter()
                .filter_map(|input| {
                    let mut shape = input.source.polyline()?;
                    for point in &mut shape.points {
                        *point = input.matrix.transform_point3(*point);
                    }
                    Some(shape)
                })
                .collect();
            return Built {
                curve: Some(spline_boolean::spline_boolean(&shapes, settings)),
                ..Built::default()
            };
        }
        Options::Reduce(reduce) => return replay(&reduce_orders(inputs, *reduce), reduce.keep),
        Options::Symmetry(settings) => {
            for input in inputs {
                add_surface(&mut merged, input);
            }
            symmetry::mirror(&merged.finish(), settings)
        }
        Options::Join(settings) => {
            for input in inputs {
                add_surface(&mut merged, input);
            }
            let mesh = merged.finish();
            if settings.weld && mesh.face_count() > 0 {
                let weld = OptimizeOptions {
                    merge_points: true,
                    tolerance: settings.tolerance.clamp(0.0, Join::MAX_TOLERANCE),
                    unused_points: true,
                };
                optimize(&mesh, None, &weld).map_or(mesh, |rebuilt| rebuilt.mesh)
            } else {
                mesh
            }
        }
        Options::Subdivision(options) => {
            for input in inputs {
                add_surface(&mut merged, input);
            }
            let creases: subdivision::Creases = merged
                .creases
                .iter()
                .map(|(&edge, &percent)| (edge, subdivision::sharpness(percent)))
                .collect();
            subdivision::subdivide_creased(&merged.finish(), options, &creases)
        }
        Options::Extrude { extrusion, .. } => {
            for input in inputs {
                let Some(mut polyline) = input.source.polyline() else {
                    continue;
                };
                for point in &mut polyline.points {
                    *point = input.matrix.transform_point3(*point);
                }
                merged.add(&surface::extrude(&polyline, extrusion), Mat4::IDENTITY);
            }
            merged.finish()
        }
        Options::Lathe(lathe) => {
            for input in inputs {
                let Some(mut polyline) = input.source.polyline() else {
                    continue;
                };
                for point in &mut polyline.points {
                    *point = input.matrix.transform_point3(*point);
                }
                merged.add(&surface::lathe(&polyline, lathe), Mat4::IDENTITY);
            }
            merged.finish()
        }
        Options::Sweep(sweep) => {
            let [profile, path] = inputs else {
                return Built::default();
            };
            let (Some(contour), Some(mut route)) = (profile.source.polyline(), path.source.polyline()) else {
                return Built::default();
            };
            for point in &mut route.points {
                *point = path.matrix.transform_point3(*point);
            }
            surface::sweep(&contour, &route, sweep)
        }
        Options::Loft(loft) => {
            let sections: Vec<Polyline> = inputs
                .iter()
                .filter_map(|input| {
                    let mut section = input.source.polyline()?;
                    for point in &mut section.points {
                        *point = input.matrix.transform_point3(*point);
                    }
                    Some(section)
                })
                .collect();
            surface::loft(&sections, loft)
        }
    };
    Built {
        mesh,
        ..Built::default()
    }
}

/// The collapse orders of a Reduce's inputs: one for all of them, or one
/// per child when it reduces them apart.
pub(crate) fn reduce_orders(inputs: &[Input], reduce: Reduce) -> Arc<Vec<CollapseOrder>> {
    let constraints = Constraints {
        borders: reduce.keep_borders,
        seams: reduce.keep_seams,
    };
    let mut groups: Vec<(u32, Merged)> = Vec::new();
    // Hard edges kept like borders when asked.
    for input in inputs {
        let at = match groups.iter().position(|(operand, _)| *operand == input.operand) {
            Some(at) => at,
            None => {
                groups.push((input.operand, Merged::default()));
                groups.len() - 1
            }
        };
        add_surface(&mut groups[at].1, input);
    }
    Arc::new(
        groups
            .into_iter()
            .map(|(_, merged)| {
                let kept = if reduce.keep_hard {
                    merged.hard.clone()
                } else {
                    std::collections::HashSet::new()
                };
                CollapseOrder::keeping(&merged.finish(), constraints, &kept)
            })
            .collect(),
    )
}

/// A Reduce's result: each order replayed to `keep` (0 – 1) and merged.
pub(crate) fn replay(orders: &[CollapseOrder], keep: f32) -> Built {
    let mut merged = Merged::default();
    for order in orders {
        merged.add(&order.replay(keep), Mat4::IDENTITY);
    }
    Built {
        mesh: merged.finish(),
        ..Built::default()
    }
}

/// The Boolean of its two operands; the first alone (with a note) when the
/// second is missing or the Boolean cannot be made.
fn build_boolean(inputs: &[Input], settings: Boolean) -> Built {
    let mut operands = [Merged::default(), Merged::default()];
    for input in inputs {
        add_surface(&mut operands[usize::from(input.operand >= 1)], input);
    }
    let [a, b] = operands;
    let (a, b) = (a.finish(), b.finish());
    if b.face_count() == 0 || a.face_count() == 0 {
        return Built {
            mesh: a,
            note: Some(GeneratorNote::MissingInput),
            ..Built::default()
        };
    }
    match boolean::boolean(&a, &b, settings.mode, settings.options()) {
        Ok(result) => Built {
            hard: if settings.hard_cut_edges {
                result.cut_edges.clone()
            } else {
                Vec::new()
            },
            mesh: result.mesh,
            cut_edges: result.cut_edges,
            ..Built::default()
        },
        Err(error) => Built {
            mesh: a,
            note: Some(match error {
                BooleanError::NotFinite => GeneratorNote::NotANumber,
                BooleanError::TooLarge => GeneratorNote::TooLarge,
            }),
            ..Built::default()
        },
    }
}

/// Meshes merged into one, with their UVs (a part without UVs gets a box
/// projection of its own, so the result always has them).
#[derive(Default)]
struct Merged {
    points: Vec<glam::Vec3>,
    faces: Vec<Vec<u32>>,
    uvs: Vec<glam::Vec2>,
    /// The merged parts' creased edges (by point pair) and their percent.
    creases: std::collections::HashMap<[u32; 2], u8>,
    /// The merged parts' hard edges (by point pair).
    hard: std::collections::HashSet<[u32; 2]>,
}

impl Merged {
    /// Adds a part's edge marks, its points numbered from `offset`.
    fn mark(&mut self, marks: &EdgeMarks, offset: u32) {
        let moved = |[a, b]: [u32; 2]| [a + offset, b + offset];
        for &(edge, percent) in &marks.creases {
            let entry = self.creases.entry(moved(edge)).or_insert(0);
            *entry = (*entry).max(percent);
        }
        self.hard.extend(marks.hard.iter().map(|&edge| moved(edge)));
    }

    /// Adds `mesh` placed by `matrix`.
    fn add(&mut self, mesh: &PolyMesh, matrix: Mat4) {
        let Ok(offset) = u32::try_from(self.points.len()) else {
            return;
        };
        let projected;
        let uvs = match mesh.uvs() {
            Some(uvs) => uvs,
            None => {
                projected = uv::box_uvs(mesh);
                &projected
            }
        };
        self.points
            .extend(mesh.points().iter().map(|&p| matrix.transform_point3(p)));
        // A mirroring transform turns the faces inside out: wind them back,
        // their UVs with them (the picture shows mirrored, as in a mirror).
        let mirrored = matrix.determinant() < 0.0;
        for f in 0..mesh.face_count() {
            let mut corners: Vec<u32> = mesh.face(f).iter().map(|&p| p + offset).collect();
            let start = self.uvs.len();
            self.uvs.extend(uvs.get(mesh.corner_range(f)).unwrap_or_default());
            if mirrored {
                corners.reverse();
                self.uvs[start..].reverse();
            }
            self.faces.push(corners);
        }
    }

    fn finish(self) -> PolyMesh {
        let mut mesh = PolyMesh::from_faces(self.points, self.faces).unwrap_or_default();
        if mesh.corner_count() == self.uvs.len() && !self.uvs.is_empty() {
            let _ = mesh.set_uvs(Some(self.uvs));
        }
        mesh
    }
}

/// The colour generator `id`'s result shows without a material: the
/// Subdivision Surface's cage's (the result stands in for it), a spline
/// generator's own; `None` if `id` is not a generator.
#[must_use]
pub fn generator_color(scene: &Scene, id: NodeId) -> Option<[f32; 3]> {
    let node = scene.get(id)?;
    match options(&node.data.kind, Purpose::Editor)? {
        Options::Subdivision(_) => scene
            .children(id)
            .first()
            .and_then(|&cage| scene.get(cage))
            .map(|cage| cage.data.display_color),
        // The result stands in for the first child.
        Options::Symmetry(_) | Options::Join(_) | Options::Reduce(_) => scene
            .children(id)
            .first()
            .and_then(|&first| scene.get(first))
            .map(|first| first.data.display_color),
        Options::Extrude { .. }
        | Options::Lathe(_)
        | Options::Sweep(_)
        | Options::Loft(_)
        | Options::Boolean(_)
        | Options::SplineBoolean(_) => Some(node.data.display_color),
    }
}

/// The mesh generator `id` shows in the viewport at `frame` (what Make
/// Editable bakes), in the generator's space; `None` if `id` is not a
/// generator or has no children.
#[must_use]
pub fn generator_mesh(scene: &Scene, id: NodeId, frame: f64) -> Option<PolyMesh> {
    generator_built(scene, id, frame).map(|built| built.mesh)
}

/// The curve a generator of curves (a Spline Boolean) shows at `frame`, in
/// its space (what Make Editable bakes); `None` for other objects.
#[must_use]
pub fn generator_curve(scene: &Scene, id: NodeId, frame: f64) -> Option<Polyline> {
    generator_built(scene, id, frame)?.curve
}

/// [`generator_mesh`] with what else the generator has to say (a Boolean's
/// cut edges, for Make Editable to select).
#[must_use]
pub fn generator_built(scene: &Scene, id: NodeId, frame: f64) -> Option<Built> {
    generator_built_with(scene, id, frame, &Pictures::default())
}

/// [`generator_built`] with the pictures Displace deformers inside its
/// inputs read (see [`crate::EvaluatedScene::set_pictures`]).
#[must_use]
pub fn generator_built_with(scene: &Scene, id: NodeId, frame: f64, pictures: &Pictures) -> Option<Built> {
    let options = options(&scene.get(id)?.data.kind_at(frame), Purpose::Editor)?;
    let mut inputs = Vec::new();
    gather(
        scene,
        id,
        &options,
        Purpose::Editor,
        frame,
        pictures,
        &mut Vec::new(),
        &mut inputs,
    )?;
    Some(build_all(&inputs, &options))
}

pub(crate) fn editable_mesh_exists(kind: &ObjectKind) -> bool {
    matches!(
        kind,
        ObjectKind::Cube { .. }
            | ObjectKind::Sphere { .. }
            | ObjectKind::Cylinder { .. }
            | ObjectKind::Plane { .. }
            | ObjectKind::Cone { .. }
            | ObjectKind::Torus { .. }
            | ObjectKind::Disc { .. }
            | ObjectKind::Capsule { .. }
    )
}
