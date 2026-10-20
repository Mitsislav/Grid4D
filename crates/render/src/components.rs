//! Component modes in the viewport: picking points, edges and polygons of the
//! active polygon object (a click, a rectangle, a brush stroke), and the overlay
//! that shows them.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec2, Vec3, Vec4};
use grid4d_eval::EvaluatedScene;
use grid4d_geom::{
    BitSet, Component, Triangulator,
    selection::{edges_from_corners, hidden_closure},
};
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
    pub(crate) fn to_screen(self, clip: Vec4) -> Vec2 {
        let ndc = clip.truncate().truncate() / clip.w;
        Vec2::new((ndc.x * 0.5 + 0.5) * self.size.x, (0.5 - ndc.y * 0.5) * self.size.y)
    }
}

/// `u32` index → `usize` (lossless on every supported platform).
fn idx(index: u32) -> usize {
    usize::try_from(index).unwrap_or(usize::MAX)
}

/// `usize` index of a validated mesh → `u32`.
fn index_u32(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

/// Ray parameters closer than this to the target (along eye → target) still
/// count as reaching it: the component's own faces must not hide it.
const VISIBLE_TOLERANCE: f32 = 1e-3;

/// A visible polygon object as screen-space selection sees it.
struct ObjectView<'a> {
    handle: &'a MeshHandle,
    world: Mat4,
    /// The camera position in object space.
    eye: Vec3,
    view: &'a ScreenView,
    hidden: Hidden,
}

impl<'a> ObjectView<'a> {
    fn new(scene: &'a EvaluatedScene, node: NodeId, view: &'a ScreenView) -> Option<Self> {
        let (evaluated, handle, components) = scene.polygon(node)?;
        let world = evaluated.world;
        if world.determinant().abs() <= f32::EPSILON {
            return None;
        }
        Some(Self {
            handle,
            world,
            eye: world.inverse().transform_point3(view.eye),
            view,
            hidden: Hidden::new(handle, components.state()),
        })
    }

    fn clip(&self, p: Vec3) -> Vec4 {
        self.view.view_proj * self.world.transform_point3(p).extend(1.0)
    }

    /// Screen position of object-space point `p`; `None` behind the camera.
    fn screen(&self, p: Vec3) -> Option<Vec2> {
        let clip = self.clip(p);
        (clip.w > 1e-6 && clip.z >= 0.0).then(|| self.view.to_screen(clip))
    }

    /// No (unhidden) polygon lies between the camera and object-space point `target`.
    fn visible(&self, target: Vec3) -> bool {
        self.handle
            .raycast_where(self.eye, target - self.eye, |f| !self.hidden.polygons.contains(idx(f)))
            .is_none_or(|hit| hit.t >= 1.0 - VISIBLE_TOLERANCE)
    }

    /// A polygon is visible if its centre or a point near one of its corners is.
    fn face_visible(&self, face: usize) -> bool {
        let mesh = self.handle.mesh();
        let centre = mesh.face_center(face);
        self.visible(centre)
            || mesh
                .face(face)
                .iter()
                .any(|&p| self.visible(mesh.points()[idx(p)].lerp(centre, 0.2)))
    }
}

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
    let o = ObjectView::new(scene, node, view)?;
    let mesh = o.handle.mesh();
    match component {
        Component::Polygon => {
            let ndc = Vec2::new(cursor.x / view.size.x * 2.0 - 1.0, 1.0 - cursor.y / view.size.y * 2.0);
            let to_object = (view.view_proj * o.world).inverse();
            let far = to_object.project_point3(ndc.extend(1.0));
            let near = to_object.project_point3(ndc.extend(0.0));
            o.handle
                .raycast_where(near, far - near, |f| !o.hidden.polygons.contains(idx(f)))
                .map(|hit| ComponentHit::Polygon(hit.face))
        }
        Component::Point => {
            let mut candidates: Vec<(f32, usize)> = (0..mesh.point_count())
                .filter(|&p| !o.hidden.points.contains(p))
                .filter_map(|p| Some((o.screen(mesh.points()[p])?.distance(cursor), p)))
                .filter(|&(d, _)| d <= radius)
                .collect();
            candidates.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
            candidates
                .into_iter()
                .find(|&(_, p)| o.visible(mesh.points()[p]))
                .map(|(_, p)| ComponentHit::Point(index_u32(p)))
        }
        Component::Edge => {
            let topology = o.handle.topology();
            let mut candidates: Vec<(f32, usize, Vec3)> = topology
                .edges()
                .iter()
                .enumerate()
                .filter(|&(e, _)| !o.hidden.edges.contains(e))
                .filter_map(|(e, &[a, b])| {
                    let (a, b) = (mesh.points()[idx(a)], mesh.points()[idx(b)]);
                    let (d, closest) = edge_distance(view, o.clip(a), o.clip(b), a, b, cursor)?;
                    Some((d, e, closest))
                })
                .filter(|&(d, _, _)| d <= radius)
                .collect();
            candidates.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
            candidates
                .into_iter()
                .find(|&(_, _, closest)| o.visible(closest))
                .map(|(_, e, _)| ComponentHit::Edge(index_u32(e)))
        }
    }
}

/// The edge of polygon `face` nearest to `cursor` on screen (a unique edge index):
/// which way a polygon loop or ring through that polygon runs.
#[must_use]
pub fn nearest_face_edge(
    scene: &EvaluatedScene,
    node: NodeId,
    face: u32,
    view: &ScreenView,
    cursor: Vec2,
) -> Option<u32> {
    let o = ObjectView::new(scene, node, view)?;
    let (mesh, topology) = (o.handle.mesh(), o.handle.topology());
    mesh.corner_range(idx(face))
        .filter_map(|c| {
            let e = topology.corner_edge(c)?;
            let [a, b] = topology.edges()[idx(e)];
            let (a, b) = (mesh.points()[idx(a)], mesh.points()[idx(b)]);
            Some((edge_distance(view, o.clip(a), o.clip(b), a, b, cursor)?.0, e))
        })
        .min_by(|x, y| x.0.total_cmp(&y.0))
        .map(|(_, e)| e)
}

/// How a frame or brush decides which components it takes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegionRules {
    /// Take an edge or polygon as soon as any part of it is inside, not only
    /// when all of it (or, for the brush, three quarters of it) is.
    pub tolerant: bool,
    /// Leave out components hidden behind other polygons.
    pub visible_only: bool,
}

/// The components of kind `component` inside the screen rectangle spanned by
/// `corners` (points, unique edge indices or polygons), as the Rectangle
/// Selection tool takes them (see [`RegionRules`]): an edge or polygon counts
/// when all of its points are inside, or — tolerant — when any part of it is.
#[must_use]
pub fn components_in_rect(
    scene: &EvaluatedScene,
    node: NodeId,
    component: Component,
    view: &ScreenView,
    corners: [Vec2; 2],
    rules: RegionRules,
) -> Vec<u32> {
    let RegionRules { tolerant, visible_only } = rules;
    let [corner_a, corner_b] = corners;
    let Some(o) = ObjectView::new(scene, node, view) else {
        return Vec::new();
    };
    let (min, max) = (corner_a.min(corner_b), corner_a.max(corner_b));
    let inside = |s: Option<Vec2>| s.is_some_and(|s| s.cmpge(min).all() && s.cmple(max).all());
    let (mesh, topology) = (o.handle.mesh(), o.handle.topology());
    let points = mesh.points();
    let found = match component {
        Component::Point => (0..mesh.point_count())
            .filter(|&p| !o.hidden.points.contains(p) && inside(o.screen(points[p])))
            .filter(|&p| !visible_only || o.visible(points[p]))
            .collect::<Vec<_>>(),
        Component::Edge => topology
            .edges()
            .iter()
            .enumerate()
            .filter(|&(e, _)| !o.hidden.edges.contains(e))
            .filter(|&(_, &[a, b])| {
                let (pa, pb) = (points[idx(a)], points[idx(b)]);
                let (sa, sb) = (o.screen(pa), o.screen(pb));
                let taken = if tolerant {
                    inside(sa)
                        || inside(sb)
                        || sa
                            .zip(sb)
                            .is_some_and(|(sa, sb)| segment_crosses_rect(sa, sb, min, max))
                } else {
                    inside(sa) && inside(sb)
                };
                taken && (!visible_only || o.visible(pa.lerp(pb, 0.5)))
            })
            .map(|(e, _)| e)
            .collect(),
        Component::Polygon => (0..mesh.face_count())
            .filter(|&f| !o.hidden.polygons.contains(f))
            .filter(|&f| {
                let corners: Vec<Option<Vec2>> = mesh.face(f).iter().map(|&p| o.screen(points[idx(p)])).collect();
                let taken = if tolerant {
                    corners.iter().any(|&s| inside(s)) || {
                        let outline: Option<Vec<Vec2>> = corners.iter().copied().collect();
                        outline.is_some_and(|outline| {
                            point_in_polygon((min + max) * 0.5, &outline)
                                || (0..outline.len()).any(|i| {
                                    segment_crosses_rect(outline[i], outline[(i + 1) % outline.len()], min, max)
                                })
                        })
                    }
                } else {
                    corners.iter().all(|&s| inside(s))
                };
                taken && (!visible_only || o.face_visible(f))
            })
            .collect(),
    };
    found.into_iter().map(index_u32).collect()
}

/// Does the segment `a`–`b` cross the rectangle `[min, max]`'s outline?
fn segment_crosses_rect(a: Vec2, b: Vec2, min: Vec2, max: Vec2) -> bool {
    let corners = [min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)];
    (0..4).any(|i| segments_cross(a, b, corners[i], corners[(i + 1) % 4]))
}

fn segments_cross(p1: Vec2, p2: Vec2, q1: Vec2, q2: Vec2) -> bool {
    let side = |a: Vec2, b: Vec2, c: Vec2| (b - a).perp_dot(c - a);
    let (d1, d2) = (side(q1, q2, p1), side(q1, q2, p2));
    let (d3, d4) = (side(p1, p2, q1), side(p1, p2, q2));
    (d1 * d2 <= 0.0) && (d3 * d4 <= 0.0) && !(d1 == 0.0 && d2 == 0.0)
}

/// Even-odd rule.
fn point_in_polygon(point: Vec2, polygon: &[Vec2]) -> bool {
    let mut inside = false;
    for i in 0..polygon.len() {
        let (a, b) = (polygon[i], polygon[(i + polygon.len() - 1) % polygon.len()]);
        if (a.y > point.y) != (b.y > point.y) && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
    }
    inside
}

/// Samples along an edge for the brush's coverage.
const EDGE_SAMPLES: u32 = 8;
/// At most this many samples per polygon (the centre, corners, edge midpoints).
const MAX_FACE_SAMPLES: usize = 32;

/// What a Live Selection stroke has painted so far: for each component, which of
/// its sample points the brush has covered. Keep one per stroke.
#[derive(Debug, Default)]
pub struct BrushCoverage {
    /// Component → (covered samples as bits, sample count).
    covered: HashMap<u32, (u32, u32)>,
}

impl BrushCoverage {
    pub fn clear(&mut self) {
        self.covered.clear();
    }

    /// The components painted enough: any part (`tolerant`) or at least three
    /// quarters of their samples (a point has one sample).
    pub fn painted(&self, tolerant: bool) -> impl Iterator<Item = u32> + '_ {
        self.covered.iter().filter_map(move |(&element, &(mask, samples))| {
            let enough = if tolerant {
                mask != 0
            } else {
                mask.count_ones() * 4 >= samples * 3
            };
            enough.then_some(element)
        })
    }
}

/// A Live Selection brush stamp, in screen units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Brush {
    pub centre: Vec2,
    pub radius: f32,
    /// Leave out samples hidden behind other polygons.
    pub visible_only: bool,
}

/// Paints one `brush` stamp onto `coverage`: every sample point of a component
/// inside the brush (and, if `visible_only`, not hidden behind other polygons)
/// counts as painted.
pub fn paint_brush(
    scene: &EvaluatedScene,
    node: NodeId,
    component: Component,
    view: &ScreenView,
    brush: Brush,
    coverage: &mut BrushCoverage,
) {
    let Brush {
        centre,
        radius,
        visible_only,
    } = brush;
    let Some(o) = ObjectView::new(scene, node, view) else {
        return;
    };
    let (mesh, topology) = (o.handle.mesh(), o.handle.topology());
    let points = mesh.points();
    let r2 = radius * radius;
    let mut stamp = |element: usize, sample: u32, samples: u32, p: Vec3| {
        let painted = o.screen(p).is_some_and(|s| s.distance_squared(centre) <= r2) && (!visible_only || o.visible(p));
        if painted {
            let entry = coverage.covered.entry(index_u32(element)).or_insert((0, samples));
            entry.0 |= 1 << sample;
        }
    };
    match component {
        Component::Point => {
            for p in (0..mesh.point_count()).filter(|&p| !o.hidden.points.contains(p)) {
                stamp(p, 0, 1, points[p]);
            }
        }
        Component::Edge => {
            for (e, &[a, b]) in topology.edges().iter().enumerate() {
                if o.hidden.edges.contains(e) {
                    continue;
                }
                for i in 0..EDGE_SAMPLES {
                    let t = (f32::from(u8::try_from(i).unwrap_or(0)) + 0.5)
                        / f32::from(u8::try_from(EDGE_SAMPLES).unwrap_or(1));
                    stamp(e, i, EDGE_SAMPLES, points[idx(a)].lerp(points[idx(b)], t));
                }
            }
        }
        Component::Polygon => {
            let mut samples = Vec::with_capacity(MAX_FACE_SAMPLES);
            for f in (0..mesh.face_count()).filter(|&f| !o.hidden.polygons.contains(f)) {
                let face = mesh.face(f);
                let centre = mesh.face_center(f);
                samples.clear();
                samples.push(centre);
                // Corners and edge midpoints, pulled in a little so neighbours do not share them.
                let step = face.len().div_ceil((MAX_FACE_SAMPLES - 1) / 2).max(1);
                for i in (0..face.len()).step_by(step) {
                    let (a, b) = (points[idx(face[i])], points[idx(face[(i + 1) % face.len()])]);
                    samples.push(a.lerp(centre, 0.15));
                    samples.push(a.lerp(b, 0.5).lerp(centre, 0.15));
                }
                let count = index_u32(samples.len());
                for (i, &p) in samples.iter().enumerate() {
                    stamp(f, index_u32(i), count, p);
                }
            }
        }
    }
}

/// Screen distance from `cursor` to the edge `a`–`b` (object space, with clip
/// coordinates `ca`/`cb`), clipped to the near plane, and the edge's point closest
/// to the cursor in object space. `None` if the edge is behind the camera.
pub(crate) fn edge_distance(
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
/// polygons are all hidden ([`hidden_closure`]).
struct Hidden {
    points: BitSet,
    /// By unique edge index.
    edges: BitSet,
    polygons: BitSet,
}

impl Hidden {
    fn new(handle: &MeshHandle, state: &ComponentState) -> Self {
        let topology = handle.topology();
        let closure = hidden_closure(handle.mesh(), topology, &state.hidden);
        Self {
            edges: edges_from_corners(&closure.edges, topology),
            points: closure.points,
            polygons: closure.polygons,
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
    /// What the hover arrays show: one component, or a whole loop about to be selected.
    hover: Vec<ComponentHit>,
    triangulator: Triangulator,
    triangles: Vec<[u32; 3]>,
}

impl ComponentOverlay {
    pub(crate) fn update(
        &mut self,
        scene: &EvaluatedScene,
        target: Option<(NodeId, Component)>,
        hover: &[ComponentHit],
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
            // The hover arrays may refer to the old mesh or placement.
            self.set_hover(handle, world, hover);
        } else if hover != self.hover.as_slice() {
            self.set_hover(handle, world, hover);
        }
    }

    fn clear(&mut self) {
        if self.built.is_none() && self.hover.is_empty() {
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
        self.hover.clear();
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

    fn set_hover(&mut self, handle: &MeshHandle, world: Mat4, hover: &[ComponentHit]) {
        self.hover_points.clear();
        self.hover_edges.clear();
        self.hover_faces.clear();
        let mesh = handle.mesh();
        let at = |p: u32| mesh.points().get(idx(p)).map(|&q| world.transform_point3(q).to_array());
        for &hit in hover {
            match hit {
                ComponentHit::Point(p) => {
                    if let Some(position) = at(p) {
                        self.hover_points.push(PointInstance {
                            position,
                            flags: HOVERED,
                        });
                    }
                }
                ComponentHit::Edge(e) => {
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
                ComponentHit::Polygon(f) if idx(f) < mesh.face_count() => {
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
                ComponentHit::Polygon(_) => {}
            }
        }
        self.hover.clear();
        self.hover.extend_from_slice(hover);
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
        overlay.update(&eval, Some((id, Component::Point)), &[]);
        assert_eq!(overlay.points.len(), 8);
        assert_eq!(overlay.points.iter().filter(|p| p.flags == SELECTED).count(), 1);
        assert_eq!(overlay.edges.len(), 12, "context edges");
        assert!(overlay.edges.iter().all(|e| e.flags == CONTEXT));
        let generation = overlay.generation;
        let capacity = (overlay.points.capacity(), overlay.edges.capacity());

        for _ in 0..3 {
            overlay.update(&eval, Some((id, Component::Point)), &[]);
        }
        assert_eq!(overlay.generation, generation, "idle frames rebuild nothing");

        overlay.update(&eval, Some((id, Component::Point)), &[ComponentHit::Point(5)]);
        assert_eq!(
            overlay.generation, generation,
            "hovering does not touch the main arrays"
        );
        assert_eq!(overlay.hover_points.len(), 1);
        assert_eq!((overlay.points.capacity(), overlay.edges.capacity()), capacity);

        overlay.update(&eval, Some((id, Component::Polygon)), &[]);
        assert!(overlay.generation > generation, "another mode rebuilds");
        assert!(
            overlay.points.is_empty() && overlay.faces.is_empty(),
            "nothing selected in Polygons"
        );
        overlay.update(&eval, None, &[]);
        assert!(overlay.edges.is_empty() && overlay.hover_points.is_empty());
    }

    #[test]
    fn selected_polygons_are_tinted_and_edges_highlighted() {
        let mut state = ComponentState::default();
        state.selected.polygons.insert(0);
        state.selected.edges.insert(0);
        let (_, id, eval) = scene_with_cube(state);
        let mut overlay = ComponentOverlay::default();
        overlay.update(&eval, Some((id, Component::Polygon)), &[]);
        assert_eq!(overlay.faces.len(), 6, "one quad = two triangles");
        overlay.update(&eval, Some((id, Component::Edge)), &[]);
        assert_eq!(overlay.edges.iter().filter(|e| e.flags == SELECTED).count(), 1);
    }

    #[test]
    fn a_rectangle_takes_only_whole_visible_components_unless_tolerant() {
        let (_, id, eval) = scene_with_cube(ComponentState::default());
        let view = view();
        let mesh = mesh_of(&eval, id);
        let points = mesh.mesh().points();
        let everything = (Vec2::ZERO, view.size);
        let visible = components_in_rect(
            &eval,
            id,
            Component::Point,
            &view,
            [everything.0, everything.1],
            RegionRules {
                tolerant: false,
                visible_only: true,
            },
        );
        assert_eq!(visible.len(), 7, "the corner at the back is hidden by the cube");
        let all = components_in_rect(
            &eval,
            id,
            Component::Point,
            &view,
            [everything.0, everything.1],
            RegionRules {
                tolerant: false,
                visible_only: false,
            },
        );
        assert_eq!(all.len(), 8, "visible only off");

        // A rectangle around the top face's centre only: no face fully inside…
        let top = (0..mesh.mesh().face_count())
            .find(|&f| mesh.mesh().face_normal(f).abs_diff_eq(Vec3::Y, 1e-5))
            .unwrap();
        let centre = screen(&view, mesh.mesh().face_center(top));
        let (a, b) = (centre - Vec2::splat(3.0), centre + Vec2::splat(3.0));
        assert!(
            components_in_rect(
                &eval,
                id,
                Component::Polygon,
                &view,
                [a, b],
                RegionRules {
                    tolerant: false,
                    visible_only: true
                }
            )
            .is_empty()
        );
        // …but the top face is touched.
        let touched = components_in_rect(
            &eval,
            id,
            Component::Polygon,
            &view,
            [a, b],
            RegionRules {
                tolerant: true,
                visible_only: true,
            },
        );
        assert_eq!(touched, [u32::try_from(top).unwrap()]);
        // A rectangle around all of the top face's corners takes it whole.
        let corners: Vec<Vec2> = mesh
            .mesh()
            .face(top)
            .iter()
            .map(|&p| screen(&view, points[p as usize]))
            .collect();
        let (lo, hi) = corners
            .iter()
            .fold((Vec2::INFINITY, Vec2::NEG_INFINITY), |(lo, hi), &c| {
                (lo.min(c), hi.max(c))
            });
        let whole = components_in_rect(
            &eval,
            id,
            Component::Polygon,
            &view,
            [lo - 2.0, hi + 2.0],
            RegionRules {
                tolerant: false,
                visible_only: true,
            },
        );
        assert!(whole.contains(&u32::try_from(top).unwrap()));
    }

    #[test]
    fn the_brush_paints_points_and_needs_most_of_a_polygon() {
        let (_, id, eval) = scene_with_cube(ComponentState::default());
        let view = view();
        let mesh = mesh_of(&eval, id);
        let top = (0..mesh.mesh().face_count())
            .find(|&f| mesh.mesh().face_normal(f).abs_diff_eq(Vec3::Y, 1e-5))
            .unwrap();
        let centre = screen(&view, mesh.mesh().face_center(top));

        let mut coverage = BrushCoverage::default();
        paint_brush(
            &eval,
            id,
            Component::Polygon,
            &view,
            Brush {
                centre,
                radius: 4.0,
                visible_only: true,
            },
            &mut coverage,
        );
        let top = u32::try_from(top).unwrap();
        assert!(coverage.painted(true).any(|f| f == top), "tolerant: touched is enough");
        assert!(
            !coverage.painted(false).any(|f| f == top),
            "strict: a dab in the middle is not enough"
        );
        paint_brush(
            &eval,
            id,
            Component::Polygon,
            &view,
            Brush {
                centre,
                radius: 400.0,
                visible_only: true,
            },
            &mut coverage,
        );
        assert!(coverage.painted(false).any(|f| f == top), "covered everywhere");

        let mut points = BrushCoverage::default();
        paint_brush(
            &eval,
            id,
            Component::Point,
            &view,
            Brush {
                centre: view.size * 0.5,
                radius: 2000.0,
                visible_only: true,
            },
            &mut points,
        );
        assert_eq!(points.painted(false).count(), 7, "the hidden corner is not painted");
        let mut through = BrushCoverage::default();
        paint_brush(
            &eval,
            id,
            Component::Point,
            &view,
            Brush {
                centre: view.size * 0.5,
                radius: 2000.0,
                visible_only: false,
            },
            &mut through,
        );
        assert_eq!(through.painted(false).count(), 8);
    }

    #[test]
    fn loops_can_be_previewed_under_the_cursor() {
        let (_, id, eval) = scene_with_cube(ComponentState::default());
        let mut overlay = ComponentOverlay::default();
        let loop_edges = [ComponentHit::Edge(0), ComponentHit::Edge(1), ComponentHit::Edge(2)];
        overlay.update(&eval, Some((id, Component::Edge)), &loop_edges);
        assert_eq!(overlay.hover_edges.len(), 3);
        let generation = overlay.hover_generation;
        overlay.update(&eval, Some((id, Component::Edge)), &loop_edges);
        assert_eq!(overlay.hover_generation, generation, "same preview, no re-upload");
    }

    #[test]
    fn instance_layouts_match_the_shader() {
        assert_eq!(size_of::<PointInstance>(), 16);
        assert_eq!(size_of::<EdgeInstance>(), 32);
        assert_eq!(size_of::<FaceVertex>(), 16);
    }
}
