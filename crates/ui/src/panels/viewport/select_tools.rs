//! Selection tools in the viewport: Live Selection strokes, Rectangle Selection
//! frames, Loop and Ring selection (with a preview under the cursor), and the
//! double-click shortcuts of the transform tools in component modes.

use egui::{Key, Modifiers, PointerButton, Pos2, Rect, Response, Ui};
use grid4d_eval::PickShape;
use grid4d_geom::{
    BitSet, Component, loops,
    selection::{self, corners_from_edges, edges_from_corners},
};
use grid4d_render::{
    Brush, BrushCoverage, ComponentHit, RegionRules, components_in_rect, nearest_face_edge, paint_brush, pick_component,
};
use grid4d_scene::{Document, NodeData, NodeId, ObjectKind, SceneEdit};

use super::{
    ViewportPanel,
    gizmo::{PointEdit, centre_of, move_label},
    local, project,
};
use crate::{EditMode, EditorCommand, EditorState, SelectMode, Tool, theme::metrics};

/// A Live Selection stroke being painted.
#[derive(Debug)]
pub(super) struct Stroke {
    node: NodeId,
    component: Component,
    mode: SelectMode,
    /// The object as it was before the stroke: the undo step, and what Esc restores.
    before: NodeData,
    /// The selection the stroke adds to or removes from (stored form).
    base: BitSet,
    coverage: BrushCoverage,
}

/// What the pointer is doing with a selection tool.
#[derive(Debug, Default)]
pub(super) enum Gesture {
    #[default]
    None,
    Stroke(Stroke),
    /// A Rectangle Selection frame, from where the drag started.
    Frame {
        start: Pos2,
        end: Pos2,
    },
    /// Tweak: one component dragged in the view plane, the selection untouched.
    Tweak {
        node: NodeId,
        component: Component,
        before: NodeData,
        edit: PointEdit,
        last: Pos2,
    },
    /// A modelling tool's drag (Extrude, Inset).
    MeshTool(super::mesh_tools::MeshDrag),
    /// A Line Cut being drawn on polygon object `node`.
    LineCut {
        node: NodeId,
        start: Pos2,
        end: Pos2,
    },
    /// Esc cancelled the gesture: ignore the pointer until it is released.
    Cancelled,
}

impl ViewportPanel {
    /// Handles the selection tools' pointer input. Returns `true` if it used the
    /// pointer this frame, so the click is not also treated as a plain pick.
    pub(super) fn selection_tools(
        &mut self,
        ui: &Ui,
        response: &Response,
        rect: Rect,
        doc: &mut Document,
        editor: &mut EditorState,
    ) -> bool {
        let (pressed, down, escape, modifiers, pointer, origin) = ui.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.key_pressed(Key::Escape),
                i.modifiers,
                i.pointer.interact_pos(),
                i.pointer.press_origin(),
            )
        });
        if escape {
            match std::mem::take(&mut self.gesture) {
                Gesture::Stroke(stroke) => {
                    if let Some(node) = doc.scene.get_mut(stroke.node) {
                        node.data = stroke.before;
                    }
                    self.gesture = Gesture::Cancelled;
                }
                Gesture::Tweak { node, before, .. } => {
                    if let Some(node) = doc.scene.get_mut(node) {
                        node.data = before;
                    }
                    self.gesture = Gesture::Cancelled;
                }
                Gesture::Frame { .. } => self.gesture = Gesture::Cancelled,
                other => self.gesture = other,
            }
        }
        if matches!(self.gesture, Gesture::Cancelled) {
            if !down {
                self.gesture = Gesture::None;
            }
            return true;
        }

        let target = self.component_target(editor);
        match (editor.tool, target) {
            (Tool::LiveSelection, Some((node, component))) => self.live_selection(
                response, rect, doc, editor, node, component, pressed, down, modifiers, pointer,
            ),
            (Tool::RectangleSelection, _) => {
                if response.drag_started_by(PointerButton::Primary)
                    && !modifiers.alt
                    && let (Some(start), Some(end)) = (origin, pointer)
                {
                    self.gesture = Gesture::Frame { start, end };
                }
                let Gesture::Frame { start, end } = &mut self.gesture else {
                    return false;
                };
                if let Some(pointer) = pointer {
                    *end = pointer;
                }
                if !down {
                    let frame = Rect::from_two_pos(*start, *end);
                    self.gesture = Gesture::None;
                    self.select_in_frame(rect, frame, editor, target, modifiers);
                }
                true
            }
            (Tool::LoopSelection | Tool::RingSelection, Some((node, component))) => {
                if !response.clicked_by(PointerButton::Primary) || modifiers.alt {
                    return false;
                }
                if let Some(pointer) = pointer {
                    let hits = self.loop_hits(rect, node, component, editor, local(pointer, rect));
                    editor.commands.push(EditorCommand::SelectComponents {
                        node,
                        component,
                        hits,
                        mode: SelectMode::for_components(modifiers),
                    });
                }
                true
            }
            (Tool::Move, Some((node, component)))
                if matches!(self.gesture, Gesture::Tweak { .. })
                    || (editor.transform_settings.tweak
                        && response.drag_started_by(PointerButton::Primary)
                        && !modifiers.alt
                        && nothing_selected(doc, node, component)) =>
            {
                self.tweak(rect, doc, editor, node, component, origin, pointer, down)
            }
            (Tool::Move | Tool::Scale | Tool::Rotate, Some((node, component)))
                if response.double_clicked_by(PointerButton::Primary) && !modifiers.alt =>
            {
                // Double-click: an edge's loop (a border edge: the whole border), a polygon's island.
                let Some(pointer) = pointer else { return false };
                let view = self.screen_view(rect);
                let radius = metrics::COMPONENT_PICK_RADIUS;
                let picked = pick_component(&self.evaluated, node, component, &view, local(pointer, rect), radius);
                let Some((_, handle, components)) = self.evaluated.polygon(node) else {
                    return false;
                };
                let (mesh, topology) = (handle.mesh(), handle.topology());
                let hits: Vec<u32> = match picked {
                    Some(ComponentHit::Edge(e)) => loops::edge_loop(
                        mesh,
                        topology,
                        idx(e),
                        editor.selection_settings.loop_options,
                        &edges_from_corners(&components.selected().edges, topology),
                        &BitSet::new(),
                    )
                    .iter()
                    .map(index_u32)
                    .collect(),
                    Some(ComponentHit::Polygon(f)) => {
                        let seed: BitSet = [idx(f)].into_iter().collect();
                        selection::connected(mesh, topology, &seed, Component::Polygon)
                            .iter()
                            .map(index_u32)
                            .collect()
                    }
                    Some(ComponentHit::Point(_)) | None => return false,
                };
                editor.commands.push(EditorCommand::SelectComponents {
                    node,
                    component,
                    hits,
                    mode: SelectMode::for_components(modifiers),
                });
                true
            }
            _ => false,
        }
    }

    /// Live Selection: paints from the press on, writing the selection live and
    /// recording one undo step when the button is released.
    #[expect(
        clippy::too_many_arguments,
        reason = "one call site; the pieces of one frame's input"
    )]
    fn live_selection(
        &mut self,
        response: &Response,
        rect: Rect,
        doc: &mut Document,
        editor: &mut EditorState,
        node: NodeId,
        component: Component,
        pressed: bool,
        down: bool,
        modifiers: Modifiers,
        pointer: Option<Pos2>,
    ) -> bool {
        if pressed && !modifiers.alt && response.hovered() && matches!(self.gesture, Gesture::None) {
            let Some(data) = doc.scene.get(node).map(|n| n.data.clone()) else {
                return false;
            };
            let ObjectKind::Polygon { components, .. } = &data.kind else {
                return false;
            };
            let mode = SelectMode::for_components(modifiers);
            let base = match mode {
                SelectMode::Replace => BitSet::new(),
                SelectMode::Add | SelectMode::Remove | SelectMode::Toggle => {
                    components.selected().get(component).clone()
                }
            };
            self.gesture = Gesture::Stroke(Stroke {
                node,
                component,
                mode,
                before: data,
                base,
                coverage: BrushCoverage::default(),
            });
        }
        if !matches!(self.gesture, Gesture::Stroke(_)) {
            return false;
        }
        if let Some(pointer) = pointer {
            self.paint(rect, doc, editor, pointer);
        }
        if !down && let Gesture::Stroke(stroke) = std::mem::take(&mut self.gesture) {
            editor.commands.push(EditorCommand::CommitLive {
                inverse: SceneEdit::SetData {
                    id: stroke.node,
                    data: stroke.before,
                },
                label: "Live Selection",
            });
        }
        true
    }

    /// Tweak: starts on the component under the press, moves it in the view plane
    /// at its own depth, and records one undo step on release.
    #[expect(
        clippy::too_many_arguments,
        reason = "one call site; the pieces of one frame's input"
    )]
    fn tweak(
        &mut self,
        rect: Rect,
        doc: &mut Document,
        editor: &mut EditorState,
        node: NodeId,
        component: Component,
        origin: Option<Pos2>,
        pointer: Option<Pos2>,
        down: bool,
    ) -> bool {
        if !matches!(self.gesture, Gesture::Tweak { .. }) {
            let Some(press) = origin else { return false };
            let view = self.screen_view(rect);
            let radius = metrics::COMPONENT_PICK_RADIUS;
            let Some(hit) = pick_component(&self.evaluated, node, component, &view, local(press, rect), radius) else {
                return false; // not on a component: an ordinary drag
            };
            let Some(before) = doc.scene.get(node).map(|n| n.data.clone()) else {
                return false;
            };
            let ObjectKind::Polygon { mesh, .. } = &before.kind else {
                return false;
            };
            let one: BitSet = match hit {
                ComponentHit::Edge(e) => mesh.topology().edge_corners(idx(e)).iter().map(|&c| idx(c)).collect(),
                ComponentHit::Point(i) | ComponentHit::Polygon(i) => [idx(i)].into_iter().collect(),
            };
            let points = selection::points_of(mesh.mesh(), &one, component);
            let world = doc.scene.world_matrix(node);
            let Some(centre) = centre_of(mesh, &points) else {
                return false;
            };
            let edit = PointEdit::new(mesh, &points, world, world.transform_point3(centre));
            self.gesture = Gesture::Tweak {
                node,
                component,
                before,
                edit,
                last: press,
            };
        }
        let camera = &self.camera;
        let Gesture::Tweak { node, edit, last, .. } = &mut self.gesture else {
            return false;
        };
        if let Some(pointer) = pointer
            && pointer != *last
        {
            let depth = (edit.pivot - camera.eye()).dot(camera.forward()).max(camera.near);
            let per_point = camera.world_units_per_pixel(depth, rect.height());
            let d = pointer - *last;
            // Screen y points down.
            edit.moved += (camera.right() * d.x - camera.up() * d.y) * per_point;
            edit.apply(glam::Mat4::from_translation(edit.moved), &mut doc.scene, *node);
            *last = pointer;
        }
        if !down
            && let Gesture::Tweak {
                node,
                component,
                before,
                ..
            } = std::mem::take(&mut self.gesture)
        {
            editor.commands.push(EditorCommand::CommitLive {
                inverse: SceneEdit::SetData { id: node, data: before },
                label: move_label(component),
            });
        }
        true
    }

    /// One brush stamp of the current stroke, written to the document.
    fn paint(&mut self, rect: Rect, doc: &mut Document, editor: &EditorState, pointer: Pos2) {
        let view = self.screen_view(rect);
        let settings = editor.selection_settings;
        let Gesture::Stroke(stroke) = &mut self.gesture else {
            return;
        };
        paint_brush(
            &self.evaluated,
            stroke.node,
            stroke.component,
            &view,
            Brush {
                centre: local(pointer, rect),
                radius: settings.radius,
                visible_only: settings.live_visible_only,
            },
            &mut stroke.coverage,
        );
        let Some(node) = doc.scene.get(stroke.node) else { return };
        let ObjectKind::Polygon { mesh, components } = &node.data.kind else {
            return;
        };
        let painted: BitSet = stroke.coverage.painted(settings.live_tolerant).map(idx).collect();
        let painted = match stroke.component {
            Component::Edge => corners_from_edges(&painted, mesh.topology()),
            Component::Point | Component::Polygon => painted,
        };
        let mut bits = stroke.base.clone();
        match stroke.mode {
            SelectMode::Remove | SelectMode::Toggle => bits.remove_all(&painted),
            SelectMode::Replace | SelectMode::Add => bits.extend(painted.iter()),
        }
        if bits == *components.selected().get(stroke.component) {
            return;
        }
        let (mesh, mut components) = (mesh.clone(), components.clone());
        *components.make_mut().selected.get_mut(stroke.component) = bits;
        if let Some(node) = doc.scene.get_mut(stroke.node) {
            node.data.kind = ObjectKind::Polygon { mesh, components };
        }
    }

    /// The end of a Rectangle Selection drag: components inside the frame, or —
    /// in Model mode — the objects whose origin is inside it.
    fn select_in_frame(
        &self,
        rect: Rect,
        frame: Rect,
        editor: &mut EditorState,
        target: Option<(NodeId, Component)>,
        modifiers: Modifiers,
    ) {
        let mode = SelectMode::for_components(modifiers);
        if let Some((node, component)) = target {
            let settings = editor.selection_settings;
            let hits = components_in_rect(
                &self.evaluated,
                node,
                component,
                &self.screen_view(rect),
                [local(frame.min, rect), local(frame.max, rect)],
                RegionRules {
                    tolerant: settings.rectangle_tolerant,
                    visible_only: settings.rectangle_visible_only,
                },
            );
            editor.commands.push(EditorCommand::SelectComponents {
                node,
                component,
                hits,
                mode,
            });
        } else if editor.mode == EditMode::Model {
            let view_proj = self.camera.view_projection(rect.aspect_ratio());
            if mode == SelectMode::Replace {
                editor.commands.push(EditorCommand::DeselectAll);
            }
            for node in self.evaluated.nodes() {
                if node.pick != PickShape::None
                    && project(&view_proj, rect, node.world.w_axis.truncate()).is_some_and(|p| frame.contains(p))
                {
                    let mode = if mode == SelectMode::Remove {
                        SelectMode::Remove
                    } else {
                        SelectMode::Add
                    };
                    editor.commands.push(EditorCommand::Select { id: node.id, mode });
                }
            }
        }
    }

    /// What Loop or Ring Selection would select at `cursor` (points, unique edges
    /// or polygons): through the edge under the cursor, or — in Polygons mode —
    /// through the polygon under it, along the direction of its nearest edge.
    pub(super) fn loop_hits(
        &self,
        rect: Rect,
        node: NodeId,
        component: Component,
        editor: &EditorState,
        cursor: glam::Vec2,
    ) -> Vec<u32> {
        let view = self.screen_view(rect);
        let Some((_, handle, components)) = self.evaluated.polygon(node) else {
            return Vec::new();
        };
        let (mesh, topology) = (handle.mesh(), handle.topology());
        let selected = components.selected();
        let ring = editor.tool == Tool::RingSelection;
        let settings = editor.selection_settings;
        let options = if ring {
            settings.ring_options
        } else {
            settings.loop_options
        };
        let none = BitSet::new();
        let found: BitSet = match component {
            Component::Point | Component::Edge => {
                let radius = metrics::COMPONENT_PICK_RADIUS * 2.0;
                let Some(ComponentHit::Edge(e)) =
                    pick_component(&self.evaluated, node, Component::Edge, &view, cursor, radius)
                else {
                    return Vec::new();
                };
                // Fences for "stop at selections": what is selected in this mode.
                let selected_edges = match component {
                    Component::Edge => edges_from_corners(&selected.edges, topology),
                    Component::Point | Component::Polygon => (0..topology.edge_count())
                        .filter(|&e| topology.edges()[e].iter().all(|&p| selected.points.contains(idx(p))))
                        .collect(),
                };
                let stop_points = if component == Component::Point {
                    &selected.points
                } else {
                    &none
                };
                let edges = if ring {
                    loops::edge_ring(mesh, topology, idx(e), options, &selected_edges, &none).edges
                } else {
                    loops::edge_loop(mesh, topology, idx(e), options, &selected_edges, stop_points)
                };
                match component {
                    Component::Point => edges.iter().flat_map(|e| topology.edges()[e]).map(idx).collect(),
                    Component::Edge | Component::Polygon => edges,
                }
            }
            Component::Polygon => {
                let Some(ComponentHit::Polygon(f)) =
                    pick_component(&self.evaluated, node, Component::Polygon, &view, cursor, 0.0)
                else {
                    return Vec::new();
                };
                let Some(e) = nearest_face_edge(&self.evaluated, node, f, &view, cursor) else {
                    return Vec::new();
                };
                // Loop: the strip running across the nearest edge; Ring: the one along it.
                let edge = if ring {
                    let corners = mesh.corner_range(idx(f));
                    let n = corners.len();
                    corners
                        .clone()
                        .position(|c| topology.corner_edge(c) == Some(e))
                        .and_then(|i| topology.corner_edge(corners.start + (i + 1) % n))
                        .unwrap_or(e)
                } else {
                    e
                };
                let mut faces = loops::edge_ring(mesh, topology, idx(edge), options, &none, &selected.polygons).faces;
                faces.insert(idx(f));
                faces
            }
        };
        found.iter().map(index_u32).collect()
    }
}

#[cfg(test)]
impl ViewportPanel {
    pub(super) fn gesture_is_idle(&self) -> bool {
        matches!(self.gesture, Gesture::None)
    }
}

/// Nothing of kind `component` is selected on polygon object `node`.
fn nothing_selected(doc: &Document, node: NodeId, component: Component) -> bool {
    doc.scene.get(node).is_some_and(|n| match &n.data.kind {
        ObjectKind::Polygon { components, .. } => components.selected().get(component).is_empty(),
        _ => false,
    })
}

fn idx(index: u32) -> usize {
    usize::try_from(index).unwrap_or(usize::MAX)
}

fn index_u32(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}
