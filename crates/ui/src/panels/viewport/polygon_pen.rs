//! The Polygon Pen (Mesh ▸ Polygon Pen; see
//! `docs/reference/modeling-tools-2.md`): clicks on the active polygon
//! object's points, edges or surface — or in the plane through its centre
//! facing the view — collect corners; a click on the first corner closes the
//! polygon (`EditorCommand::PenPolygon`, one undo step). Backspace removes
//! the last corner, Esc drops them. Ctrl + drag on a border edge pulls a
//! quad out of it (one undo step; Esc puts it back).

use egui::{Key, Painter, PointerButton, Pos2, Rect, Response, Stroke, Ui, vec2};
use glam::Vec3;
use grid4d_geom::{
    BitSet, Component,
    ops::{self, EdgeExtrudeOptions, PenCorner},
};
use grid4d_render::{ComponentHit, pick_component};
use grid4d_scene::{Components, Document, MeshHandle, NodeData, NodeId, ObjectKind, SceneEdit};

use super::{ViewportPanel, local, project, to_ndc};
use crate::{
    EditorCommand, EditorState, Tool,
    theme::{metrics, view_palette as p},
};

/// The Polygon Pen's state.
#[derive(Debug, Default)]
pub(super) struct PolygonPenState {
    /// The object the corners belong to.
    node: Option<NodeId>,
    /// The corners so far, each with where it is in the world.
    pub(super) corners: Vec<(PenCorner, Vec3)>,
    /// Where the next corner would go (the end of the preview line).
    hover: Option<Vec3>,
    /// A border edge being pulled out.
    pull: Option<Pull>,
}

/// A Ctrl + drag on a border edge.
#[derive(Debug)]
struct Pull {
    node: NodeId,
    before: NodeData,
    edge: usize,
    start: Pos2,
    /// World units per point of horizontal travel.
    per_point: f32,
}

impl ViewportPanel {
    /// Handles the Polygon Pen. Returns `true` while it is the tool and has
    /// a polygon object to draw on: it takes every click in the view.
    pub(super) fn polygon_pen(
        &mut self,
        ui: &Ui,
        response: &Response,
        rect: Rect,
        doc: &mut Document,
        editor: &mut EditorState,
    ) -> bool {
        let target = self.component_target(editor).map(|(node, _)| node);
        let Some(node) = target.filter(|_| editor.tool == Tool::PolygonPen) else {
            self.polygon_pen = PolygonPenState::default();
            return false;
        };
        if self.polygon_pen.node != Some(node) {
            self.polygon_pen = PolygonPenState {
                node: Some(node),
                ..PolygonPenState::default()
            };
        }
        let snap = editor.modeling_settings.pen_snap;
        let (escape, backspace, ctrl, pointer, down, origin) = ui.input(|i| {
            (
                i.key_pressed(Key::Escape),
                i.key_pressed(Key::Backspace),
                i.modifiers.ctrl || i.modifiers.command,
                i.pointer.interact_pos(),
                i.pointer.primary_down(),
                i.pointer.press_origin(),
            )
        });
        if self.polygon_pen.pull.is_some() {
            self.pull_edge(doc, editor, escape, pointer, down);
            return true;
        }
        self.polygon_pen.hover = response
            .hover_pos()
            .and_then(|at| self.pen_place(rect, node, at, snap))
            .map(|(_, world)| world);
        if ctrl && response.drag_started_by(PointerButton::Primary) {
            if let Some(at) = origin {
                self.start_pull(rect, doc, node, at, snap);
            }
            return true;
        }
        if escape {
            self.polygon_pen.corners.clear();
        }
        if backspace {
            self.polygon_pen.corners.pop();
        }
        if response.clicked()
            && let Some(at) = response.interact_pointer_pos()
        {
            if self.polygon_pen.corners.len() >= 3 && self.near_first(rect, at, snap) {
                let corners = self.polygon_pen.corners.drain(..).map(|(c, _)| c).collect();
                editor.commands.push(EditorCommand::PenPolygon { node, corners });
            } else if let Some(corner) = self.pen_place(rect, node, at, snap) {
                self.polygon_pen.corners.push(corner);
            }
        }
        true
    }

    /// Whether `at` (screen) is on the first corner.
    fn near_first(&self, rect: Rect, at: Pos2, snap: f32) -> bool {
        let view_proj = self.camera.view_projection(rect.aspect_ratio());
        self.polygon_pen
            .corners
            .first()
            .and_then(|&(_, world)| project(&view_proj, rect, world))
            .is_some_and(|first| first.distance(at) <= snap.max(metrics::COMPONENT_PICK_RADIUS))
    }

    /// The corner a click at `at` places: an existing point or a point on an
    /// edge within `snap` points, else a point on the surface under it, else
    /// in the plane through the object's centre facing the view.
    fn pen_place(&self, rect: Rect, node: NodeId, at: Pos2, snap: f32) -> Option<(PenCorner, Vec3)> {
        let (shown, mesh, _) = self.evaluated.polygon(node)?;
        let world = shown.world;
        let view = self.screen_view(rect);
        let cursor = local(at, rect);
        let points = mesh.mesh().points();
        if let Some(ComponentHit::Point(p)) =
            pick_component(&self.evaluated, node, Component::Point, &view, cursor, snap)
        {
            let position = *points.get(usize::try_from(p).ok()?)?;
            return Some((PenCorner::Point(p), world.transform_point3(position)));
        }
        if let Some(ComponentHit::Edge(e)) = pick_component(&self.evaluated, node, Component::Edge, &view, cursor, snap)
        {
            let edge = usize::try_from(e).ok()?;
            let [a, b] = *mesh.topology().edges().get(edge)?;
            let (a, b) = (points[a as usize], points[b as usize]);
            let view_proj = self.camera.view_projection(rect.aspect_ratio());
            let (sa, sb) = (
                project(&view_proj, rect, world.transform_point3(a))?,
                project(&view_proj, rect, world.transform_point3(b))?,
            );
            let along = sb - sa;
            let t = ((at - sa).dot(along) / along.length_sq().max(1e-6)).clamp(0.05, 0.95);
            return Some((PenCorner::Edge { edge, t }, world.transform_point3(a.lerp(b, t))));
        }
        let ray = self.camera.ray_from_ndc(to_ndc(at, rect), rect.aspect_ratio());
        let into = world.inverse();
        let (origin, dir) = (into.transform_point3(ray.origin), into.transform_vector3(ray.dir));
        if let Some(hit) = mesh.raycast(origin, dir) {
            let local = origin + dir * hit.t;
            return Some((PenCorner::New(local), world.transform_point3(local)));
        }
        let centre = world.transform_point3(mesh.bounds().map_or(Vec3::ZERO, |b| b.center()));
        let normal = self.camera.forward();
        let along = ray.dir.dot(normal);
        if along.abs() <= 1e-6 {
            return None;
        }
        let t = (centre - ray.origin).dot(normal) / along;
        let point = ray.origin + ray.dir * t;
        (t > 0.0).then(|| (PenCorner::New(into.transform_point3(point)), point))
    }

    /// Starts pulling the border edge under `at`, if there is one.
    fn start_pull(&mut self, rect: Rect, doc: &Document, node: NodeId, at: Pos2, snap: f32) {
        let view = self.screen_view(rect);
        let Some(ComponentHit::Edge(e)) =
            pick_component(&self.evaluated, node, Component::Edge, &view, local(at, rect), snap)
        else {
            return;
        };
        let Some((shown, mesh, _)) = self.evaluated.polygon(node) else {
            return;
        };
        let Ok(edge) = usize::try_from(e) else { return };
        if mesh.topology().edge_faces(edge).len() != 1 {
            return; // only open borders
        }
        let centre = shown
            .world
            .transform_point3(mesh.bounds().map_or(Vec3::ZERO, |b| b.center()));
        let depth = (centre - self.camera.eye())
            .dot(self.camera.forward())
            .max(self.camera.near);
        let Some(before) = doc.scene.get(node).map(|n| n.data.clone()) else {
            return;
        };
        self.polygon_pen.pull = Some(Pull {
            node,
            before,
            edge,
            start: at,
            per_point: self.camera.world_units_per_pixel(depth, rect.height()),
        });
    }

    /// One frame of pulling an edge: the quad rebuilt from the mesh at the
    /// press; on release one undo step.
    fn pull_edge(
        &mut self,
        doc: &mut Document,
        editor: &mut EditorState,
        escape: bool,
        pointer: Option<Pos2>,
        down: bool,
    ) {
        let Some(pull) = &self.polygon_pen.pull else { return };
        if escape {
            if let Some(node) = doc.scene.get_mut(pull.node) {
                node.data = pull.before.clone();
            }
            self.polygon_pen.pull = None;
            return;
        }
        if let Some(pointer) = pointer
            && let ObjectKind::Polygon { mesh, components } = &pull.before.kind
        {
            let offset = (pointer.x - pull.start.x).abs() * pull.per_point;
            let edges: BitSet = [pull.edge].into_iter().collect();
            let options = EdgeExtrudeOptions {
                offset,
                rotation: 0.0,
                subdivision: 0,
                create_ngons: false,
            };
            if offset > 0.0
                && let Ok(pulled) = ops::extrude_edges(mesh.mesh(), mesh.topology(), &edges, &options)
                && let Some(node) = doc.scene.get_mut(pull.node)
            {
                let mut state = components.state().clone();
                state.selected.edges = pulled.outer_corners.iter().map(|&c| c as usize).collect();
                node.data.kind = ObjectKind::Polygon {
                    mesh: MeshHandle::new(pulled.mesh),
                    components: Components::new(state),
                };
            }
        }
        if !down && let Some(pull) = self.polygon_pen.pull.take() {
            editor.commands.push(EditorCommand::CommitLive {
                inverse: SceneEdit::SetData {
                    id: pull.node,
                    data: pull.before,
                },
                label: Tool::PolygonPen.label(),
            });
        }
    }

    /// The corners so far, the line to the cursor and the closing line.
    pub(super) fn paint_polygon_pen(&self, painter: &Painter, rect: Rect, editor: &EditorState) {
        if editor.tool != Tool::PolygonPen || self.polygon_pen.corners.is_empty() {
            return;
        }
        let view_proj = self.camera.view_projection(rect.aspect_ratio());
        let screen = |q: Vec3| project(&view_proj, rect, q);
        let line = Stroke::new(1.5, p().highlight);
        let corners: Vec<Pos2> = self
            .polygon_pen
            .corners
            .iter()
            .filter_map(|&(_, world)| screen(world))
            .collect();
        for pair in corners.windows(2) {
            painter.line_segment([pair[0], pair[1]], line);
        }
        if let (Some(&last), Some(to)) = (corners.last(), self.polygon_pen.hover.and_then(screen)) {
            painter.line_segment([last, to], Stroke::new(1.0, p().text_dim));
        }
        for (i, &at) in corners.iter().enumerate() {
            let size = if i == 0 { 8.0 } else { 6.0 };
            painter.rect_filled(Rect::from_center_size(at, vec2(size, size)), 0.0, p().accent);
        }
    }
}
