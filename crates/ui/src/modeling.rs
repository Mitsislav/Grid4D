//! The modelling tools' operation on a polygon object (Mesh ▸ Extrude, Inset),
//! shared by the viewport drag (re-run from the mesh at the press every frame)
//! and the Attribute Manager's Apply. The algorithms are `grid4d_geom::ops`.

use grid4d_geom::{
    BitSet, Component,
    loops::{self, WalkOptions},
    ops::{self, EdgeExtrudeOptions, ExtrudeOptions, InsetOptions, Rebuilt},
    selection,
};
use grid4d_scene::{ComponentState, Components, MeshHandle, NodeData, ObjectKind};

use crate::{ModelingSettings, Tool};

/// `tool` works in the component mode of `component` (Inset: polygons only;
/// Extrude: polygons and edges — points later).
pub(crate) const fn works_in(tool: Tool, component: Component) -> bool {
    matches!(
        (tool, component),
        (Tool::Extrude, Component::Polygon | Component::Edge)
            | (Tool::Inset, Component::Polygon)
            | (Tool::Bevel, Component::Point | Component::Edge)
    )
}

/// The Offset option of `tool` (what Apply uses and a drag writes back).
pub(crate) fn offset_mut(tool: Tool, settings: &mut ModelingSettings) -> Option<&mut f32> {
    match tool {
        Tool::Extrude => Some(&mut settings.extrude.offset),
        Tool::Inset => Some(&mut settings.inset.offset),
        Tool::Bevel => Some(&mut settings.bevel.offset),
        Tool::LiveSelection
        | Tool::RectangleSelection
        | Tool::LoopSelection
        | Tool::RingSelection
        | Tool::Move
        | Tool::Scale
        | Tool::Rotate
        | Tool::LoopCut
        | Tool::LineCut
        | Tool::Bridge
        | Tool::Weld => None,
    }
}

/// What the new parts of a rebuilt mesh become in the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fresh {
    Unselected,
    /// The operation's new edges replace the edge selection.
    EdgesSelected,
    /// The new polygons replace the polygon selection.
    PolygonsSelected,
}

impl Fresh {
    pub(crate) const fn edges_if(select: bool) -> Self {
        if select { Self::EdgesSelected } else { Self::Unselected }
    }
}

/// `data` (a polygon object) with the mesh `rebuilt` from it: its selection and
/// hidden flags carried across, the new parts selected as `fresh` says. `None` if
/// the mesh did not change.
pub(crate) fn rebuilt_data(data: &NodeData, rebuilt: Rebuilt, fresh: Fresh) -> Option<NodeData> {
    let ObjectKind::Polygon { mesh, components } = &data.kind else {
        return None;
    };
    if rebuilt.mesh == *mesh.mesh() {
        return None;
    }
    let topology = mesh.topology();
    let mut state = ComponentState {
        selected: rebuilt.carry(components.selected(), topology),
        hidden: rebuilt.carry(components.hidden(), topology),
    };
    match fresh {
        Fresh::EdgesSelected if !rebuilt.new_edges.is_empty() => state.selected.edges = rebuilt.new_edge_corners(),
        Fresh::PolygonsSelected => {
            let new: BitSet = (0..rebuilt.mesh.face_count())
                .filter(|&f| rebuilt.face_origin[f] == ops::NEW)
                .collect();
            if !new.is_empty() {
                state.selected.polygons = new;
            }
        }
        Fresh::EdgesSelected | Fresh::Unselected => {}
    }
    let mut data = data.clone();
    data.kind = ObjectKind::Polygon {
        mesh: MeshHandle::new(rebuilt.mesh),
        components: Components::new(state),
    };
    Some(data)
}

/// A Loop/Path Cut of `mesh` across unique edge `edge` at `fractions` (the ring
/// runs through quads only).
pub(crate) fn loop_cut(mesh: &MeshHandle, edge: usize, fractions: &[f32]) -> Option<Rebuilt> {
    let (source, topology) = (mesh.mesh(), mesh.topology());
    let options = WalkOptions {
        stop_at_non_quads: true,
        ..WalkOptions::default()
    };
    let ring = loops::edge_ring(source, topology, edge, options, &BitSet::new(), &BitSet::new());
    ops::loop_cut(source, topology, &ring, edge, fractions).ok()
}

/// `data` (a polygon object) with `tool` applied at `offset` to its selected
/// components of kind `component` — to every visible one when none is selected.
/// `None` if the tool does not apply here or would change nothing.
pub(crate) fn apply(
    data: &NodeData,
    tool: Tool,
    component: Component,
    settings: &ModelingSettings,
    offset: f32,
) -> Option<NodeData> {
    let ObjectKind::Polygon { mesh, components } = &data.kind else {
        return None;
    };
    if !works_in(tool, component) || offset == 0.0 {
        return None;
    }
    let (source, topology) = (mesh.mesh(), mesh.topology());
    let state = components.state();
    let mut components = components.clone();
    let result = match (tool, component) {
        (Tool::Extrude, Component::Polygon) => {
            let faces = scope(&state.selected.polygons, &state.hidden.polygons, source.face_count());
            let options = ExtrudeOptions {
                offset,
                ..settings.extrude
            };
            ops::extrude_polygons(source, topology, &faces, &options).ok()?
        }
        (Tool::Inset, Component::Polygon) => {
            let faces = scope(&state.selected.polygons, &state.hidden.polygons, source.face_count());
            let options = InsetOptions {
                offset,
                ..settings.inset
            };
            ops::inset(source, topology, &faces, &options).ok()?
        }
        (Tool::Extrude, Component::Edge) => {
            let selected = selection::edges_from_corners(&state.selected.edges, topology);
            let hidden = selection::edges_from_corners(&state.hidden.edges, topology);
            let edges = scope(&selected, &hidden, topology.edge_count());
            let options = EdgeExtrudeOptions {
                offset,
                rotation: settings.edge_rotation,
                subdivision: settings.extrude.subdivision,
                create_ngons: settings.extrude.create_ngons,
            };
            let extrusion = ops::extrude_edges(source, topology, &edges, &options).ok()?;
            // The new outer edges take over the selection, so the next drag continues.
            let selected = &mut components.make_mut().selected.edges;
            selected.clear();
            for corner in extrusion.outer_corners {
                selected.insert(usize::try_from(corner).ok()?);
            }
            extrusion.mesh
        }
        (Tool::Bevel, Component::Edge | Component::Point) => {
            let edges = if component == Component::Edge {
                selection::edges_from_corners(&state.selected.edges, topology)
            } else {
                BitSet::new()
            };
            let points = if component == Component::Point {
                state.selected.points.clone()
            } else {
                BitSet::new()
            };
            if edges.is_empty() && points.is_empty() {
                return None; // nothing selected: nothing to bevel
            }
            let options = ops::BevelOptions {
                offset,
                ..settings.bevel
            };
            let rebuilt = ops::bevel(source, topology, &edges, &points, &options).ok()?;
            return rebuilt_data(data, rebuilt, Fresh::edges_if(component == Component::Edge));
        }
        _ => return None,
    };
    if result == *source {
        return None; // e.g. no border edge among the selected ones
    }
    let mut data = data.clone();
    data.kind = ObjectKind::Polygon {
        mesh: MeshHandle::new(result),
        components,
    };
    Some(data)
}

/// The selected members of `0..len` that are not hidden; every visible one when
/// none is selected.
fn scope(selected: &BitSet, hidden: &BitSet, len: usize) -> BitSet {
    let mut set = selected.clone();
    if set.is_empty() {
        set.insert_range(0..len);
    }
    set.remove_all(hidden);
    set
}

#[cfg(test)]
mod tests {
    use grid4d_scene::{Node, ObjectKindTag};

    use super::*;

    fn polygon_cube() -> NodeData {
        let mesh = grid4d_eval::editable_mesh(&ObjectKindTag::Cube.default_kind()).unwrap();
        Node::new("Cube", ObjectKind::polygon(MeshHandle::new(mesh))).data
    }

    fn mesh_of(data: &NodeData) -> &MeshHandle {
        let ObjectKind::Polygon { mesh, .. } = &data.kind else {
            panic!("not a polygon object")
        };
        mesh
    }

    #[test]
    fn nothing_selected_means_every_visible_polygon() {
        let mut cube = polygon_cube();
        let settings = ModelingSettings::default();
        let all = apply(&cube, Tool::Inset, Component::Polygon, &settings, 10.0).unwrap();
        assert_eq!(mesh_of(&all).mesh().face_count(), 6 + 6 * 4, "every face inset");

        let ObjectKind::Polygon { components, .. } = &mut cube.kind else {
            unreachable!()
        };
        components.make_mut().hidden.polygons.insert_range(1..6);
        let one = apply(&cube, Tool::Inset, Component::Polygon, &settings, 10.0).unwrap();
        assert_eq!(
            mesh_of(&one).mesh().face_count(),
            6 + 4,
            "hidden polygons are left alone"
        );
    }

    #[test]
    fn an_edge_extrusion_selects_the_new_outer_edge() {
        let mesh = grid4d_eval::editable_mesh(&ObjectKindTag::Plane.default_kind()).unwrap();
        let topology = grid4d_geom::Topology::build(&mesh);
        let (faces, corners) = (mesh.face_count(), mesh.corner_count());
        let border = (0..topology.edge_count())
            .find(|&e| topology.edge_faces(e).len() == 1)
            .unwrap();
        let corner = usize::try_from(topology.edge_corners(border)[0]).unwrap();
        let mut data = Node::new("Plane", ObjectKind::polygon(MeshHandle::new(mesh))).data;
        let ObjectKind::Polygon { components, .. } = &mut data.kind else {
            unreachable!()
        };
        components.make_mut().selected.edges.insert(corner);
        let out = apply(
            &data,
            Tool::Extrude,
            Component::Edge,
            &ModelingSettings::default(),
            10.0,
        )
        .unwrap();
        let ObjectKind::Polygon { mesh, components } = &out.kind else {
            unreachable!()
        };
        assert_eq!(mesh.mesh().face_count(), faces + 1);
        let selected: Vec<usize> = components.selected().edges.iter().collect();
        assert_eq!(selected.len(), 1);
        assert!(selected[0] >= corners, "a corner of the new polygon, not the old edge");
    }

    #[test]
    fn tools_apply_only_where_they_work() {
        let cube = polygon_cube();
        let settings = ModelingSettings::default();
        assert!(
            apply(&cube, Tool::Extrude, Component::Point, &settings, 10.0).is_none(),
            "points: later"
        );
        assert!(apply(&cube, Tool::Inset, Component::Edge, &settings, 10.0).is_none());
        assert!(
            apply(&cube, Tool::Extrude, Component::Polygon, &settings, 0.0).is_none(),
            "offset 0"
        );
        assert!(
            apply(&cube, Tool::Extrude, Component::Edge, &settings, 10.0).is_none(),
            "a closed cube has no border edge"
        );
        let mut plain = cube;
        plain.kind = ObjectKindTag::Cube.default_kind();
        assert!(apply(&plain, Tool::Extrude, Component::Polygon, &settings, 10.0).is_none());
    }
}
