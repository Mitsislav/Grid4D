//! Generators: objects that build their geometry from their children. The
//! Subdivision Surface smooths its first child — the *cage*, with the cage's own
//! descendants merged into one mesh — in the generator's space; the Extrude
//! pushes its first child spline (or, hierarchical, every spline below it) into
//! a solid; the Lathe turns its first child spline around its Y axis; the
//! Sweep moves its first child spline (the profile) along its second (the path);
//! the Loft stretches a skin over all its child splines; the Boolean combines
//! its first two children (each with its own children, other generators'
//! results included) as solids.

use glam::Mat4;
use grid4d_geom::{
    PolyMesh,
    boolean::{self, BooleanError},
    spline::{Interpolation, Polyline, SplinePrimitive, Text},
    subdivision::{self, SubdivisionOptions},
    surface::{self, Extrusion, Lathe, Loft, Sweep},
    uv,
};
use grid4d_scene::{Boolean, MeshHandle, NodeId, ObjectKind, Scene, SplineHandle};

use crate::{
    Purpose,
    deform::{self, Link},
    editable_mesh,
};

/// What a generator makes from its inputs, for one evaluation's purpose.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Options {
    Subdivision(SubdivisionOptions),
    Extrude { extrusion: Extrusion, hierarchical: bool },
    Lathe(Lathe),
    Sweep(Sweep),
    Loft(Loft),
    Boolean(Boolean),
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
    /// A Boolean's edges along the cut (point pairs of `mesh`).
    pub cut_edges: Vec<[u32; 2]>,
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
    /// Which of a Boolean's two inputs it belongs to (0 for others).
    operand: u8,
}

#[derive(Clone, Debug, PartialEq)]
enum Source {
    /// A polygon object's mesh (compared by id).
    Mesh(MeshHandle),
    /// A primitive's parameters (its mesh is generated when needed).
    Primitive(ObjectKind),
    /// A spline primitive's parameters (its curve is sampled when needed).
    SplinePrimitive(SplinePrimitive),
    /// A text's parameters (its outline is made when needed).
    Text(Text),
    /// An editable spline (compared by id) and how it is sampled.
    Spline(SplineHandle, Interpolation),
    /// Another generator inside a Boolean's input: its options and inputs
    /// (in its own space; its result is built when needed).
    Generated(Options, Vec<Input>),
}

impl Source {
    /// The sampled curve of a spline input; `None` for meshes.
    fn polyline(&self) -> Option<Polyline> {
        match self {
            Self::SplinePrimitive(primitive) => Some(primitive.polyline()),
            Self::Text(text) => Some(grid4d_text::text_spline(text).sample(text.interpolation)),
            Self::Spline(spline, interpolation) => Some(spline.spline().sample(*interpolation)),
            Self::Mesh(_) | Self::Primitive(_) | Self::Generated(..) => None,
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
        _ => None,
    }
}

/// Fills `inputs` with the inputs of generator `id` — for a Subdivision
/// Surface its cage (the first child and that child's visible, enabled
/// descendants that have geometry), for an Extrude its spline(s) — placed as
/// they are at `frame` (animation included), and returns the colour the result
/// shows without a material. `stack` is scratch space.
pub(crate) fn gather(
    scene: &Scene,
    id: NodeId,
    options: &Options,
    purpose: Purpose,
    frame: f64,
    stack: &mut Vec<NodeId>,
    inputs: &mut Vec<Input>,
) -> Option<[f32; 3]> {
    inputs.clear();
    stack.clear();
    match options {
        Options::Sweep(_) => return gather_sweep(scene, id, purpose, frame, inputs),
        Options::Loft(_) => return gather_loft(scene, id, purpose, frame, inputs),
        Options::Boolean(_) => return gather_boolean(scene, id, purpose, frame, 0, inputs),
        Options::Subdivision(_) | Options::Extrude { .. } | Options::Lathe(_) => {}
    }
    let &first = scene.children(id).first()?;
    let (colour, walk) = match *options {
        // The cage's colour: the smooth surface stands in for it.
        Options::Subdivision(_) => (scene.get(first)?.data.display_color, true),
        // Its own: a spline's line colour would not suit a solid.
        Options::Extrude { hierarchical, .. } => (scene.get(id)?.data.display_color, hierarchical),
        Options::Lathe(_) => (scene.get(id)?.data.display_color, false),
        Options::Sweep(_) | Options::Loft(_) | Options::Boolean(_) => return None,
    };
    let into_generator = scene.world_matrix_at(id, frame).inverse();
    stack.push(first);
    while let Some(node_id) = stack.pop() {
        let Some(node) = scene.get(node_id) else { continue };
        let data = &node.data;
        if !purpose.shows(data)
            || !scene.layers().shows_own(data.layer, purpose == Purpose::Render)
            || !data.enabled
            || data.kind.is_generator()
        {
            continue; // hidden parts are left out; nested generators are not merged
        }
        let kind = data.kind_at(frame);
        let source = match (&*kind, options) {
            (ObjectKind::Polygon { mesh, .. }, Options::Subdivision(_)) => Some(Source::Mesh(mesh.clone())),
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
                Source::Mesh(_) | Source::Primitive(_) => deform::input_chain(scene, node_id, id, frame),
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
        let source = match &*kind {
            ObjectKind::SplinePrimitive(primitive) => Source::SplinePrimitive(*primitive),
            ObjectKind::Text(text) => Source::Text(text.clone()),
            ObjectKind::Spline {
                spline, interpolation, ..
            } => Source::Spline(spline.clone(), *interpolation),
            _ => break,
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
        let source = match &*kind {
            ObjectKind::SplinePrimitive(primitive) => Source::SplinePrimitive(*primitive),
            ObjectKind::Text(text) => Source::Text(text.clone()),
            ObjectKind::Spline {
                spline, interpolation, ..
            } => Source::Spline(spline.clone(), *interpolation),
            _ => continue,
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

/// The Boolean's inputs: its first two children, each with its visible,
/// enabled descendants that have a surface (operand 0 and 1), in the
/// generator's space. A generator among them enters with its own inputs
/// (its children are its, not the Boolean's), up to [`MAX_NESTING`] deep.
fn gather_boolean(
    scene: &Scene,
    id: NodeId,
    purpose: Purpose,
    frame: f64,
    nesting: u32,
    inputs: &mut Vec<Input>,
) -> Option<[f32; 3]> {
    let colour = scene.get(id)?.data.display_color;
    let into_generator = scene.world_matrix_at(id, frame).inverse();
    let mut stack = Vec::new();
    for (operand, &child) in (0u8..).zip(scene.children(id).iter().take(2)) {
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
                    Options::Boolean(_) => gather_boolean(scene, node_id, purpose, frame, nesting + 1, &mut nested),
                    _ => gather(scene, node_id, &options, purpose, frame, &mut Vec::new(), &mut nested),
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
                ObjectKind::Polygon { mesh, .. } => Some(Source::Mesh(mesh.clone())),
                kind if editable_mesh_exists(kind) => Some(Source::Primitive(kind.clone())),
                _ => None,
            };
            if let Some(source) = source {
                inputs.push(Input {
                    source,
                    matrix,
                    links: deform::input_chain(scene, node_id, id, frame),
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
        Source::Mesh(handle) if input.links.is_empty() => merged.add(handle.mesh(), input.matrix),
        Source::Mesh(handle) => merged.add(&deform::deform_mesh(handle.mesh(), &input.links), input.matrix),
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
        Options::Subdivision(options) => {
            for input in inputs {
                add_surface(&mut merged, input);
            }
            subdivision::subdivide(&merged.finish(), options)
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
        cut_edges: Vec::new(),
        note: None,
    }
}

/// The Boolean of its two operands; the first alone (with a note) when the
/// second is missing or the Boolean cannot be made.
fn build_boolean(inputs: &[Input], settings: Boolean) -> Built {
    let mut operands = [Merged::default(), Merged::default()];
    for input in inputs {
        add_surface(&mut operands[usize::from(input.operand.min(1))], input);
    }
    let [a, b] = operands;
    let (a, b) = (a.finish(), b.finish());
    if b.face_count() == 0 || a.face_count() == 0 {
        return Built {
            mesh: a,
            cut_edges: Vec::new(),
            note: Some(GeneratorNote::MissingInput),
        };
    }
    match boolean::boolean(&a, &b, settings.mode, settings.options()) {
        Ok(result) => Built {
            mesh: result.mesh,
            cut_edges: result.cut_edges,
            note: None,
        },
        Err(error) => Built {
            mesh: a,
            cut_edges: Vec::new(),
            note: Some(match error {
                BooleanError::NotFinite => GeneratorNote::NotANumber,
                BooleanError::TooLarge => GeneratorNote::TooLarge,
            }),
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
}

impl Merged {
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
        Options::Extrude { .. } | Options::Lathe(_) | Options::Sweep(_) | Options::Loft(_) | Options::Boolean(_) => {
            Some(node.data.display_color)
        }
    }
}

/// The mesh generator `id` shows in the viewport at `frame` (what Make
/// Editable bakes), in the generator's space; `None` if `id` is not a
/// generator or has no children.
#[must_use]
pub fn generator_mesh(scene: &Scene, id: NodeId, frame: f64) -> Option<PolyMesh> {
    generator_built(scene, id, frame).map(|built| built.mesh)
}

/// [`generator_mesh`] with what else the generator has to say (a Boolean's
/// cut edges, for Make Editable to select).
#[must_use]
pub fn generator_built(scene: &Scene, id: NodeId, frame: f64) -> Option<Built> {
    let options = options(&scene.get(id)?.data.kind_at(frame), Purpose::Editor)?;
    let mut inputs = Vec::new();
    gather(
        scene,
        id,
        &options,
        Purpose::Editor,
        frame,
        &mut Vec::new(),
        &mut inputs,
    )?;
    Some(build_all(&inputs, &options))
}

fn editable_mesh_exists(kind: &ObjectKind) -> bool {
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
