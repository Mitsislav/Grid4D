//! Component modes in the viewport: picking points, edges and polygons of the
//! active polygon object, and the overlay that shows them.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec2, Vec3, Vec4};
use grid4d_eval::EvaluatedScene;
use grid4d_geom::{BitSet, Component, Triangulator, selection::edges_from_corners};
use grid4d_scene::{ComponentState, Components, MeshHandle, MeshId, NodeId};

/// A picked component of a polygon object: a point index, a unique edge index
/// (into its topology's edge list) or a polygon index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ComponentHit {
    Point(u32),
    Edge(u32),
    Polygon(u32),
}

impl ComponentHit {
    #[must_use]
    pub const fn component(self) -> Component {
        match self {
            Self::Point(_) => Component::Point,
            Self::Edge(_) => Component::Edge,
            Self::Polygon(_) => Component::Polygon,
        }
    }

    #[must_use]
    pub const fn index(self) -> u32 {
        match self {
            Self::Point(i) | Self::Edge(i) | Self::Polygon(i) => i,
        }
    }
}

/// How the viewport maps the world to the screen, for screen-space picking.
/// `size` and the cursor passed to [`pick_component`] share one unit (points or
/// pixels) and origin (the view's top-left corner).
#[derive(Clone, Copy, Debug)]
pub struct ScreenView {
    pub view_proj: Mat4,
    pub eye: Vec3,
    pub size: Vec2,
}

impl ScreenView {
    /// Screen position of a clip-space point (`w > 0`).
    fn to_screen(self, clip: Vec4) -> Vec2 {
        let ndc = clip.truncate().truncate() / clip.w;
        Vec2::new((ndc.x * 0.5 + 0.5) * self.size.x, (0.5 - ndc.y * 0.5) * self.size.y)
    }
}

/// `u32` index → `usize` (lossless on every supported platform).
fn idx(index: u32) -> usize {
    usize::try_from(index).unwrap_or(usize::MAX)
}

/// Ray parameters closer than this to the target (along eye → target) still
/// count as reaching it: the component's own faces must not hide it.
const VISIBLE_TOLERANCE: f32 = 1e-3;

/// The visible component of kind `component` of polygon object `node` under
/// `cursor`: the nearest point or edge within `radius` (screen distance) that no
/// face hides, or the polygon under the cursor. Hidden components are skipped and
/// hidden polygons hide nothing. `None` if nothing qualifies (or `node` is not a
/// visible polygon object).
///
/// Runs on clicks and while hovering, not every frame: it projects every point
/// (or edge) of the mesh, then ray-casts the few candidates near the cursor.
#[must_use]
pub fn pick_component(
    scene: &EvaluatedScene,
    node: NodeId,
    component: Component,
    view: &ScreenView,
    cursor: Vec2,
    radius: f32,
) -> Option<ComponentHit> {
    let (evaluated, handle, components) = scene.polygon(node)?;
    let world = evaluated.world;
    if world.determinant().abs() <= f32::EPSILON {
        return None;
    }
    let to_object = world.inverse();
    let eye = to_object.transform_point3(view.eye);
    let hidden = Hidden::new(handle, components.state());
    let mesh = handle.mesh();
    let visible = |target: Vec3| {
        handle
            .raycast_where(eye, target - eye, |f| !hidden.polygons.contains(idx(f)))
            .is_none_or(|hit| hit.t >= 1.0 - VISIBLE_TOLERANCE)
    };
    let clip_of = |p: Vec3| view.view_proj * world.transform_point3(p).extend(1.0);

    match component {
        Component::Polygon => {
            let ndc = Vec2::new(cursor.x / view.size.x * 2.0 - 1.0, 1.0 - cursor.y / view.size.y * 2.0);
            let inverse = view.view_proj.inverse();
            let far = inverse.project_point3(ndc.extend(1.0));
            let near = inverse.project_point3(ndc.extend(0.0));
            let (origin, dir) = (
                to_object.transform_point3(near),
                to_object.transform_vector3(far - near),
            );
            handle
                .raycast_where(origin, dir, |f| !hidden.polygons.contains(idx(f)))
                .map(|hit| ComponentHit::Polygon(hit.face))
        }
        Component::Point => {
            let mut candidates: Vec<(f32, u32)> = mesh
                .points()
                .iter()
                .enumerate()
                .filter(|&(p, _)| !hidden.points.contains(p))
                .filter_map(|(p, &position)| {
                    let clip = clip_of(position);
                    (clip.w > 1e-6).then(|| (view.to_screen(clip).distance(cursor), u32::try_from(p).ok()))
                })
                .filter_map(|(d, p)| Some((d, p?)))
                .filter(|&(d, _)| d <= radius)
                .collect();
            candidates.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
            candidates
                .into_iter()
                .find(|&(_, p)| visible(mesh.points()[idx(p)]))
                .map(|(_, p)| ComponentHit::Point(p))
        }
        Component::Edge => {
            let topology = handle.topology();
            let mut candidates: Vec<(f32, u32, Vec3)> = topology
                .edges()
                .iter()
                .enumerate()
                .filter(|&(e, _)| !hidden.edges.contains(e))
                .filter_map(|(e, &[a, b])| {
                    let (a, b) = (mesh.points()[idx(a)], mesh.points()[idx(b)]);
                    let (d, closest) = edge_distance(view, clip_of(a), clip_of(b), a, b, cursor)?;
                    Some((d, u32::try_from(e).ok()?, closest))
                })
                .filter(|&(d, _, _)| d <= radius)
                .collect();
            candidates.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
            candidates
                .into_iter()
                .find(|&(_, _, closest)| visible(closest))
                .map(|(_, e, _)| ComponentHit::Edge(e))
        }
    }
}

/// Screen distance from `cursor` to the edge `a`–`b` (object space, with clip
/// coordinates `ca`/`cb`), clipped to the near plane, and the edge's point closest
/// to the cursor in object space. `None` if the edge is behind the camera.
fn edge_distance(
    view: &ScreenView,
    mut ca: Vec4,
    mut cb: Vec4,
    mut a: Vec3,
    mut b: Vec3,
    cursor: Vec2,
) -> Option<(f32, Vec3)> {
    // Near plane: z = 0 in clip space (depth 0…1). The map is affine in the
    // homogeneous point, so the same parameter clips the object-space ends.
    if ca.z < 0.0 && cb.z < 0.0 {
        return None;
    }
    if ca.z < 0.0 {
        let t = ca.z / (ca.z - cb.z);
        (ca, a) = (ca.lerp(cb, t), a.lerp(b, t));
    } else if cb.z < 0.0 {
        let t = cb.z / (cb.z - ca.z);
        (cb, b) = (cb.lerp(ca, t), b.lerp(a, t));
    }
    if ca.w <= 1e-6 || cb.w <= 1e-6 {
        return None;
    }
    let (sa, sb) = (view.to_screen(ca), view.to_screen(cb));
    let span = sb - sa;
    let s = if span.length_squared() > 0.0 {
        ((cursor - sa).dot(span) / span.length_squared()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let distance = (sa + span * s).distance(cursor);
    // Screen parameter → object parameter (perspective-correct).
    let (ia, ib) = ((1.0 - s) / ca.w, s / cb.w);
    let t = ib / (ia + ib);
    Some((distance, a.lerp(b, t)))
}

/// What is hidden, per kind: flagged components, plus points and edges whose
/// polygons are all hidden (hiding faces hides their points).
struct Hidden {
    points: BitSet,
    /// By unique edge index.
    edges: BitSet,
    polygons: BitSet,
}

impl Hidden {
    fn new(handle: &MeshHandle, state: &ComponentState) -> Self {
        let flags = &state.hidden;
        let topology = handle.topology();
        let mut points = flags.points.clone();
        let mut edges = edges_from_corners(&flags.edges, topology);
        if !flags.polygons.is_empty() {
            let all_hidden =
                |faces: &[u32]| !faces.is_empty() && faces.iter().all(|&f| flags.polygons.contains(idx(f)));
            points.extend((0..handle.mesh().point_count()).filter(|&p| all_hidden(topology.point_faces(p))));
            edges.extend((0..topology.edge_count()).filter(|&e| all_hidden(topology.edge_faces(e))));
        }
        Self {
            points,
            edges,
            polygons: flags.polygons.clone(),
        }
    }
}

// ── Overlay ──────────────────────────────────────────────────────────────────

/// Instance flag: the component is selected.
pub const SELECTED: u32 = 1;
/// Instance flag: the cursor is over the component (pre-selection).
pub const HOVERED: u32 = 2;
/// Instance flag: an edge drawn for context in Points or Polygons mode.
pub const CONTEXT: u32 = 4;

/// One point square. Layout must match `PointIn` in `shaders/overlay.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct PointInstance {
    /// World space.
    pub position: [f32; 3],
    pub flags: u32,
}

/// One edge line. Layout must match `EdgeIn` in `shaders/overlay.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct EdgeInstance {
    /// World space.
    pub a: [f32; 3],
    pub flags: u32,
    /// World space.
    pub b: [f32; 3],
    pub pad: u32,
}

/// One vertex of a tinted polygon's triangles. Layout must match `FaceIn`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct FaceVertex {
    /// World space.
    pub position: [f32; 3],
    pub flags: u32,
}

/// Colours (linear RGBA) and sizes (physical pixels) of the component overlay;
/// the UI fills it from its theme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComponentStyle {
    pub wire: [f32; 4],
    pub selected: [f32; 4],
    pub hover: [f32; 4],
    pub point_size: f32,
    pub edge_width: f32,
    pub selected_edge_width: f32,
    /// Opacity of the tint on selected polygons.
    pub face_alpha: f32,
    /// Opacity of the tint on the polygon under the cursor.
    pub hover_face_alpha: f32,
    /// Opacity of context edges (Points and Polygons modes).
    pub context_alpha: f32,
}

impl Default for ComponentStyle {
    fn default() -> Self {
        Self {
            wire: [0.08, 0.27, 0.89, 1.0],
            selected: [0.91, 0.27, 0.02, 1.0],
            hover: [0.85, 0.87, 0.89, 1.0],
            point_size: 6.0,
            edge_width: 1.5,
            selected_edge_width: 3.0,
            face_alpha: 0.4,
            hover_face_alpha: 0.2,
            context_alpha: 0.5,
        }
    }
}

/// What the main overlay was built from: rebuilt only when one of these changes.
#[derive(Debug)]
struct Built {
    node: NodeId,
    mesh: MeshId,
    components: Components,
    world: Mat4,
    component: Component,
}

/// CPU side of the component overlay, owned by [`FrameData`](crate::FrameData).
///
/// The main arrays (every point/edge, selected polygons) are rebuilt only when
/// the mesh, its selection, its placement or the mode changes; the hover arrays
/// (one component) whenever the cursor moves to another component. Idle frames
/// compare a few ids and allocate nothing.
#[derive(Debug, Default)]
pub(crate) struct ComponentOverlay {
    pub(crate) points: Vec<PointInstance>,
    pub(crate) edges: Vec<EdgeInstance>,
    pub(crate) faces: Vec<FaceVertex>,
    pub(crate) hover_points: Vec<PointInstance>,
    pub(crate) hover_edges: Vec<EdgeInstance>,
    pub(crate) hover_faces: Vec<FaceVertex>,
    /// Bumped whenever the main arrays change (the renderer re-uploads them).
    pub(crate) generation: u64,
    /// Bumped whenever the hover arrays change.
    pub(crate) hover_generation: u64,
    built: Option<Built>,
    hover: Option<ComponentHit>,
    triangulator: Triangulator,
    triangles: Vec<[u32; 3]>,
}

impl ComponentOverlay {
    pub(crate) fn update(
        &mut self,
        scene: &EvaluatedScene,
        target: Option<(NodeId, Component)>,
        hover: Option<ComponentHit>,
    ) {
        let Some((node, component)) = target else {
            self.clear();
            return;
        };
        let Some((evaluated, handle, components)) = scene.polygon(node) else {
            self.clear();
            return;
        };
        let world = evaluated.world;
        let unchanged = self.built.as_ref().is_some_and(|b| {
            b.node == node
                && b.mesh == handle.id()
                && b.components.same(components)
                && b.world == world
                && b.component == component
        });
        if !unchanged {
            self.rebuild(handle, components.state(), world, component);
            self.built = Some(Built {
                node,
                mesh: handle.id(),
                components: components.clone(),
                world,
                component,
            });
            self.generation += 1;
            self.hover = None; // the hover arrays may refer to the old mesh
            self.set_hover(handle, world, None);
        }
        let hover = hover.filter(|h| h.component() == component);
        if hover != self.hover {
            self.set_hover(handle, world, hover);
        }
    }

    fn clear(&mut self) {
        if self.built.is_none() && self.hover.is_none() {
            return;
        }
        for v in [&mut self.points, &mut self.hover_points] {
            v.clear();
        }
        for v in [&mut self.edges, &mut self.hover_edges] {
            v.clear();
        }
        for v in [&mut self.faces, &mut self.hover_faces] {
            v.clear();
        }
        self.built = None;
        self.hover = None;
        self.generation += 1;
        self.hover_generation += 1;
    }

    fn rebuild(&mut self, handle: &MeshHandle, state: &ComponentState, world: Mat4, component: Component) {
        let (mesh, topology) = (handle.mesh(), handle.topology());
        let hidden = Hidden::new(handle, state);
        let selected = &state.selected;
        let at = |p: u32| world.transform_point3(mesh.points()[idx(p)]).to_array();

        self.points.clear();
        if component == Component::Point {
            self.points.extend(
                (0..mesh.point_count())
                    .filter(|&p| !hidden.points.contains(p))
                    .filter_map(|p| u32::try_from(p).ok())
                    .map(|p| PointInstance {
                        position: at(p),
                        flags: if selected.points.contains(idx(p)) { SELECTED } else { 0 },
                    }),
            );
        }

        self.edges.clear();
        let selected_edges = edges_from_corners(&selected.edges, topology);
        self.edges.extend(
            topology
                .edges()
                .iter()
                .enumerate()
                .filter(|&(e, _)| !hidden.edges.contains(e))
                .map(|(e, &[a, b])| EdgeInstance {
                    a: at(a),
                    b: at(b),
                    flags: match component {
                        Component::Edge if selected_edges.contains(e) => SELECTED,
                        Component::Edge => 0,
                        Component::Point | Component::Polygon => CONTEXT,
                    },
                    pad: 0,
                }),
        );

        self.faces.clear();
        if component == Component::Polygon {
            for f in selected
                .polygons
                .iter()
                .filter(|&f| f < mesh.face_count() && !hidden.polygons.contains(f))
            {
                self.triangles.clear();
                self.triangulator.face_triangles(mesh, f, &mut self.triangles);
                for &[a, b, c] in &self.triangles {
                    for p in [a, b, c] {
                        self.faces.push(FaceVertex {
                            position: at(p),
                            flags: SELECTED,
                        });
                    }
                }
            }
        }
    }

    fn set_hover(&mut self, handle: &MeshHandle, world: Mat4, hover: Option<ComponentHit>) {
        self.hover_points.clear();
        self.hover_edges.clear();
        self.hover_faces.clear();
        let mesh = handle.mesh();
        let at = |p: u32| mesh.points().get(idx(p)).map(|&q| world.transform_point3(q).to_array());
        match hover {
            None => {}
            Some(ComponentHit::Point(p)) => {
                if let Some(position) = at(p) {
                    self.hover_points.push(PointInstance {
                        position,
                        flags: HOVERED,
                    });
                }
            }
            Some(ComponentHit::Edge(e)) => {
                if let Some(&[a, b]) = handle.topology().edges().get(idx(e))
                    && let (Some(a), Some(b)) = (at(a), at(b))
                {
                    self.hover_edges.push(EdgeInstance {
                        a,
                        b,
                        flags: HOVERED,
                        pad: 0,
                    });
                }
            }
            Some(ComponentHit::Polygon(f)) if idx(f) < mesh.face_count() => {
                self.triangles.clear();
                self.triangulator.face_triangles(mesh, idx(f), &mut self.triangles);
                for &[a, b, c] in &self.triangles {
                    for p in [a, b, c] {
                        if let Some(position) = at(p) {
                            self.hover_faces.push(FaceVertex {
                                position,
                                flags: HOVERED,
                            });
                        }
                    }
                }
            }
            Some(ComponentHit::Polygon(_)) => {}
        }
        self.hover = hover;
        self.hover_generation += 1;
    }
}

#[cfg(test)]
mod tests {
    use grid4d_scene::{ComponentState, Document, Node, ObjectKind, ObjectKindTag};

    use super::*;
    use crate::OrbitCamera;

    /// A 200 cm polygon cube at the origin, seen by the default camera.
    fn scene_with_cube(state: ComponentState) -> (Document, NodeId, EvaluatedScene) {
        let mesh = grid4d_eval::editable_mesh(&ObjectKindTag::Cube.default_kind()).unwrap();
        let mut doc = Document::new("components");
        let kind = ObjectKind::Polygon {
            mesh: MeshHandle::new(mesh),
            components: Components::new(state),
        };
        let id = doc.scene.insert(Node::new("Cube", kind));
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        (doc, id, eval)
    }

    fn view() -> ScreenView {
        let camera = OrbitCamera::default();
        ScreenView {
            view_proj: camera.view_projection(1.5),
            eye: camera.eye(),
            size: Vec2::new(900.0, 600.0),
        }
    }

    fn screen(view: &ScreenView, world: Vec3) -> Vec2 {
        view.to_screen(view.view_proj * world.extend(1.0))
    }

    fn mesh_of(eval: &EvaluatedScene, id: NodeId) -> MeshHandle {
        eval.polygon(id).unwrap().1.clone()
    }

    #[test]
    fn the_nearest_visible_point_is_picked_and_hidden_ones_are_not() {
        let (_, id, eval) = scene_with_cube(ComponentState::default());
        let view = view();
        let mesh = mesh_of(&eval, id);
        // The cube corner nearest to the camera is visible; the farthest is behind the cube.
        let by_distance = |far: bool| {
            let points = mesh.mesh().points();
            let mut order: Vec<usize> = (0..points.len()).collect();
            order.sort_by(|&a, &b| points[a].distance(view.eye).total_cmp(&points[b].distance(view.eye)));
            if far { order[points.len() - 1] } else { order[0] }
        };
        let near = by_distance(false);
        let cursor = screen(&view, mesh.mesh().points()[near]) + Vec2::new(3.0, -2.0);
        let hit = pick_component(&eval, id, Component::Point, &view, cursor, 8.0);
        assert_eq!(hit, Some(ComponentHit::Point(u32::try_from(near).unwrap())));

        let far = by_distance(true);
        let cursor = screen(&view, mesh.mesh().points()[far]);
        let hit = pick_component(&eval, id, Component::Point, &view, cursor, 4.0);
        assert_ne!(
            hit,
            Some(ComponentHit::Point(u32::try_from(far).unwrap())),
            "behind the cube"
        );

        let mut state = ComponentState::default();
        state.hidden.points.insert(near);
        let (_, id, eval) = scene_with_cube(state);
        let cursor = screen(&view, mesh.mesh().points()[near]);
        assert_ne!(
            pick_component(&eval, id, Component::Point, &view, cursor, 4.0),
            Some(ComponentHit::Point(u32::try_from(near).unwrap())),
            "hidden points are not pickable"
        );
    }

    #[test]
    fn polygons_and_edges_under_the_cursor() {
        let (_, id, eval) = scene_with_cube(ComponentState::default());
        let view = view();
        let mesh = mesh_of(&eval, id);
        // The +Y face's centre: the default camera looks down at the cube.
        let top = (0..mesh.mesh().face_count())
            .find(|&f| mesh.mesh().face_normal(f).abs_diff_eq(Vec3::Y, 1e-5))
            .unwrap();
        let cursor = screen(&view, mesh.mesh().face_center(top));
        let hit = pick_component(&eval, id, Component::Polygon, &view, cursor, 8.0);
        assert_eq!(hit, Some(ComponentHit::Polygon(u32::try_from(top).unwrap())));
        assert_eq!(
            pick_component(&eval, id, Component::Polygon, &view, Vec2::new(5.0, 5.0), 8.0),
            None,
            "empty space"
        );

        // The middle of an edge of the top face.
        let face = mesh.mesh().face(top);
        let (a, b) = (mesh.mesh().points()[idx(face[0])], mesh.mesh().points()[idx(face[1])]);
        let cursor = screen(&view, (a + b) * 0.5) + Vec2::new(1.0, 1.0);
        let Some(ComponentHit::Edge(e)) = pick_component(&eval, id, Component::Edge, &view, cursor, 8.0) else {
            panic!("no edge picked");
        };
        let [p, q] = mesh.topology().edges()[idx(e)];
        let mut ends = [p, q];
        ends.sort_unstable();
        let mut expected = [face[0], face[1]];
        expected.sort_unstable();
        assert_eq!(ends, expected);
    }

    #[test]
    fn the_overlay_rebuilds_only_when_something_changed() {
        let mut state = ComponentState::default();
        state.selected.points.insert(2);
        let (_, id, eval) = scene_with_cube(state);
        let mut overlay = ComponentOverlay::default();
        overlay.update(&eval, Some((id, Component::Point)), None);
        assert_eq!(overlay.points.len(), 8);
        assert_eq!(overlay.points.iter().filter(|p| p.flags == SELECTED).count(), 1);
        assert_eq!(overlay.edges.len(), 12, "context edges");
        assert!(overlay.edges.iter().all(|e| e.flags == CONTEXT));
        let generation = overlay.generation;
        let capacity = (overlay.points.capacity(), overlay.edges.capacity());

        for _ in 0..3 {
            overlay.update(&eval, Some((id, Component::Point)), None);
        }
        assert_eq!(overlay.generation, generation, "idle frames rebuild nothing");

        overlay.update(&eval, Some((id, Component::Point)), Some(ComponentHit::Point(5)));
        assert_eq!(
            overlay.generation, generation,
            "hovering does not touch the main arrays"
        );
        assert_eq!(overlay.hover_points.len(), 1);
        assert_eq!((overlay.points.capacity(), overlay.edges.capacity()), capacity);

        overlay.update(&eval, Some((id, Component::Polygon)), None);
        assert!(overlay.generation > generation, "another mode rebuilds");
        assert!(
            overlay.points.is_empty() && overlay.faces.is_empty(),
            "nothing selected in Polygons"
        );
        overlay.update(&eval, None, None);
        assert!(overlay.edges.is_empty() && overlay.hover_points.is_empty());
    }

    #[test]
    fn selected_polygons_are_tinted_and_edges_highlighted() {
        let mut state = ComponentState::default();
        state.selected.polygons.insert(0);
        state.selected.edges.insert(0);
        let (_, id, eval) = scene_with_cube(state);
        let mut overlay = ComponentOverlay::default();
        overlay.update(&eval, Some((id, Component::Polygon)), None);
        assert_eq!(overlay.faces.len(), 6, "one quad = two triangles");
        overlay.update(&eval, Some((id, Component::Edge)), None);
        assert_eq!(overlay.edges.iter().filter(|e| e.flags == SELECTED).count(), 1);
    }

    #[test]
    fn instance_layouts_match_the_shader() {
        assert_eq!(size_of::<PointInstance>(), 16);
        assert_eq!(size_of::<EdgeInstance>(), 32);
        assert_eq!(size_of::<FaceVertex>(), 16);
    }
}
