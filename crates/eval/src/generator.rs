//! Generators: objects that build their geometry from their children. The
//! Subdivision Surface smooths its first child — the *cage*, with the cage's own
//! descendants merged into one mesh — in the generator's space; the Extrude
//! pushes its first child spline (or, hierarchical, every spline below it) into
//! a solid; the Lathe turns its first child spline around its Y axis.

use glam::Mat4;
use grid4d_geom::{
    PolyMesh,
    spline::{Interpolation, Polyline, SplinePrimitive},
    subdivision::{self, SubdivisionOptions},
    surface::{self, Extrusion, Lathe},
};
use grid4d_scene::{MeshHandle, NodeId, ObjectKind, Scene, SplineHandle};

use crate::{Purpose, editable_mesh};

/// What a generator makes from its inputs, for one evaluation's purpose.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Options {
    Subdivision(SubdivisionOptions),
    Extrude { extrusion: Extrusion, hierarchical: bool },
    Lathe(Lathe),
}

/// One input of a generator, in the generator's space: what decides the result,
/// so an unchanged list means an unchanged result (a selection change in the
/// cage, or a move of the generator itself, changes nothing).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Input {
    source: Source,
    /// The input node's transform relative to the generator.
    matrix: Mat4,
}

#[derive(Clone, Debug, PartialEq)]
enum Source {
    /// A polygon object's mesh (compared by id).
    Mesh(MeshHandle),
    /// A primitive's parameters (its mesh is generated when needed).
    Primitive(ObjectKind),
    /// A spline primitive's parameters (its curve is sampled when needed).
    SplinePrimitive(SplinePrimitive),
    /// An editable spline (compared by id) and how it is sampled.
    Spline(SplineHandle, Interpolation),
}

impl Source {
    /// The sampled curve of a spline input; `None` for meshes.
    fn polyline(&self) -> Option<Polyline> {
        match self {
            Self::SplinePrimitive(primitive) => Some(primitive.polyline()),
            Self::Spline(spline, interpolation) => Some(spline.spline().sample(*interpolation)),
            Self::Mesh(_) | Self::Primitive(_) => None,
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
        _ => None,
    }
}

/// Fills `inputs` with the inputs of generator `id` — for a Subdivision
/// Surface its cage (the first child and that child's visible, enabled
/// descendants that have geometry), for an Extrude its spline(s) — and returns
/// the colour the result shows without a material. `stack` is scratch space.
pub(crate) fn gather(
    scene: &Scene,
    id: NodeId,
    options: &Options,
    purpose: Purpose,
    stack: &mut Vec<NodeId>,
    inputs: &mut Vec<Input>,
) -> Option<[f32; 3]> {
    inputs.clear();
    stack.clear();
    let &first = scene.children(id).first()?;
    let (colour, walk) = match *options {
        // The cage's colour: the smooth surface stands in for it.
        Options::Subdivision(_) => (scene.get(first)?.data.display_color, true),
        // Its own: a spline's line colour would not suit a solid.
        Options::Extrude { hierarchical, .. } => (scene.get(id)?.data.display_color, hierarchical),
        Options::Lathe(_) => (scene.get(id)?.data.display_color, false),
    };
    let into_generator = scene.world_matrix(id).inverse();
    stack.push(first);
    while let Some(node_id) = stack.pop() {
        let Some(node) = scene.get(node_id) else { continue };
        let data = &node.data;
        if !purpose.shows(data) || !data.enabled || data.kind.is_generator() {
            continue; // hidden parts are left out; nested generators are not merged
        }
        let source = match (&data.kind, options) {
            (ObjectKind::Polygon { mesh, .. }, Options::Subdivision(_)) => Some(Source::Mesh(mesh.clone())),
            (kind, Options::Subdivision(_)) if editable_mesh_exists(kind) => Some(Source::Primitive(kind.clone())),
            (ObjectKind::SplinePrimitive(primitive), Options::Extrude { .. } | Options::Lathe(_)) => {
                Some(Source::SplinePrimitive(*primitive))
            }
            (
                ObjectKind::Spline {
                    spline, interpolation, ..
                },
                Options::Extrude { .. } | Options::Lathe(_),
            ) => Some(Source::Spline(spline.clone(), *interpolation)),
            _ => None,
        };
        if let Some(source) = source {
            inputs.push(Input {
                source,
                matrix: into_generator * scene.world_matrix(node_id),
            });
        }
        if walk {
            // Children in reverse so they pop in order.
            stack.extend(node.children().iter().rev());
        }
    }
    Some(colour)
}

/// The generator's result from `inputs`, in its space: the Subdivision
/// Surface's merged and smoothed cage, the Extrude's solids merged, or the
/// Lathe's body.
pub(crate) fn build(inputs: &[Input], options: &Options) -> PolyMesh {
    let mut merged = Merged::default();
    match options {
        Options::Subdivision(options) => {
            for input in inputs {
                match &input.source {
                    Source::Mesh(handle) => merged.add(handle.mesh(), input.matrix),
                    Source::Primitive(kind) => {
                        if let Some(mesh) = editable_mesh(kind) {
                            merged.add(&mesh, input.matrix);
                        }
                    }
                    Source::SplinePrimitive(_) | Source::Spline(..) => {}
                }
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
    }
}

/// Meshes merged into one.
#[derive(Default)]
struct Merged {
    points: Vec<glam::Vec3>,
    faces: Vec<Vec<u32>>,
}

impl Merged {
    /// Adds `mesh` placed by `matrix`.
    fn add(&mut self, mesh: &PolyMesh, matrix: Mat4) {
        let Ok(offset) = u32::try_from(self.points.len()) else {
            return;
        };
        self.points
            .extend(mesh.points().iter().map(|&p| matrix.transform_point3(p)));
        // A mirroring transform turns the faces inside out: wind them back.
        let mirrored = matrix.determinant() < 0.0;
        self.faces.extend(mesh.faces().map(|face| {
            let mut corners: Vec<u32> = face.iter().map(|&p| p + offset).collect();
            if mirrored {
                corners.reverse();
            }
            corners
        }));
    }

    fn finish(self) -> PolyMesh {
        PolyMesh::from_faces(self.points, self.faces).unwrap_or_default()
    }
}

/// The mesh generator `id` shows in the viewport (what Make Editable bakes), in
/// the generator's space; `None` if `id` is not a generator or has no children.
#[must_use]
pub fn generator_mesh(scene: &Scene, id: NodeId) -> Option<PolyMesh> {
    let options = options(&scene.get(id)?.data.kind, Purpose::Editor)?;
    let mut inputs = Vec::new();
    gather(scene, id, &options, Purpose::Editor, &mut Vec::new(), &mut inputs)?;
    Some(build(&inputs, &options))
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
