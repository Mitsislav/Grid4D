//! Generators: objects that build their geometry from their children. The
//! Subdivision Surface smooths its first child — the *cage*, with the cage's own
//! descendants merged into one mesh — in the generator's space.

use glam::Mat4;
use grid4d_geom::{
    PolyMesh,
    subdivision::{self, SubdivisionOptions},
};
use grid4d_scene::{MeshHandle, NodeId, ObjectKind, Scene};

use crate::{Purpose, editable_mesh};

/// One mesh of a cage, in the generator's space: what decides the result, so an
/// unchanged list means an unchanged result (a selection change in the cage, or
/// a move of the generator itself, changes nothing).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Input {
    source: Source,
    /// The cage node's transform relative to the generator.
    matrix: Mat4,
}

#[derive(Clone, Debug, PartialEq)]
enum Source {
    /// A polygon object's mesh (compared by id).
    Mesh(MeshHandle),
    /// A primitive's parameters (its mesh is generated when needed).
    Primitive(ObjectKind),
}

/// The generator's options for `purpose` (its viewport or render levels),
/// `None` if `kind` is not a generator.
pub(crate) fn options(kind: &ObjectKind, purpose: Purpose) -> Option<SubdivisionOptions> {
    match *kind {
        ObjectKind::SubdivisionSurface {
            viewport_levels,
            render_levels,
            linear_pre,
            boundary,
        } => Some(SubdivisionOptions {
            levels: match purpose {
                Purpose::Editor => viewport_levels,
                Purpose::Render => render_levels,
            },
            linear_pre,
            boundary,
        }),
        _ => None,
    }
}

/// Fills `inputs` with the cage of generator `id` (its first child and that
/// child's visible, enabled descendants that have geometry), and returns the
/// cage's display colour. `stack` is scratch space.
pub(crate) fn gather(
    scene: &Scene,
    id: NodeId,
    purpose: Purpose,
    stack: &mut Vec<NodeId>,
    inputs: &mut Vec<Input>,
) -> Option<[f32; 3]> {
    inputs.clear();
    stack.clear();
    let &cage = scene.children(id).first()?;
    let colour = scene.get(cage)?.data.display_color;
    let into_generator = scene.world_matrix(id).inverse();
    stack.push(cage);
    while let Some(node_id) = stack.pop() {
        let Some(node) = scene.get(node_id) else { continue };
        let data = &node.data;
        if !purpose.shows(data) || !data.enabled || data.kind.is_generator() {
            continue; // hidden parts are left out; nested generators are not merged
        }
        let source = match &data.kind {
            ObjectKind::Polygon { mesh, .. } => Some(Source::Mesh(mesh.clone())),
            kind if editable_mesh_exists(kind) => Some(Source::Primitive(kind.clone())),
            _ => None,
        };
        if let Some(source) = source {
            inputs.push(Input {
                source,
                matrix: into_generator * scene.world_matrix(node_id),
            });
        }
        // Children in reverse so they pop in order.
        stack.extend(node.children().iter().rev());
    }
    Some(colour)
}

/// The smoothed mesh of `inputs`: merged into one mesh in the generator's space,
/// then subdivided.
pub(crate) fn build(inputs: &[Input], options: &SubdivisionOptions) -> PolyMesh {
    let mut points = Vec::new();
    let mut faces: Vec<Vec<u32>> = Vec::new();
    for input in inputs {
        let generated;
        let mesh = match &input.source {
            Source::Mesh(handle) => handle.mesh(),
            Source::Primitive(kind) => {
                let Some(mesh) = editable_mesh(kind) else { continue };
                generated = mesh;
                &generated
            }
        };
        let Ok(offset) = u32::try_from(points.len()) else {
            continue;
        };
        points.extend(mesh.points().iter().map(|&p| input.matrix.transform_point3(p)));
        // A mirroring transform turns the faces inside out: wind them back.
        let mirrored = input.matrix.determinant() < 0.0;
        faces.extend(mesh.faces().map(|face| {
            let mut corners: Vec<u32> = face.iter().map(|&p| p + offset).collect();
            if mirrored {
                corners.reverse();
            }
            corners
        }));
    }
    let merged = PolyMesh::from_faces(points, faces).unwrap_or_default();
    subdivision::subdivide(&merged, options)
}

/// The mesh generator `id` shows in the viewport (what Make Editable bakes), in
/// the generator's space; `None` if `id` is not a generator or has no cage.
#[must_use]
pub fn generator_mesh(scene: &Scene, id: NodeId) -> Option<PolyMesh> {
    let options = options(&scene.get(id)?.data.kind, Purpose::Editor)?;
    let mut inputs = Vec::new();
    gather(scene, id, Purpose::Editor, &mut Vec::new(), &mut inputs)?;
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
