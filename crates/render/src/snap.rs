//! Snapping (see `docs/reference/modeling-snapping.md`): the target near a screen
//! position that a moved element jumps onto — a mesh point, an edge, the middle of
//! an edge, a polygon's centre or surface, an object's axis, a floor grid point or
//! grid line.
//!
//! Runs while a snapping drag moves (once per pointer move), in screen space like
//! component picking: it projects the points and edges of every drawn mesh.
//! Primitives' unit meshes are generated once per [`MeshKey`] and kept in a
//! [`SnapCache`].

use std::collections::HashMap;

use glam::{Mat4, Vec2, Vec3};
use grid4d_eval::{EvaluatedScene, MeshKey};
use grid4d_geom::BitSet;
use grid4d_scene::{MeshHandle, NodeId};

use crate::components::{ScreenView, edge_distance};

/// What snapping can stick to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SnapKind {
    Vertex,
    Edge,
    EdgeMidPoint,
    PolygonCenter,
    Polygon,
    Axis,
    GridPoint,
    GridLine,
}

impl SnapKind {
    pub const ALL: [Self; 8] = [
        Self::Vertex,
        Self::Edge,
        Self::EdgeMidPoint,
        Self::PolygonCenter,
        Self::Polygon,
        Self::Axis,
        Self::GridPoint,
        Self::GridLine,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Vertex => "Vertex",
            Self::Edge => "Edge",
            Self::EdgeMidPoint => "Edge Mid Point",
            Self::PolygonCenter => "Polygon Center",
            Self::Polygon => "Polygon",
            Self::Axis => "Axis",
            Self::GridPoint => "Grid Point",
            Self::GridLine => "Grid Line",
        }
    }

    /// Point-like targets win over line-like ones, which win over surfaces, so a
    /// vertex can be caught next to the edges that end in it.
    const fn class(self) -> u8 {
        match self {
            Self::Vertex | Self::EdgeMidPoint | Self::PolygonCenter | Self::Axis | Self::GridPoint => 0,
            Self::Edge | Self::GridLine => 1,
            Self::Polygon => 2,
        }
    }
}

/// Which targets snapping uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapTargets {
    pub vertex: bool,
    pub edge: bool,
    pub edge_mid_point: bool,
    pub polygon_center: bool,
    pub polygon: bool,
    pub axis: bool,
    pub grid_point: bool,
    pub grid_line: bool,
}

impl Default for SnapTargets {
    fn default() -> Self {
        Self {
            vertex: true,
            edge: true,
            edge_mid_point: true,
            polygon_center: true,
            polygon: false,
            axis: true,
            grid_point: true,
            grid_line: false,
        }
    }
}

impl SnapTargets {
    #[must_use]
    pub const fn get(&self, kind: SnapKind) -> bool {
        match kind {
            SnapKind::Vertex => self.vertex,
            SnapKind::Edge => self.edge,
            SnapKind::EdgeMidPoint => self.edge_mid_point,
            SnapKind::PolygonCenter => self.polygon_center,
            SnapKind::Polygon => self.polygon,
            SnapKind::Axis => self.axis,
            SnapKind::GridPoint => self.grid_point,
            SnapKind::GridLine => self.grid_line,
        }
    }

    /// The flag of `kind`, for a checkbox.
    pub const fn flag_mut(&mut self, kind: SnapKind) -> &mut bool {
        match kind {
            SnapKind::Vertex => &mut self.vertex,
            SnapKind::Edge => &mut self.edge,
            SnapKind::EdgeMidPoint => &mut self.edge_mid_point,
            SnapKind::PolygonCenter => &mut self.polygon_center,
            SnapKind::Polygon => &mut self.polygon,
            SnapKind::Axis => &mut self.axis,
            SnapKind::GridPoint => &mut self.grid_point,
            SnapKind::GridLine => &mut self.grid_line,
        }
    }

    /// Whether any mesh-based target is on.
    const fn any_mesh(self) -> bool {
        self.vertex || self.edge || self.edge_mid_point || self.polygon_center || self.polygon
    }
}

/// One snap search.
#[derive(Clone, Copy, Debug)]
pub struct SnapQuery<'a> {
    /// The cursor: targets are looked for around it (the unit and origin of
    /// [`ScreenView::size`]).
    pub at: Vec2,
    /// How close a target must be, on screen.
    pub radius: f32,
    pub targets: SnapTargets,
    /// Spacing of the floor grid (world units).
    pub grid_spacing: f32,
    /// Objects whose whole subtree moves (or changes) with the drag: no targets.
    pub skip: &'a [NodeId],
    /// A polygon object whose points are being moved: its own mesh is searched
    /// (even inside a skipped subtree) without those points and the edges and
    /// polygons that use them.
    pub moving: Option<(NodeId, &'a BitSet)>,
}

/// A snap target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapHit {
    pub kind: SnapKind,
    /// World position the element snaps to.
    pub position: Vec3,
    /// Its screen position.
    pub screen: Vec2,
    /// For an edge or a grid line: a stretch of it (world), to highlight.
    pub segment: Option<[Vec3; 2]>,
}

/// Unit meshes of primitives, generated once per key.
#[derive(Debug, Default)]
pub struct SnapCache {
    unit: HashMap<MeshKey, Option<MeshHandle>>,
}

impl SnapCache {
    fn unit_mesh(&mut self, key: MeshKey) -> Option<MeshHandle> {
        self.unit
            .entry(key)
            .or_insert_with(|| key.build().map(MeshHandle::new))
            .clone()
    }
}

/// The best target so far.
struct Best {
    radius: f32,
    hit: Option<(u8, f32, SnapHit)>,
}

impl Best {
    /// Whether a target of `kind` could still win.
    fn wants(&self, kind: SnapKind) -> bool {
        self.hit.is_none_or(|(class, _, _)| kind.class() <= class)
    }

    fn offer(&mut self, kind: SnapKind, distance: f32, position: Vec3, screen: Vec2, segment: Option<[Vec3; 2]>) {
        if distance > self.radius || !self.wants(kind) {
            return;
        }
        let class = kind.class();
        if self.hit.is_some_and(|(c, d, _)| c == class && d <= distance) {
            return;
        }
        let hit = SnapHit {
            kind,
            position,
            screen,
            segment,
        };
        self.hit = Some((class, distance, hit));
    }
}

/// Screen position of world point `p`; `None` behind the camera.
fn project(view: &ScreenView, p: Vec3) -> Option<Vec2> {
    let clip = view.view_proj * p.extend(1.0);
    (clip.w > 1e-6 && clip.z >= 0.0).then(|| view.to_screen(clip))
}

/// The world-space ray through screen position `at`, from the eye (the camera
/// is a perspective one). The direction comes from a point deep in the view
/// (depth 0.999, about a thousand near distances away).
fn ray(view: &ScreenView, at: Vec2) -> Option<(Vec3, Vec3)> {
    let ndc = Vec2::new(at.x / view.size.x * 2.0 - 1.0, 1.0 - at.y / view.size.y * 2.0);
    // In f64: the projection's inverse loses precision in f32 (near 2, far 200 000).
    let deep = view
        .view_proj
        .as_dmat4()
        .inverse()
        .project_point3(ndc.extend(0.999).as_dvec3());
    let dir = (deep - view.eye.as_dvec3()).as_vec3();
    (dir.is_finite() && dir.length_squared() > 0.0).then_some((view.eye, dir))
}

/// The target nearest `query.at` (point-like targets first, see
/// [`SnapKind`]), within `query.radius`; `None` if there is none.
#[must_use]
pub fn find_snap(
    scene: &EvaluatedScene,
    view: &ScreenView,
    cache: &mut SnapCache,
    query: &SnapQuery<'_>,
) -> Option<SnapHit> {
    let mut best = Best {
        radius: query.radius,
        hit: None,
    };
    let targets = query.targets;
    let point = |best: &mut Best, kind: SnapKind, p: Vec3| {
        if let Some(screen) = project(view, p) {
            best.offer(kind, screen.distance(query.at), p, screen, None);
        }
    };

    let mut skip_depth: Option<u32> = None;
    for node in scene.nodes() {
        let moving = query.moving.filter(|&(id, _)| id == node.id).map(|(_, points)| points);
        if moving.is_none() {
            if skip_depth.is_some_and(|d| node.depth > d) {
                continue;
            }
            skip_depth = None;
            if query.skip.contains(&node.id) {
                skip_depth = Some(node.depth);
                continue;
            }
        }
        if targets.axis {
            point(&mut best, SnapKind::Axis, node.world.w_axis.truncate());
        }
        if !targets.any_mesh() {
            continue;
        }
        let source = match (moving, node.drawable) {
            (Some(_), _) => scene.polygon(node.id).map(|(_, mesh, _)| (mesh.clone(), node.world)),
            (None, Some(drawable)) if !drawable.marker => {
                let model = node.world * Mat4::from_scale(drawable.scale);
                match drawable.key {
                    MeshKey::Polygon(id) => scene.mesh(id).cloned(),
                    key => cache.unit_mesh(key),
                }
                .map(|mesh| (mesh, model))
            }
            (None, _) => None,
        };
        if let Some((mesh, model)) = source {
            search_mesh(&mut best, view, query, &mesh, model, moving);
        }
    }
    if targets.grid_point || targets.grid_line {
        search_grid(&mut best, view, query);
    }
    best.hit.map(|(_, _, hit)| hit)
}

/// Offers the targets of one mesh (`model`: its object-to-world matrix).
fn search_mesh(
    best: &mut Best,
    view: &ScreenView,
    query: &SnapQuery<'_>,
    handle: &MeshHandle,
    model: Mat4,
    moving: Option<&BitSet>,
) {
    let targets = query.targets;
    let mesh = handle.mesh();
    let points = mesh.points();
    let moved = |p: u32| moving.is_some_and(|m| usize::try_from(p).is_ok_and(|p| m.contains(p)));
    let world = |p: u32| {
        usize::try_from(p)
            .ok()
            .and_then(|p| points.get(p))
            .map(|&p| model.transform_point3(p))
    };
    let offer_point = |best: &mut Best, kind: SnapKind, p: Vec3| {
        if let Some(screen) = project(view, p) {
            best.offer(kind, screen.distance(query.at), p, screen, None);
        }
    };

    if targets.vertex {
        for (i, &p) in points.iter().enumerate() {
            if moving.is_some_and(|m| m.contains(i)) {
                continue;
            }
            offer_point(best, SnapKind::Vertex, model.transform_point3(p));
        }
    }
    if targets.edge_mid_point || (targets.edge && best.wants(SnapKind::Edge)) {
        for &[a, b] in handle.topology().edges() {
            if moved(a) || moved(b) {
                continue;
            }
            let (Some(pa), Some(pb)) = (world(a), world(b)) else {
                continue;
            };
            if targets.edge_mid_point {
                offer_point(best, SnapKind::EdgeMidPoint, (pa + pb) * 0.5);
            }
            if targets.edge && best.wants(SnapKind::Edge) {
                let clip = |p: Vec3| view.view_proj * p.extend(1.0);
                if let Some((distance, closest)) = edge_distance(view, clip(pa), clip(pb), pa, pb, query.at)
                    && let Some(screen) = project(view, closest)
                {
                    best.offer(SnapKind::Edge, distance, closest, screen, Some([pa, pb]));
                }
            }
        }
    }
    if targets.polygon_center {
        for f in 0..mesh.face_count() {
            if mesh.face(f).iter().any(|&p| moved(p)) {
                continue;
            }
            offer_point(
                best,
                SnapKind::PolygonCenter,
                model.transform_point3(mesh.face_center(f)),
            );
        }
    }
    if targets.polygon
        && best.wants(SnapKind::Polygon)
        && model.determinant().abs() > f32::EPSILON
        && let Some((origin, dir)) = ray(view, query.at)
    {
        let to_object = model.inverse();
        let hit = handle.raycast_where(
            to_object.transform_point3(origin),
            to_object.transform_vector3(dir),
            |f| {
                usize::try_from(f)
                    .ok()
                    .is_none_or(|f| !mesh.face(f).iter().any(|&p| moved(p)))
            },
        );
        if let Some(hit) = hit {
            let position = origin + dir * hit.t;
            if let Some(screen) = project(view, position) {
                best.offer(SnapKind::Polygon, screen.distance(query.at), position, screen, None);
            }
        }
    }
}

/// Offers the floor grid's point and lines nearest to where the ray through
/// `query.at` meets the floor (y = 0).
fn search_grid(best: &mut Best, view: &ScreenView, query: &SnapQuery<'_>) {
    let spacing = query.grid_spacing;
    if spacing <= 0.0 || !spacing.is_finite() {
        return;
    }
    let Some((origin, dir)) = ray(view, query.at) else {
        return;
    };
    if dir.y.abs() <= f32::EPSILON {
        return;
    }
    let t = -origin.y / dir.y;
    if t <= 0.0 {
        return;
    }
    let floor = origin + dir * t;
    let (x, z) = (
        (floor.x / spacing).round() * spacing,
        (floor.z / spacing).round() * spacing,
    );
    if query.targets.grid_point {
        let p = Vec3::new(x, 0.0, z);
        if let Some(screen) = project(view, p) {
            best.offer(SnapKind::GridPoint, screen.distance(query.at), p, screen, None);
        }
    }
    if query.targets.grid_line && best.wants(SnapKind::GridLine) {
        let reach = spacing * 5.0;
        // The line x = const through the floor point, and the line z = const.
        for (p, along) in [
            (Vec3::new(x, 0.0, floor.z), Vec3::Z),
            (Vec3::new(floor.x, 0.0, z), Vec3::X),
        ] {
            if let Some(screen) = project(view, p) {
                let segment = [p - along * reach, p + along * reach];
                best.offer(SnapKind::GridLine, screen.distance(query.at), p, screen, Some(segment));
            }
        }
    }
}

/// Where a moved element goes when it snaps to `hit`.
///
/// `start` and `axis` (unit) describe a move constrained to a line; `None` for a
/// free move. `depth_from` is the element's current position, whose depth a 2D
/// snap keeps. `three_d`: snap in space (true) or on screen (false).
#[must_use]
pub fn snapped_position(
    view: &ScreenView,
    hit: &SnapHit,
    line: Option<(Vec3, Vec3)>,
    depth_from: Vec3,
    three_d: bool,
) -> Vec3 {
    match (line, three_d) {
        // On the line, as close as possible to the target.
        (Some((start, axis)), true) => start + axis * (hit.position - start).dot(axis),
        // On the line, as close as possible to the view ray through the target.
        (Some((start, axis)), false) => {
            let Some((origin, dir)) = ray(view, hit.screen) else {
                return start + axis * (hit.position - start).dot(axis);
            };
            let dir = dir.normalize_or_zero();
            let w = start - origin;
            let (b, d, e) = (axis.dot(dir), axis.dot(w), dir.dot(w));
            let denominator = 1.0 - b * b;
            if denominator.abs() <= 1e-6 {
                return start + axis * (hit.position - start).dot(axis);
            }
            start + axis * ((b * e - d) / denominator)
        }
        (None, true) => hit.position,
        // On the view ray through the target, in the element's own depth plane
        // (perpendicular to the view direction).
        (None, false) => {
            let (Some((origin, dir)), Some((_, forward))) = (ray(view, hit.screen), ray(view, view.size * 0.5)) else {
                return hit.position;
            };
            let forward = forward.normalize_or_zero();
            let along = dir.dot(forward);
            if along.abs() <= f32::EPSILON {
                return hit.position;
            }
            origin + dir * ((depth_from - origin).dot(forward) / along)
        }
    }
}

#[cfg(test)]
mod tests {
    use grid4d_scene::{Document, Node, ObjectKind, ObjectKindTag};

    use super::*;
    use crate::OrbitCamera;

    fn view() -> ScreenView {
        let camera = OrbitCamera::default();
        ScreenView {
            view_proj: camera.view_projection(1.5),
            eye: camera.eye(),
            size: Vec2::new(900.0, 600.0),
        }
    }

    fn screen(view: &ScreenView, p: Vec3) -> Vec2 {
        project(view, p).unwrap()
    }

    /// A 200 cm polygon cube at the origin inside a Null, and a primitive cube
    /// (also 200 cm) at x = 400.
    struct Fixture {
        doc: Document,
        group: NodeId,
        cube: NodeId,
        primitive: NodeId,
        eval: EvaluatedScene,
    }

    fn fixture() -> Fixture {
        let mut doc = Document::new("snap");
        let group = doc.scene.insert(Node::new("Group", ObjectKind::Null));
        let mesh = grid4d_eval::editable_mesh(&ObjectKindTag::Cube.default_kind()).unwrap();
        let cube = doc
            .scene
            .insert_child(group, Node::new("Cube", ObjectKind::polygon(MeshHandle::new(mesh))))
            .unwrap();
        let primitive = doc.scene.insert(
            Node::new("Primitive", ObjectKindTag::Cube.default_kind()).with_position(Vec3::new(400.0, 0.0, 0.0)),
        );
        let mut eval = EvaluatedScene::new();
        eval.update(&doc.scene);
        Fixture {
            doc,
            group,
            cube,
            primitive,
            eval,
        }
    }

    fn only(kinds: &[SnapKind]) -> SnapTargets {
        let mut targets = SnapTargets::default();
        for kind in SnapKind::ALL {
            *targets.flag_mut(kind) = kinds.contains(&kind);
        }
        targets
    }

    fn query(at: Vec2, targets: SnapTargets) -> SnapQuery<'static> {
        SnapQuery {
            at,
            radius: 10.0,
            targets,
            grid_spacing: 100.0,
            skip: &[],
            moving: None,
        }
    }

    fn find(f: &Fixture, q: &SnapQuery<'_>) -> Option<SnapHit> {
        find_snap(&f.eval, &view(), &mut SnapCache::default(), q)
    }

    const CORNER: Vec3 = Vec3::new(100.0, 100.0, 100.0);

    #[test]
    fn a_vertex_is_found_at_its_exact_position() {
        let f = fixture();
        let v = view();
        let hit = find(
            &f,
            &query(screen(&v, CORNER) + Vec2::new(3.0, -2.0), only(&[SnapKind::Vertex])),
        )
        .unwrap();
        assert_eq!(hit.kind, SnapKind::Vertex);
        assert!(hit.position.abs_diff_eq(CORNER, 1e-3), "{}", hit.position);
        assert!(hit.screen.distance(screen(&v, CORNER)) < 1e-3);
    }

    #[test]
    fn vertices_beat_their_edges_and_edges_are_caught_between_their_ends() {
        let f = fixture();
        let v = view();
        let targets = only(&[SnapKind::Vertex, SnapKind::Edge]);
        let toward = (screen(&v, Vec3::new(100.0, 100.0, -100.0)) - screen(&v, CORNER)).normalize();
        let near_corner = find(&f, &query(screen(&v, CORNER) + toward * 4.0, targets)).unwrap();
        assert_eq!(near_corner.kind, SnapKind::Vertex, "on the edge, 4 px from its end");

        let middle = Vec3::new(100.0, 100.0, 0.0);
        let hit = find(&f, &query(screen(&v, middle) + Vec2::new(0.0, 1.0), targets)).unwrap();
        assert_eq!(hit.kind, SnapKind::Edge);
        assert!(hit.position.abs_diff_eq(middle, 2.0), "{}", hit.position);
        let [a, b] = hit.segment.unwrap();
        assert!(a.x == 100.0 && b.x == 100.0 && a.y == 100.0 && b.y == 100.0, "{a} {b}");
    }

    #[test]
    fn mid_points_centres_axes_and_primitive_vertices_are_exact() {
        let f = fixture();
        let v = view();
        let middle = Vec3::new(100.0, 100.0, 0.0);
        let hit = find(&f, &query(screen(&v, middle), only(&[SnapKind::EdgeMidPoint]))).unwrap();
        assert_eq!((hit.kind, hit.position), (SnapKind::EdgeMidPoint, middle));

        let top = Vec3::new(0.0, 100.0, 0.0);
        let hit = find(
            &f,
            &query(screen(&v, top) + Vec2::new(2.0, 2.0), only(&[SnapKind::PolygonCenter])),
        )
        .unwrap();
        assert_eq!(hit.kind, SnapKind::PolygonCenter);
        assert!(hit.position.abs_diff_eq(top, 1e-3));

        let axis = Vec3::new(400.0, 0.0, 0.0);
        let hit = find(&f, &query(screen(&v, axis), only(&[SnapKind::Axis]))).unwrap();
        assert_eq!((hit.kind, hit.position), (SnapKind::Axis, axis));

        let corner = Vec3::new(500.0, 100.0, 100.0);
        let hit = find(&f, &query(screen(&v, corner), only(&[SnapKind::Vertex]))).unwrap();
        assert!(
            hit.position.abs_diff_eq(corner, 1e-3),
            "the primitive's size: {}",
            hit.position
        );
    }

    #[test]
    fn grid_points_and_grid_lines_follow_the_spacing() {
        let f = fixture();
        let v = view();
        let point = Vec3::new(-300.0, 0.0, 400.0);
        let hit = find(
            &f,
            &query(screen(&v, point) + Vec2::new(3.0, 2.0), only(&[SnapKind::GridPoint])),
        )
        .unwrap();
        assert_eq!(hit.kind, SnapKind::GridPoint);
        assert!(hit.position.abs_diff_eq(point, 1e-3), "{}", hit.position);

        let on_line = Vec3::new(-250.0, 0.0, 400.0);
        let hit = find(&f, &query(screen(&v, on_line), only(&[SnapKind::GridLine]))).unwrap();
        assert_eq!(hit.kind, SnapKind::GridLine);
        assert!(hit.position.abs_diff_eq(on_line, 0.5), "{}", hit.position);
        assert!(hit.segment.is_some());
    }

    #[test]
    fn far_or_disabled_targets_are_ignored() {
        let f = fixture();
        let v = view();
        let away = screen(&v, CORNER) + Vec2::new(15.0, 0.0);
        assert_eq!(find(&f, &query(away, only(&[SnapKind::Vertex]))), None, "15 px > 10 px");
        let mut wide = query(away, only(&[SnapKind::Vertex]));
        wide.radius = 20.0;
        assert!(find(&f, &wide).is_some());
        assert_eq!(find(&f, &query(screen(&v, CORNER), only(&[]))), None, "nothing enabled");
    }

    #[test]
    fn whatever_moves_is_not_a_target() {
        let f = fixture();
        let v = view();
        let mesh = f.eval.polygon(f.cube).unwrap().1.clone();
        let corner = mesh.mesh().points().iter().position(|&p| p == CORNER).unwrap();
        let moved: BitSet = [corner].into_iter().collect();
        let at = screen(&v, CORNER);
        let mut q = query(at, only(&[SnapKind::Vertex, SnapKind::Edge, SnapKind::EdgeMidPoint]));
        q.moving = Some((f.cube, &moved));
        let hit = find(&f, &q);
        assert!(hit.is_none_or(|h| !h.position.abs_diff_eq(CORNER, 1.0)), "{hit:?}");

        // The moving object is searched even inside a skipped subtree.
        let nothing = BitSet::default();
        let skip = [f.group];
        let mut q = query(at, only(&[SnapKind::Vertex]));
        q.skip = &skip;
        assert_eq!(find(&f, &q), None, "the group's subtree is skipped");
        q.moving = Some((f.cube, &nothing));
        assert!(find(&f, &q).is_some(), "but not the object being edited");

        let corner = Vec3::new(500.0, 100.0, 100.0);
        let skip = [f.primitive];
        let mut q = query(screen(&v, corner), only(&[SnapKind::Vertex, SnapKind::Axis]));
        q.skip = &skip;
        assert_eq!(find(&f, &q), None);
        assert!(f.doc.scene.contains(f.primitive));
    }

    #[test]
    fn the_surface_under_the_position_is_a_target() {
        let f = fixture();
        let v = view();
        let on_top = Vec3::new(-20.0, 100.0, 30.0);
        let hit = find(&f, &query(screen(&v, on_top), only(&[SnapKind::Polygon]))).unwrap();
        assert_eq!(hit.kind, SnapKind::Polygon);
        assert!(hit.position.abs_diff_eq(on_top, 0.5), "{}", hit.position);
    }

    #[test]
    fn snapped_positions_on_a_line_or_free_in_3d_and_2d() {
        let v = view();
        let target = Vec3::new(50.0, 30.0, 20.0);
        let hit = SnapHit {
            kind: SnapKind::Vertex,
            position: target,
            screen: screen(&v, target),
            segment: None,
        };
        let line = Some((Vec3::ZERO, Vec3::X));
        assert!(snapped_position(&v, &hit, line, Vec3::ZERO, true).abs_diff_eq(Vec3::new(50.0, 0.0, 0.0), 1e-4));
        assert_eq!(snapped_position(&v, &hit, None, Vec3::ZERO, true), target);

        // 2D, free: on screen at the target, at the element's own depth.
        let element = Vec3::new(-100.0, 0.0, -100.0);
        let p = snapped_position(&v, &hit, None, element, false);
        assert!(
            screen(&v, p).distance(hit.screen) < 0.01,
            "{} vs {}",
            screen(&v, p),
            hit.screen
        );
        let forward = (OrbitCamera::default().target - v.eye).normalize();
        assert!(((p - element).dot(forward)).abs() < 0.01, "same depth");

        // 2D, on a line: the point of the line that looks closest to the target.
        let p = snapped_position(&v, &hit, line, Vec3::ZERO, false);
        assert!(p.y.abs() < 1e-3 && p.z.abs() < 1e-3, "on the line: {p}");
        let d = screen(&v, p).distance(hit.screen);
        for step in [-1.0, 1.0] {
            let other = p + Vec3::X * step;
            assert!(screen(&v, other).distance(hit.screen) >= d - 1e-3);
        }
    }
}
