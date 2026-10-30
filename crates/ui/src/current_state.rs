//! Current State to Object: copies of objects as the viewport shows them at
//! the current frame — deformed, generated, animated — as plain objects.

use glam::Mat4;
use grid4d_eval::{EvaluatedNode, EvaluatedScene, MeshKey, Overrides};
use grid4d_scene::{
    Components, Document, MeshHandle, Node, NodeData, NodeId, ObjectKind, Scene, SplineHandle, Tag, TagKind, Tags,
    Transform,
};

/// One copy to insert.
pub(crate) struct Baked {
    /// The copy it goes under (an index into the list), or `None` for a top
    /// copy, placed right after its original.
    pub(crate) parent: Option<usize>,
    pub(crate) original: NodeId,
    pub(crate) data: NodeData,
}

/// The copies of the topmost selected objects and their visible subtrees, in
/// pre-order (parents before children): what the viewport shows at the current
/// frame, frozen. Deformers and the objects enabled generators consume are
/// left out.
/// `simulated`: the simulation's poses at the current frame.
pub(crate) fn bake(doc: &Document, selection: &[NodeId], simulated: &Overrides) -> Vec<Baked> {
    let frame = f64::from(doc.time.frame());
    let mut eval = EvaluatedScene::new();
    eval.set_frame(frame);
    eval.set_overrides(simulated);
    eval.update(&doc.scene);
    let nodes = eval.nodes();
    let mut baked = Vec::new();
    for &root in selection {
        if has_selected_ancestor(&doc.scene, selection, root) {
            continue;
        }
        let Some(start) = nodes.iter().position(|n| n.id == root) else {
            continue;
        };
        let top = nodes[start].depth;
        // The copies of the current ancestors, by depth.
        let mut parents: Vec<(u32, usize)> = Vec::new();
        let mut skip_below: Option<u32> = None;
        for (at, node) in nodes[start..].iter().enumerate() {
            if at > 0 && node.depth <= top {
                break;
            }
            if skip_below.is_some_and(|d| node.depth > d) {
                continue;
            }
            skip_below = None;
            while parents.last().is_some_and(|&(d, _)| d >= node.depth) {
                parents.pop();
            }
            let Some(original) = doc.scene.get(node.id) else {
                continue;
            };
            // Deformers live on in the shapes; consumed inputs in the result.
            if grid4d_eval::is_deformer(&original.data.kind) || (at > 0 && consumed(&doc.scene, node.id)) {
                skip_below = Some(node.depth);
                continue;
            }
            let parent = parents.last().map(|&(_, index)| index);
            parents.push((node.depth, baked.len()));
            let copied = baked.len();
            baked.push(Baked {
                parent,
                original: node.id,
                data: copy(&eval, &doc.scene, node, original, frame, simulated),
            });
            // A Cloner's copy (a Null) holds its clones as objects.
            if original.data.enabled && matches!(original.data.kind, ObjectKind::Cloner { .. }) {
                baked.extend(
                    clone_objects(&eval, &doc.scene, node.id, frame)
                        .into_iter()
                        .map(|data| Baked {
                            parent: Some(copied),
                            original: node.id,
                            data,
                        }),
                );
            }
        }
    }
    baked
}

/// The clones `eval` shows for `cloner` as plain objects placed in the
/// Cloner's space, one per shape (a cloned group gives its parts), named after
/// the object shown and the clone's number (`Cube.0`, `Cube.1`, …): polygon
/// objects of their shapes; a clone of a curve keeps the curve. Nothing for a
/// hidden or disabled Cloner.
pub(crate) fn clone_objects(eval: &EvaluatedScene, scene: &Scene, cloner: NodeId, frame: f64) -> Vec<NodeData> {
    let Some(world) = eval.nodes().iter().find(|n| n.id == cloner).map(|n| n.world) else {
        return Vec::new();
    };
    let inverse = world.inverse();
    eval.instances()
        .iter()
        .filter(|clone| clone.owner == cloner)
        .filter_map(|clone| {
            let source = scene.get(clone.source)?;
            let mut data = source.data.clone();
            data.name = format!("{}.{}", source.data.name, clone.index);
            data.transform = Transform::from_matrix(&(inverse * clone.world));
            data.tracks.clear();
            data.unkeyed = None;
            let kind = source.data.kind_at(frame);
            data.kind = match clone.drawable.map(|d| d.key) {
                Some(MeshKey::Polygon(id)) => ObjectKind::polygon(eval.mesh(id)?.clone()),
                Some(MeshKey::Curve(_)) => kind.into_owned(),
                Some(_) => ObjectKind::polygon(MeshHandle::new(grid4d_eval::editable_mesh(&kind)?)),
                None => ObjectKind::Null,
            };
            Some(data)
        })
        .collect()
}

/// `true` if an ancestor of `id` is selected too (it goes with that one).
fn has_selected_ancestor(scene: &Scene, selection: &[NodeId], id: NodeId) -> bool {
    let mut at = scene.get(id).and_then(Node::parent);
    while let Some(current) = at {
        if selection.contains(&current) {
            return true;
        }
        at = scene.get(current).and_then(Node::parent);
    }
    false
}

/// `true` if `id` is an input an enabled generator consumes (a cage, a profile).
fn consumed(scene: &Scene, id: NodeId) -> bool {
    let Some(parent) = scene.get(id).and_then(Node::parent).and_then(|p| scene.get(p)) else {
        return false;
    };
    parent.data.kind.is_generator()
        && parent.data.enabled
        && scene
            .index_in_parent(id)
            .is_some_and(|index| index < parent.data.kind.generator_inputs())
}

/// `original` as the viewport shows it at `frame`, with no animation.
fn copy(
    eval: &EvaluatedScene,
    scene: &Scene,
    node: &EvaluatedNode,
    original: &Node,
    frame: f64,
    simulated: &Overrides,
) -> NodeData {
    let mut data = original.data.clone();
    data.transform = original.data.transform_at(frame);
    // A simulated object where the simulation put it, under the same parent.
    if let Some(world) = simulated.get(node.id) {
        let parent = original
            .parent()
            .and_then(|p| eval.nodes().iter().find(|n| n.id == p))
            .map_or(Mat4::IDENTITY, |n| n.world);
        data.transform = Transform::from_matrix(&(parent.inverse() * world));
    }
    data.tracks.clear();
    data.unkeyed = None;
    let kind = original.data.kind_at(frame);
    // The surface it shows as a mesh (deformed, generated or its own).
    let shown = node
        .drawable
        .filter(|d| !d.unlit && !d.marker)
        .and_then(|d| match d.key {
            MeshKey::Polygon(id) => eval.mesh(id),
            _ => None,
        })
        .cloned();
    data.kind = match &*kind {
        ObjectKind::Polygon { mesh, components } => match eval.deformed(node.id) {
            None => kind.clone().into_owned(),
            Some(deformed) => {
                // Deforming moves points only: selections still fit unless
                // hidden polygons were left out of the shape.
                let fits = components.hidden().polygons.is_empty()
                    && deformed.mesh().face_count() == mesh.mesh().face_count()
                    && deformed.mesh().point_count() == mesh.mesh().point_count();
                ObjectKind::Polygon {
                    mesh: deformed.clone(),
                    components: if fits {
                        components.clone()
                    } else {
                        Components::default()
                    },
                }
            }
        },
        ObjectKind::SplinePrimitive(primitive) => ObjectKind::Spline {
            spline: SplineHandle::new(primitive.spline()),
            interpolation: primitive.interpolation,
            components: Components::default(),
        },
        ObjectKind::Text(text) => ObjectKind::Spline {
            spline: SplineHandle::new(grid4d_text::text_spline(text)),
            interpolation: text.interpolation,
            components: Components::default(),
        },
        generator if generator.is_generator() => match shown.filter(|_| original.data.enabled) {
            Some(mesh) => {
                if let Some(colour) = grid4d_eval::generator_color(scene, node.id) {
                    data.display_color = colour;
                }
                take_cage_tags(scene, original, &mut data.tags);
                ObjectKind::polygon(mesh)
            }
            // A disabled generator is a group; an empty one shows nothing.
            None => ObjectKind::Null,
        },
        other => match shown.or_else(|| grid4d_eval::editable_mesh(other).map(MeshHandle::new)) {
            Some(mesh) => ObjectKind::polygon(mesh),
            None => other.clone(),
        },
    };
    data
}

/// A generator shows its cage's Phong tag and material when it has none of its own.
fn take_cage_tags(scene: &Scene, generator: &Node, tags: &mut Vec<Tag>) {
    let Some(cage) = generator.children().first().and_then(|&c| scene.get(c)) else {
        return;
    };
    for kind in [TagKind::Phong, TagKind::Material] {
        if tags.tag_index(kind).is_none()
            && let Some(index) = cage.data.tags.tag_index(kind)
            && let Some(tag) = cage.data.tags.get(index)
        {
            tags.push(tag.clone());
        }
    }
}
