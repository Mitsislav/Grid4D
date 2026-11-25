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
            | (Tool::Slide, Component::Point | Component::Edge)
            | (
                Tool::PlaneCut | Tool::Mirror | Tool::Array,
                Component::Point | Component::Edge | Component::Polygon
            )
            | (Tool::ExtrudeAlongSpline, Component::Polygon)
            | (Tool::Crease, Component::Edge)
    )
}

/// The Offset option of `tool` (what Apply uses and a drag writes back).
pub(crate) fn offset_mut(tool: Tool, settings: &mut ModelingSettings) -> Option<&mut f32> {
    match tool {
        Tool::Extrude => Some(&mut settings.extrude.offset),
        Tool::Inset => Some(&mut settings.inset.offset),
        Tool::Bevel => Some(&mut settings.bevel.offset),
        Tool::Slide => Some(&mut settings.slide.offset),
        Tool::PlaneCut => Some(&mut settings.plane_cut.offset),
        Tool::Crease => Some(&mut settings.crease),
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
        | Tool::Weld
        | Tool::Pen
        | Tool::PolygonPen
        | Tool::ExtrudeAlongSpline
        | Tool::Mirror
        | Tool::Array => None,
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
    // Seams are edges by corner: carried like an edge selection.
    let seams = grid4d_geom::ComponentSet {
        edges: components.state().seams.clone(),
        ..grid4d_geom::ComponentSet::default()
    };
    let hard = grid4d_geom::ComponentSet {
        edges: components.state().hard.clone(),
        ..grid4d_geom::ComponentSet::default()
    };
    let mut state = ComponentState {
        selected: rebuilt.carry(components.selected(), topology),
        hidden: rebuilt.carry(components.hidden(), topology),
        seams: rebuilt.carry(&seams, topology).edges,
        creases: carry_creases(&rebuilt, &components.state().creases, mesh.mesh(), topology),
        hard: rebuilt.carry(&hard, topology).edges,
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

/// `creases` (by corner of `source`) on the rebuilt mesh: an edge whose two
/// ends come from the ends of a creased edge keeps its crease.
pub(crate) fn carry_creases(
    rebuilt: &Rebuilt,
    creases: &std::collections::BTreeMap<u32, u8>,
    source: &grid4d_geom::PolyMesh,
    topology: &grid4d_geom::Topology,
) -> std::collections::BTreeMap<u32, u8> {
    let mut out = std::collections::BTreeMap::new();
    if creases.is_empty() {
        return out;
    }
    // Each creased edge by its points, its largest percent.
    let mut by_edge: std::collections::HashMap<[u32; 2], u8> = std::collections::HashMap::new();
    for (&corner, &percent) in creases {
        let Some(e) = topology.corner_edge(corner as usize) else {
            continue;
        };
        let [a, b] = topology.edges()[e as usize];
        let entry = by_edge.entry([a, b]).or_insert(0);
        *entry = (*entry).max(percent);
    }
    let _ = source;
    let mesh = &rebuilt.mesh;
    let origin = |p: u32| rebuilt.point_origin.get(p as usize).copied().unwrap_or(ops::NEW);
    for f in 0..mesh.face_count() {
        let range = mesh.corner_range(f);
        for c in range.clone() {
            let next = if c + 1 == range.end { range.start } else { c + 1 };
            let (a, b) = (origin(mesh.corner_points()[c]), origin(mesh.corner_points()[next]));
            if a == ops::NEW || b == ops::NEW {
                continue;
            }
            if let Some(&percent) = by_edge.get(&[a.min(b), a.max(b)]) {
                out.insert(c as u32, percent);
            }
        }
    }
    out
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

/// What a tool needs beyond the object: where it stands, and Extrude along
/// Spline's path (in the object's space).
#[derive(Clone, Debug, Default)]
pub(crate) struct Context {
    pub(crate) world: glam::Mat4,
    pub(crate) path: Option<grid4d_geom::spline::Polyline>,
}

impl Context {
    /// The object `id`'s placement in `scene` at `frame`, and the path
    /// `settings` link (moved into the object's space).
    pub(crate) fn of(
        scene: &grid4d_scene::Scene,
        id: grid4d_scene::NodeId,
        settings: &ModelingSettings,
        frame: f64,
    ) -> Self {
        let world = scene.world_matrix_at(id, frame);
        let path = settings.along_spline.and_then(|spline| {
            let mut curve = spline_polyline(scene, spline, frame)?;
            let into = world.inverse() * scene.world_matrix_at(spline, frame);
            for point in &mut curve.points {
                *point = into.transform_point3(*point);
            }
            Some(curve)
        });
        Self { world, path }
    }
}

/// A spline object's curve as it is drawn at `frame` (its own space); `None`
/// for other objects.
pub(crate) fn spline_polyline(
    scene: &grid4d_scene::Scene,
    id: grid4d_scene::NodeId,
    frame: f64,
) -> Option<grid4d_geom::spline::Polyline> {
    let kind = scene.get(id)?.data.kind_at(frame);
    match &*kind {
        ObjectKind::SplinePrimitive(primitive) => Some(primitive.polyline()),
        ObjectKind::Text(text) => Some(grid4d_text::text_spline(text).sample(text.interpolation)),
        ObjectKind::Spline {
            spline, interpolation, ..
        } => Some(spline.spline().sample(*interpolation)),
        ObjectKind::SplineBoolean(_) => grid4d_eval::generator_curve(scene, id, frame),
        _ => None,
    }
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
    context: &Context,
) -> Option<NodeData> {
    let ObjectKind::Polygon { mesh, components } = &data.kind else {
        return None;
    };
    let offset_matters = matches!(tool, Tool::Extrude | Tool::Inset | Tool::Bevel | Tool::Slide);
    if !works_in(tool, component) || (offset_matters && offset == 0.0) {
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
        (Tool::Crease, Component::Edge) => {
            // Every corner of each selected edge gets the crease (0 removes it).
            let edges = selection::edges_from_corners(&state.selected.edges, topology);
            if edges.is_empty() {
                return None;
            }
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "clamped to 0 – 100"
            )]
            let percent = (offset.clamp(0.0, 1.0) * 100.0).round() as u8;
            let mut creases = state.creases.clone();
            for e in edges.iter() {
                for &corner in topology.edge_corners(e) {
                    if percent == 0 {
                        creases.remove(&corner);
                    } else {
                        creases.insert(corner, percent);
                    }
                }
            }
            if creases == state.creases {
                return None;
            }
            let mut data = data.clone();
            if let ObjectKind::Polygon { components, .. } = &mut data.kind {
                components.make_mut().creases = creases;
            }
            return Some(data);
        }
        (Tool::PlaneCut, _) => {
            let cut = settings.plane_cut;
            let (origin, normal) = cut.plane.in_object(offset, context.world);
            let faces = (component == Component::Polygon && cut.selected_only).then_some(&state.selected.polygons);
            let plane = ops::PlaneCut {
                origin,
                normal,
                cuts: cut.cuts,
                spacing: cut.spacing,
                split: cut.split,
            };
            let rebuilt = ops::plane_cut(source, topology, &plane, faces).ok()?;
            return rebuilt_data(data, rebuilt, Fresh::EdgesSelected);
        }
        (Tool::Mirror, _) => {
            let m = settings.mirror;
            let (origin, normal) = m.plane.in_object(0.0, context.world);
            let faces = polygons_in_scope(state, component);
            let options = ops::Mirror {
                origin,
                normal,
                duplicate: m.duplicate,
                weld: m.weld,
                tolerance: m.tolerance,
            };
            let rebuilt = ops::mirror(source, faces, &options).ok()?;
            let fresh = if m.duplicate {
                Fresh::PolygonsSelected
            } else {
                Fresh::Unselected
            };
            return rebuilt_data(data, rebuilt, fresh);
        }
        (Tool::Array, _) => {
            let faces = polygons_in_scope(state, component);
            let rebuilt = ops::array(source, faces, &settings.array).ok()?;
            return rebuilt_data(data, rebuilt, Fresh::PolygonsSelected);
        }
        (Tool::ExtrudeAlongSpline, Component::Polygon) => {
            let path = context.path.as_ref()?;
            let mut faces = state.selected.polygons.clone();
            faces.remove_all(&state.hidden.polygons);
            if faces.is_empty() {
                return None;
            }
            ops::extrude_along(source, topology, &faces, path, &settings.along).ok()?
        }
        (Tool::Slide, Component::Point | Component::Edge) => {
            let rails = slide_rails(source, topology, state, component, settings.slide_direction)?;
            let options = ops::SlideOptions {
                offset,
                ..settings.slide
            };
            if component == Component::Edge && options.duplicate {
                let rebuilt = ops::slide_duplicate(source, topology, &rails, offset).ok()?;
                return rebuilt_data(data, rebuilt, Fresh::EdgesSelected);
            }
            // Points move, nothing else: the topology is shared.
            let points = ops::slide(source, &rails, &options);
            if points == source.points() {
                return None;
            }
            let mut data = data.clone();
            if let ObjectKind::Polygon { mesh: moved, .. } = &mut data.kind {
                *moved = mesh.with_points(&points);
            }
            return Some(data);
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

/// What Mesh ▸ Normals does to hard edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HardOp {
    /// The selected edges become hard.
    MakeHard,
    /// The selected edges stop being hard.
    MakeSoft,
    /// Every edge whose polygons meet at more than the Phong angle becomes
    /// hard (and only those).
    FromAngle,
    /// No edge is hard.
    Clear,
}

impl HardOp {
    pub const ALL: [Self; 4] = [Self::MakeHard, Self::MakeSoft, Self::FromAngle, Self::Clear];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::MakeHard => "Make Hard",
            Self::MakeSoft => "Make Soft",
            Self::FromAngle => "Hard from Angle",
            Self::Clear => "Clear Hard Edges",
        }
    }
}

/// `data` (a polygon object) with its hard edges changed by `op`; `None`
/// if nothing changes.
pub(crate) fn apply_hard(data: &NodeData, op: HardOp) -> Option<NodeData> {
    let ObjectKind::Polygon { mesh, components } = &data.kind else {
        return None;
    };
    let (source, topology) = (mesh.mesh(), mesh.topology());
    let state = components.state();
    let selected = selection::edges_from_corners(&state.selected.edges, topology);
    let corners_of = |edges: &BitSet| {
        let mut corners = BitSet::new();
        for e in edges.iter() {
            for &c in topology.edge_corners(e) {
                corners.insert(c as usize);
            }
        }
        corners
    };
    let mut hard = state.hard.clone();
    match op {
        HardOp::MakeHard => hard.extend(corners_of(&selected).iter()),
        HardOp::MakeSoft => hard.remove_all(&corners_of(&selected)),
        HardOp::Clear => hard.clear(),
        HardOp::FromAngle => {
            use grid4d_scene::Tags;
            let limit = data.tags.smoothing_angle();
            let sharp: BitSet = (0..topology.edge_count())
                .filter(|&e| match topology.edge_faces(e) {
                    &[f, g] => {
                        source
                            .face_normal(f as usize)
                            .angle_between(source.face_normal(g as usize))
                            > limit + 1e-4
                    }
                    _ => false,
                })
                .collect();
            hard = corners_of(&sharp);
        }
    }
    if hard == state.hard {
        return None;
    }
    let mut data = data.clone();
    if let ObjectKind::Polygon { components, .. } = &mut data.kind {
        components.make_mut().hard = hard;
    }
    Some(data)
}

/// The selected polygons in Polygons mode (none: the whole object), visible.
fn polygons_in_scope(state: &ComponentState, component: Component) -> Option<&BitSet> {
    (component == Component::Polygon && !state.selected.polygons.is_empty()).then_some(&state.selected.polygons)
}

/// What the selected points or edges of `state` slide along: single points
/// towards `direction` (object space), edges as loops. `None` when nothing
/// of `component` is selected.
pub(crate) fn slide_rails(
    mesh: &grid4d_geom::PolyMesh,
    topology: &grid4d_geom::Topology,
    state: &ComponentState,
    component: Component,
    direction: glam::Vec3,
) -> Option<Vec<ops::Rail>> {
    let rails = match component {
        Component::Point => {
            let mut points = state.selected.points.clone();
            points.remove_all(&state.hidden.points);
            ops::point_rails(mesh, topology, &points, direction)
        }
        Component::Edge => {
            let edges = selection::edges_from_corners(&state.selected.edges, topology);
            ops::loop_rails(mesh, topology, &edges)
        }
        Component::Polygon => return None,
    };
    (!rails.is_empty()).then_some(rails)
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
        let all = apply(
            &cube,
            Tool::Inset,
            Component::Polygon,
            &settings,
            10.0,
            &Context::default(),
        )
        .unwrap();
        assert_eq!(mesh_of(&all).mesh().face_count(), 6 + 6 * 4, "every face inset");

        let ObjectKind::Polygon { components, .. } = &mut cube.kind else {
            unreachable!()
        };
        components.make_mut().hidden.polygons.insert_range(1..6);
        let one = apply(
            &cube,
            Tool::Inset,
            Component::Polygon,
            &settings,
            10.0,
            &Context::default(),
        )
        .unwrap();
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
            &Context::default(),
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
    fn slide_moves_a_loop_or_inserts_one() {
        // A 4 × 4 plane and its middle row of edges.
        let mesh = grid4d_geom::primitives::plane(100.0, 100.0, 4, 4);
        let topology = grid4d_geom::Topology::build(&mesh);
        let middle: Vec<usize> = (0..topology.edge_count())
            .filter(|&e| {
                let [a, b] = topology.edges()[e];
                mesh.points()[a as usize].z.abs() < 1e-3 && mesh.points()[b as usize].z.abs() < 1e-3
            })
            .collect();
        let corners = selection::corners_from_edges(&middle.iter().copied().collect(), &topology);
        let mut data = Node::new("Plane", ObjectKind::polygon(MeshHandle::new(mesh.clone()))).data;
        let ObjectKind::Polygon { components, .. } = &mut data.kind else {
            unreachable!()
        };
        components.make_mut().selected.edges = corners;
        let mut settings = ModelingSettings::default();
        let slid = apply(&data, Tool::Slide, Component::Edge, &settings, 0.5, &Context::default()).unwrap();
        let moved = mesh_of(&slid).mesh();
        assert_eq!(moved.face_count(), mesh.face_count(), "a slide keeps the topology");
        let row: Vec<f32> = (0..mesh.point_count())
            .filter(|&p| mesh.points()[p].z.abs() < 1e-3)
            .map(|p| moved.points()[p].z)
            .collect();
        assert!(
            row.iter().all(|&z| (z.abs() - 12.5).abs() < 1e-3 && z == row[0]),
            "{row:?}"
        );
        settings.slide.duplicate = true;
        let doubled = apply(&data, Tool::Slide, Component::Edge, &settings, 0.5, &Context::default()).unwrap();
        assert_eq!(
            mesh_of(&doubled).mesh().face_count(),
            mesh.face_count() + 4,
            "a loop inserted"
        );
        assert!(
            apply(
                &data,
                Tool::Slide,
                Component::Polygon,
                &settings,
                0.5,
                &Context::default()
            )
            .is_none()
        );
    }

    #[test]
    fn plane_cut_mirror_array_and_extrude_along_a_spline() {
        let cube = polygon_cube();
        let settings = ModelingSettings::default();
        let here = Context::default();
        // A horizontal cut through the middle: four new points on the plane.
        let cut = apply(&cube, Tool::PlaneCut, Component::Polygon, &settings, 0.0, &here).unwrap();
        assert_eq!(mesh_of(&cut).mesh().point_count(), 8 + 4);
        let ObjectKind::Polygon { components, .. } = &cut.kind else {
            unreachable!()
        };
        assert_eq!(
            components.selected().edges.len(),
            8,
            "the cut's edges selected (by corner)"
        );
        // A world plane on an object moved up by 50: the cut lies at its bottom.
        let mut world_cut = settings;
        world_cut.plane_cut.plane = crate::ToolPlane::WorldXz;
        let moved = Context {
            world: glam::Mat4::from_translation(glam::Vec3::Y * 50.0),
            path: None,
        };
        let low = apply(&cube, Tool::PlaneCut, Component::Polygon, &world_cut, 10.0, &moved).unwrap();
        let new_y: Vec<f32> = mesh_of(&low).mesh().points()[8..].iter().map(|p| p.y).collect();
        assert!(new_y.iter().all(|&y| (y + 40.0).abs() < 1e-3), "{new_y:?}");
        // Mirror: doubled, the image selected; Array: three copies.
        let mirrored = apply(&cube, Tool::Mirror, Component::Polygon, &settings, 0.0, &here).unwrap();
        assert_eq!(mesh_of(&mirrored).mesh().face_count(), 12);
        let ObjectKind::Polygon { components, .. } = &mirrored.kind else {
            unreachable!()
        };
        assert_eq!(components.selected().polygons.len(), 6);
        let copies = apply(&cube, Tool::Array, Component::Polygon, &settings, 0.0, &here).unwrap();
        assert_eq!(mesh_of(&copies).mesh().face_count(), 6 * 4);
        // Extrude along Spline: the selected top along a straight path.
        let mut top = cube;
        let ObjectKind::Polygon { mesh, components } = &mut top.kind else {
            unreachable!()
        };
        let face = (0..mesh.mesh().face_count())
            .find(|&f| mesh.mesh().face_normal(f).y > 0.9)
            .unwrap();
        components.make_mut().selected.polygons.insert(face);
        let path = grid4d_geom::spline::Polyline {
            points: vec![glam::Vec3::ZERO, glam::Vec3::new(0.0, 0.0, 300.0)],
            segments: vec![grid4d_geom::spline::Segment {
                count: 2,
                closed: false,
            }],
        };
        let along = Context {
            world: glam::Mat4::IDENTITY,
            path: Some(path),
        };
        assert!(
            apply(
                &top,
                Tool::ExtrudeAlongSpline,
                Component::Polygon,
                &settings,
                0.0,
                &here
            )
            .is_none(),
            "no spline: nothing"
        );
        let out = apply(
            &top,
            Tool::ExtrudeAlongSpline,
            Component::Polygon,
            &settings,
            0.0,
            &along,
        )
        .unwrap();
        assert_eq!(mesh_of(&out).mesh().face_count(), 6 + 4 * 10);
        let height = mesh_of(&out).mesh().face_center(face).y;
        assert!(height > 300.0, "the end moved up along the path's length: {height}");
    }

    #[test]
    fn hard_edges_and_creases_are_set_and_carried() {
        let cube = polygon_cube();
        let found = apply_hard(&cube, HardOp::FromAngle).unwrap();
        let ObjectKind::Polygon { components, .. } = &found.kind else {
            unreachable!()
        };
        assert_eq!(
            components.state().hard.len(),
            24,
            "a cube's 12 edges, both corners each"
        );
        assert!(
            apply_hard(&found, HardOp::FromAngle).is_none(),
            "again: nothing changes"
        );
        let sphere = grid4d_eval::editable_mesh(&ObjectKindTag::Sphere.default_kind()).unwrap();
        let mut round = Node::new("Sphere", ObjectKind::polygon(MeshHandle::new(sphere))).data;
        round.tags.push(grid4d_scene::Tag::Phong {
            angle_limit: true,
            angle: 40f32.to_radians(),
            weighted: false,
        });
        assert!(
            apply_hard(&round, HardOp::FromAngle).is_none(),
            "a smooth sphere has none"
        );
        let cleared = apply_hard(&found, HardOp::Clear).unwrap();
        let ObjectKind::Polygon { components, .. } = &cleared.kind else {
            unreachable!()
        };
        assert!(components.state().hard.is_empty());
        // Crease the selected edges, then bevel another edge: the crease stays.
        let mut creased = cube;
        let ObjectKind::Polygon { mesh, components } = &mut creased.kind else {
            unreachable!()
        };
        let topology = mesh.topology().clone();
        let edge = topology.edge_corners(0)[0] as usize;
        components.make_mut().selected.edges.insert(edge);
        let settings = ModelingSettings::default();
        let here = Context::default();
        let with = apply(&creased, Tool::Crease, Component::Edge, &settings, 0.6, &here).unwrap();
        let ObjectKind::Polygon { components, .. } = &with.kind else {
            unreachable!()
        };
        assert_eq!(components.state().creases.len(), 2, "both corners of the edge");
        assert!(components.state().creases.values().all(|&p| p == 60));
        let [a, b] = topology.edges()[0];
        let mut other = with.clone();
        let ObjectKind::Polygon { components, .. } = &mut other.kind else {
            unreachable!()
        };
        let far = (0..topology.edge_count())
            .find(|&e| !topology.edges()[e].contains(&a) && !topology.edges()[e].contains(&b))
            .unwrap();
        components.make_mut().selected.edges = topology.edge_corners(far).iter().map(|&c| c as usize).collect();
        let bevelled = apply(&other, Tool::Bevel, Component::Edge, &settings, 10.0, &here).unwrap();
        let ObjectKind::Polygon { mesh, components } = &bevelled.kind else {
            unreachable!()
        };
        assert_eq!(components.state().creases.len(), 2, "carried across the bevel");
        let corner = *components.state().creases.keys().next().unwrap() as usize;
        let e = mesh.topology().corner_edge(corner).unwrap() as usize;
        let ends = mesh.topology().edges()[e].map(|p| mesh.mesh().points()[p as usize]);
        let original = [a, b].map(|p| polygon_cube_points()[p as usize]);
        assert!(ends.iter().all(|p| original.contains(p)), "on the same edge");
    }

    fn polygon_cube_points() -> Vec<glam::Vec3> {
        mesh_of(&polygon_cube()).mesh().points().to_vec()
    }

    #[test]
    fn tools_apply_only_where_they_work() {
        let cube = polygon_cube();
        let settings = ModelingSettings::default();
        assert!(
            apply(
                &cube,
                Tool::Extrude,
                Component::Point,
                &settings,
                10.0,
                &Context::default()
            )
            .is_none(),
            "points: later"
        );
        assert!(
            apply(
                &cube,
                Tool::Inset,
                Component::Edge,
                &settings,
                10.0,
                &Context::default()
            )
            .is_none()
        );
        assert!(
            apply(
                &cube,
                Tool::Extrude,
                Component::Polygon,
                &settings,
                0.0,
                &Context::default()
            )
            .is_none(),
            "offset 0"
        );
        assert!(
            apply(
                &cube,
                Tool::Extrude,
                Component::Edge,
                &settings,
                10.0,
                &Context::default()
            )
            .is_none(),
            "a closed cube has no border edge"
        );
        let mut plain = cube;
        plain.kind = ObjectKindTag::Cube.default_kind();
        assert!(
            apply(
                &plain,
                Tool::Extrude,
                Component::Polygon,
                &settings,
                10.0,
                &Context::default()
            )
            .is_none()
        );
    }
}
