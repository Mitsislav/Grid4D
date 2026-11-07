//! The UV Editor (see `docs/reference/manager-uv-editor.md`): the active
//! polygon object's UVs as a flat mesh over its material's picture. The
//! selection is the object's own (shared with the viewport): clicks, frames
//! and double-clicks push `EditorCommand::SelectComponents`. The Move, Scale
//! and Rotate tools show a 2D handle at the selection; a drag writes the UVs
//! live and records one step when it ends (`EditorCommand::CommitLive`). The
//! UV menu pushes `EditorCommand::UvCommand`s.

use egui::{
    Color32, CornerRadius, PointerButton, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, TextureHandle,
    TextureOptions, Ui, pos2, vec2,
};
use glam::Vec2;
use grid4d_geom::{BitSet, Component, PolyMesh, Topology, uv};
use grid4d_scene::{Channel, ComponentState, Document, MeshHandle, NodeData, NodeId, ObjectKind, SceneEdit, TextureId};

use crate::i18n::{key, tr};
use crate::{
    EditMode, EditorCommand, EditorState, FrameCurves, SelectMode, Tool,
    textures::Status,
    theme::{metrics, palette as p},
    widgets::header_row,
};

/// Points and handles are hit within this many points; edges a little less.
const PICK: f32 = 7.0;
const EDGE_PICK: f32 = 5.0;
/// The move and scale handles' length, and the rotate ring's radius.
const HANDLE: f32 = 56.0;

/// What the editor looks at: UV position of the canvas's centre and points
/// per UV unit.
#[derive(Clone, Copy, Debug, PartialEq)]
struct View {
    centre: Vec2,
    zoom: f32,
}

impl Default for View {
    fn default() -> Self {
        Self {
            centre: Vec2::splat(0.5),
            zoom: 200.0,
        }
    }
}

impl View {
    fn screen(&self, canvas: Rect, uv: Vec2) -> Pos2 {
        let at = (uv - self.centre) * self.zoom;
        canvas.center() + vec2(at.x, at.y)
    }

    fn uv(&self, canvas: Rect, at: Pos2) -> Vec2 {
        let d = at - canvas.center();
        self.centre + Vec2::new(d.x, d.y) / self.zoom
    }

    /// Shows the box `min` – `max` with a little room around it.
    fn fit(&mut self, canvas: Rect, min: Vec2, max: Vec2) {
        let size = (max - min).max(Vec2::splat(1e-3));
        self.centre = (min + max) * 0.5;
        self.zoom = (canvas.width() / (size.x * 1.15))
            .min(canvas.height() / (size.y * 1.15))
            .max(1.0);
    }
}

/// Which part of the 2D handle a drag holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handle {
    /// The centre: free (Move), uniform (Scale); the ring (Rotate).
    Free,
    U,
    V,
}

/// A transform drag in progress.
struct Transform {
    node: NodeId,
    /// The object as it was: the undo step's inverse, and what each frame
    /// starts from.
    before: NodeData,
    mesh: MeshHandle,
    uvs: Vec<Vec2>,
    corners: Vec<usize>,
    pivot: Vec2,
    tool: Tool,
    handle: Handle,
    start: Pos2,
}

enum Drag {
    /// A selection frame from this point.
    Frame(Pos2),
    Transform(Box<Transform>),
}

/// The shown object's UVs, worked out once per mesh.
struct Shape2 {
    mesh: MeshId,
    uvs: Vec<Vec2>,
    layout: uv::UvLayout,
    stretch: Option<Vec<f32>>,
}

type MeshId = grid4d_scene::MeshId;

/// The UV Editor pane.
#[derive(Default)]
pub struct UvEditorPanel {
    view: View,
    /// The view has been fitted to the square once.
    framed: bool,
    drag: Option<Drag>,
    /// The picture behind the UVs, as uploaded (its texture and version).
    picture: Option<(TextureId, u64, TextureHandle)>,
    hide_picture: bool,
    /// Colours polygons by how much they are stretched.
    pub stretch: bool,
    shape: Option<Shape2>,
    /// A primitive's mesh shown read only, by node and revision.
    read_only: Option<(NodeId, u64, MeshHandle)>,
    /// Where the UVs were drawn last frame (tests click there).
    canvas: Option<Rect>,
}

impl std::fmt::Debug for UvEditorPanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UvEditorPanel")
            .field("view", &self.view)
            .finish_non_exhaustive()
    }
}

/// The object the editor shows: an editable polygon object, or a primitive
/// (read only).
struct Target {
    id: NodeId,
    mesh: MeshHandle,
    state: ComponentState,
    editable: bool,
}

impl UvEditorPanel {
    /// Where UV `uv` is drawn (for tests); `None` before the first frame.
    #[must_use]
    pub fn screen_position(&self, uv: Vec2) -> Option<Pos2> {
        self.canvas.map(|canvas| self.view.screen(canvas, uv))
    }

    pub fn show(&mut self, ui: &mut Ui, doc: &mut Document, editor: &mut EditorState) {
        ui.spacing_mut().item_spacing.y = 0.0;
        let target = self.target(doc, editor);
        let header = header_row(ui);
        self.header(ui, header, editor, target.as_ref());
        let canvas = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(canvas, Sense::click_and_drag());
        self.canvas = Some(canvas);
        if !self.framed && canvas.width() > 10.0 {
            self.view.fit(canvas, Vec2::ZERO, Vec2::ONE);
            self.framed = true;
        }
        let painter = ui.painter_at(canvas);
        painter.rect_filled(canvas, CornerRadius::ZERO, p().bg_well);

        let Some(target) = target else {
            self.drag = None;
            painter.text(
                canvas.center(),
                egui::Align2::CENTER_CENTER,
                tr("Select a polygon object to see its UVs"),
                egui::FontId::proportional(13.0),
                p().text_dim,
            );
            return;
        };
        self.update_shape(&target);
        if let Some(request) = editor.uv_frame.take() {
            self.frame(canvas, &target, editor, request == FrameCurves::Selected);
        }
        self.navigate(ui, &response, canvas);
        self.backdrop(ui, &painter, canvas, doc, editor, target.id);
        if target.editable {
            self.interact(ui, &response, canvas, doc, editor, &target);
        }
        self.paint(&painter, canvas, editor, &target);
        if !target.editable {
            painter.text(
                canvas.left_top() + vec2(8.0, 8.0),
                egui::Align2::LEFT_TOP,
                tr("Read only: make the object editable (C) to change its UVs"),
                egui::FontId::proportional(12.0),
                p().text_dim,
            );
        }
        if response.hovered() {
            editor.status_hint = Some(
                "UV Editor: click or drag a frame to select · Shift adds · Ctrl removes · double-click an island · Move / Scale / Rotate drag the handle · wheel zooms · middle drag pans · H / S frame",
            );
        }
    }

    fn target(&mut self, doc: &Document, editor: &EditorState) -> Option<Target> {
        let id = editor.selection.active()?;
        let node = doc.scene.get(id)?;
        match &node.data.kind {
            ObjectKind::Polygon { mesh, components } => Some(Target {
                id,
                mesh: mesh.clone(),
                state: components.state().clone(),
                editable: true,
            }),
            kind => {
                let revision = node.revision();
                let cached = self
                    .read_only
                    .as_ref()
                    .filter(|(node, seen, _)| *node == id && *seen == revision)
                    .map(|(_, _, mesh)| mesh.clone());
                let mesh = match cached {
                    Some(mesh) => mesh,
                    None => {
                        let mesh = MeshHandle::new(grid4d_eval::editable_mesh(kind)?);
                        self.read_only = Some((id, revision, mesh.clone()));
                        mesh
                    }
                };
                Some(Target {
                    id,
                    mesh,
                    state: ComponentState::default(),
                    editable: false,
                })
            }
        }
    }

    fn update_shape(&mut self, target: &Target) {
        let id = target.mesh.id();
        if self.shape.as_ref().is_none_or(|s| s.mesh != id) {
            let mesh = target.mesh.mesh();
            self.shape = Some(Shape2 {
                mesh: id,
                uvs: mesh.uvs().map_or_else(|| uv::box_uvs(mesh), <[Vec2]>::to_vec),
                layout: uv::layout(mesh, target.mesh.topology()),
                stretch: None,
            });
        }
        if self.stretch
            && let Some(shape) = &mut self.shape
            && shape.stretch.is_none()
        {
            shape.stretch = Some(uv::face_stretch(target.mesh.mesh(), target.mesh.topology()));
        }
    }

    fn header(&mut self, ui: &mut Ui, header: Rect, editor: &mut EditorState, target: Option<&Target>) {
        let editable = target.is_some_and(|t| t.editable);
        ui.scope_builder(egui::UiBuilder::new().max_rect(header.shrink2(vec2(4.0, 1.0))), |ui| {
            ui.horizontal_centered(|ui| {
                ui.menu_button(tr("UV"), |ui| crate::widgets::uv_menu(ui, editor, editable));
                ui.menu_button(tr("View"), |ui| {
                    let mut shown = !self.hide_picture;
                    if ui.checkbox(&mut shown, tr("Show Texture")).changed() {
                        self.hide_picture = !shown;
                    }
                    if ui.checkbox(&mut self.stretch, tr("Show Stretch")).changed() {
                        ui.close();
                    }
                    ui.separator();
                    if ui.add(egui::Button::new(tr("Frame All")).shortcut_text("H")).clicked() {
                        editor.uv_frame = Some(FrameCurves::All);
                        ui.close();
                    }
                    if ui
                        .add(egui::Button::new(tr("Frame Selected")).shortcut_text("S"))
                        .clicked()
                    {
                        editor.uv_frame = Some(FrameCurves::Selected);
                        ui.close();
                    }
                });
                ui.menu_button(tr("Options"), |ui| options(ui, editor));
            });
        });
    }

    /// Fits the view to every UV (or the selection's).
    fn frame(&mut self, canvas: Rect, target: &Target, editor: &EditorState, selected: bool) {
        let Some(shape) = &self.shape else { return };
        let corners: Vec<usize> = if selected {
            selected_corners(
                target.mesh.mesh(),
                target.mesh.topology(),
                &target.state,
                component(editor),
                &shape.layout,
            )
        } else {
            (0..shape.uvs.len()).collect()
        };
        let mut min = Vec2::splat(f32::INFINITY);
        let mut max = Vec2::splat(f32::NEG_INFINITY);
        for &c in &corners {
            if let Some(&uv) = shape.uvs.get(c) {
                min = min.min(uv);
                max = max.max(uv);
            }
        }
        if min.is_finite() {
            if !selected {
                // Always the whole square too.
                min = min.min(Vec2::ZERO);
                max = max.max(Vec2::ONE);
            }
            self.view.fit(canvas, min, max);
        }
    }

    fn navigate(&mut self, ui: &Ui, response: &Response, canvas: Rect) {
        let alt = ui.input(|i| i.modifiers.alt);
        if response.dragged_by(PointerButton::Middle) || (alt && response.dragged_by(PointerButton::Primary)) {
            let d = response.drag_delta();
            self.view.centre -= Vec2::new(d.x, d.y) / self.view.zoom;
        }
        let Some(at) = response.hover_pos() else { return };
        let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
        let amount = scroll.y + scroll.x;
        let factor = if amount != 0.0 { (amount * 0.002).exp() } else { zoom };
        if factor != 1.0 {
            let before = self.view.uv(canvas, at);
            self.view.zoom = (self.view.zoom * factor).clamp(5.0, 200_000.0);
            let after = self.view.uv(canvas, at);
            self.view.centre += before - after;
        }
    }

    /// The material's picture in the square, the square's outline and the
    /// grid of whole UV units.
    fn backdrop(
        &mut self,
        ui: &Ui,
        painter: &egui::Painter,
        canvas: Rect,
        doc: &Document,
        editor: &EditorState,
        id: NodeId,
    ) {
        let square = Rect::from_two_pos(
            self.view.screen(canvas, Vec2::ZERO),
            self.view.screen(canvas, Vec2::ONE),
        );
        painter.rect_filled(square, CornerRadius::ZERO, p().bg_raised);
        // The Base Color picture of the object's material, where its tiling
        // and offset put it.
        let slot = doc
            .scene
            .effective_material(id)
            .and_then(|m| doc.scene.materials().get(m))
            .and_then(|m| m.map(Channel::BaseColor).copied());
        if !self.hide_picture
            && let Some(slot) = slot
            && let Some(texture) = doc.scene.textures().get(slot.texture)
            && let Status::Ready { image, version } = editor.textures.status(slot.texture, texture)
        {
            let current = self
                .picture
                .as_ref()
                .is_some_and(|(id, seen, _)| *id == slot.texture && *seen == version);
            if !current && let Some(level) = image.level_within(2048) {
                let picture = egui::ColorImage::from_rgba_unmultiplied(
                    [
                        usize::try_from(level.width).unwrap_or(1),
                        usize::try_from(level.height).unwrap_or(1),
                    ],
                    &level.to_srgb8(texture.color_space),
                );
                let options = TextureOptions {
                    wrap_mode: egui::TextureWrapMode::Repeat,
                    ..TextureOptions::LINEAR
                };
                let handle = ui.ctx().load_texture("grid4d.uv_editor.picture", picture, options);
                self.picture = Some((slot.texture, version, handle));
            }
            if let Some((_, _, handle)) = &self.picture {
                let [tu, tv] = slot.tiling;
                let [ou, ov] = slot.offset;
                let uv = Rect::from_min_max(pos2(ou, ov), pos2(ou + tu, ov + tv));
                // Dimmed, so the UVs drawn over it stay readable.
                painter.image(handle.id(), square, uv, Color32::from_gray(150));
            }
        }
        // Whole units: the square brighter than the tiles around it.
        let (from, to) = (self.view.uv(canvas, canvas.min), self.view.uv(canvas, canvas.max));
        let line = Stroke::new(1.0, p().separator);
        for u in (from.x.floor() as i32)..=(to.x.ceil() as i32) {
            let x = self.view.screen(canvas, Vec2::new(u as f32, 0.0)).x;
            painter.line_segment([pos2(x, canvas.top()), pos2(x, canvas.bottom())], line);
        }
        for v in (from.y.floor() as i32)..=(to.y.ceil() as i32) {
            let y = self.view.screen(canvas, Vec2::new(0.0, v as f32)).y;
            painter.line_segment([pos2(canvas.left(), y), pos2(canvas.right(), y)], line);
        }
        painter.rect_stroke(
            square,
            CornerRadius::ZERO,
            Stroke::new(1.0, p().text_dim),
            StrokeKind::Middle,
        );
    }

    fn interact(
        &mut self,
        ui: &Ui,
        response: &Response,
        canvas: Rect,
        doc: &mut Document,
        editor: &mut EditorState,
        target: &Target,
    ) {
        let Some(shape) = &self.shape else { return };
        let (modifiers, pointer, escape, released) = ui.input(|i| {
            (
                i.modifiers,
                i.pointer.interact_pos(),
                i.key_pressed(egui::Key::Escape),
                i.pointer.primary_released(),
            )
        });
        let mesh = target.mesh.mesh();
        let topology = target.mesh.topology();
        let component = component(editor);

        // A drag in progress.
        match &mut self.drag {
            Some(Drag::Transform(drag)) => {
                if escape {
                    if let Some(node) = doc.scene.get_mut(drag.node) {
                        node.data = drag.before.clone();
                    }
                    self.drag = None;
                    return;
                }
                if let Some(at) = pointer {
                    let moved = transformed(drag, at, self.view.zoom, self.view.screen(canvas, drag.pivot));
                    if let Some(handle) = drag.mesh.with_uvs(moved)
                        && let Some(node) = doc.scene.get_mut(drag.node)
                        && let ObjectKind::Polygon { mesh, .. } = &mut node.data.kind
                    {
                        *mesh = handle;
                    }
                }
                if released || !response.dragged() {
                    let label = match drag.tool {
                        Tool::Scale => key("Scale UVs"),
                        Tool::Rotate => key("Rotate UVs"),
                        _ => key("Move UVs"),
                    };
                    let changed = doc.scene.get(drag.node).is_some_and(|n| n.data != drag.before);
                    if changed {
                        editor.commands.push(EditorCommand::CommitLive {
                            inverse: SceneEdit::SetData {
                                id: drag.node,
                                data: drag.before.clone(),
                            },
                            label,
                        });
                    }
                    self.drag = None;
                }
                return;
            }
            Some(Drag::Frame(start)) => {
                let start = *start;
                if released || !response.dragged() {
                    if let Some(end) = pointer {
                        let frame = Rect::from_two_pos(start, end);
                        let hits = inside(mesh, topology, &target.state, &shape.uvs, component, |uv| {
                            frame.contains(self.view.screen(canvas, uv))
                        });
                        editor.commands.push(EditorCommand::SelectComponents {
                            node: target.id,
                            component,
                            hits,
                            mode: SelectMode::for_components(modifiers),
                        });
                    }
                    self.drag = None;
                }
                return;
            }
            None => {}
        }
        if modifiers.alt {
            return;
        }

        // A drag starts: on the handle it transforms, elsewhere it frames.
        if response.drag_started_by(PointerButton::Primary)
            && let Some(start) = response.interact_pointer_pos().or(pointer)
        {
            let corners = selected_corners(mesh, topology, &target.state, component, &shape.layout);
            let tool = editor.tool;
            let handle = if matches!(tool, Tool::Move | Tool::Scale | Tool::Rotate) && !corners.is_empty() {
                let pivot = centre(&shape.uvs, &corners);
                handle_at(tool, self.view.screen(canvas, pivot), start).map(|h| (h, pivot))
            } else {
                None
            };
            self.drag = Some(match handle {
                Some((handle, pivot)) => {
                    let before = doc.scene.get(target.id).map(|n| n.data.clone());
                    match before {
                        Some(before) => Drag::Transform(Box::new(Transform {
                            node: target.id,
                            before,
                            mesh: target.mesh.clone(),
                            uvs: shape.uvs.clone(),
                            corners,
                            pivot,
                            tool,
                            handle,
                            start,
                        })),
                        None => Drag::Frame(start),
                    }
                }
                None => Drag::Frame(start),
            });
            return;
        }

        // Clicks select; a double-click selects an island.
        let Some(at) = response.interact_pointer_pos() else {
            return;
        };
        if response.double_clicked_by(PointerButton::Primary) {
            if let Some(face) = polygon_at(mesh, &target.state, &shape.uvs, self.view.uv(canvas, at)) {
                let island = shape.layout.island_of[face];
                let hits = (0..mesh.face_count())
                    .filter(|&f| shape.layout.island_of[f] == island && !target.state.hidden.polygons.contains(f))
                    .filter_map(|f| u32::try_from(f).ok())
                    .collect();
                editor.mode = EditMode::Polygons;
                editor.commands.push(EditorCommand::SelectComponents {
                    node: target.id,
                    component: Component::Polygon,
                    hits,
                    mode: SelectMode::for_components(modifiers),
                });
            }
        } else if response.clicked_by(PointerButton::Primary) {
            let hit = match component {
                Component::Point => point_at(mesh, &target.state, &shape.uvs, |uv| self.view.screen(canvas, uv), at),
                Component::Edge => edge_at(
                    mesh,
                    topology,
                    &target.state,
                    &shape.uvs,
                    |uv| self.view.screen(canvas, uv),
                    at,
                ),
                Component::Polygon => polygon_at(mesh, &target.state, &shape.uvs, self.view.uv(canvas, at)),
            };
            editor.commands.push(EditorCommand::SelectComponents {
                node: target.id,
                component,
                hits: hit.and_then(|h| u32::try_from(h).ok()).into_iter().collect(),
                mode: SelectMode::for_components(modifiers),
            });
        }
    }

    fn paint(&self, painter: &egui::Painter, canvas: Rect, editor: &EditorState, target: &Target) {
        let Some(shape) = &self.shape else { return };
        let mesh = target.mesh.mesh();
        let topology = target.mesh.topology();
        let state = &target.state;
        let to_screen = |uv: Vec2| self.view.screen(canvas, uv);
        let component = component(editor);
        let selected_colour = p().highlight;

        // Polygons: stretch colours, then the selection.
        let mut points = Vec::new();
        for f in 0..mesh.face_count() {
            if state.hidden.polygons.contains(f) {
                continue;
            }
            let fill = if component == Component::Polygon && state.selected.polygons.contains(f) {
                Some(selected_colour.gamma_multiply(0.35))
            } else {
                shape
                    .stretch
                    .as_ref()
                    .filter(|_| self.stretch)
                    .and_then(|s| s.get(f))
                    .map(|&s| stretch_colour(s))
            };
            if let Some(fill) = fill {
                points.clear();
                points.extend(mesh.corner_range(f).map(|c| to_screen(shape.uvs[c])));
                // A fan from the first corner (UV polygons are mostly convex).
                for i in 1..points.len().saturating_sub(1) {
                    painter.add(Shape::convex_polygon(
                        vec![points[0], points[i], points[i + 1]],
                        fill,
                        Stroke::NONE,
                    ));
                }
            }
        }

        // Edges, seams and selected edges.
        let edge = Stroke::new(1.0, p().text.gamma_multiply(0.8));
        let seam = Stroke::new(2.0, p().record);
        let chosen = Stroke::new(metrics::COMPONENT_EDGE_SELECTED, selected_colour);
        let seams = grid4d_geom::selection::edges_from_corners(&state.seams, topology);
        let selected_edges = grid4d_geom::selection::edges_from_corners(&state.selected.edges, topology);
        for f in 0..mesh.face_count() {
            if state.hidden.polygons.contains(f) {
                continue;
            }
            let range = mesh.corner_range(f);
            for c in range.clone() {
                let n = if c + 1 < range.end { c + 1 } else { range.start };
                let line = [to_screen(shape.uvs[c]), to_screen(shape.uvs[n])];
                let unique = topology.corner_edge(c).map(|e| e as usize);
                let stroke = if component == Component::Edge && unique.is_some_and(|e| selected_edges.contains(e)) {
                    chosen
                } else if unique.is_some_and(|e| seams.contains(e)) {
                    seam
                } else {
                    edge
                };
                painter.line_segment(line, stroke);
            }
        }

        // Points in Points mode.
        if component == Component::Point {
            let corner_points = mesh.corner_points();
            for (c, &uv) in shape.uvs.iter().enumerate() {
                let point = corner_points.get(c).map_or(usize::MAX, |&p| p as usize);
                let selected = state.selected.points.contains(point);
                let size = metrics::COMPONENT_POINT * if selected { 1.2 } else { 0.8 };
                let colour = if selected { selected_colour } else { p().text };
                painter.rect_filled(
                    Rect::from_center_size(to_screen(uv), vec2(size, size)),
                    CornerRadius::ZERO,
                    colour,
                );
            }
        }

        // The handle.
        if target.editable && matches!(editor.tool, Tool::Move | Tool::Scale | Tool::Rotate) {
            let corners = selected_corners(mesh, topology, state, component, &shape.layout);
            if !corners.is_empty() {
                let pivot = match &self.drag {
                    Some(Drag::Transform(drag)) => to_screen(drag.pivot),
                    _ => to_screen(centre(&shape.uvs, &corners)),
                };
                paint_handle(painter, editor.tool, pivot);
            }
        }

        // A selection frame.
        if let (Some(Drag::Frame(start)), Some(end)) = (&self.drag, painter.ctx().pointer_latest_pos()) {
            let frame = Rect::from_two_pos(*start, end);
            painter.rect_filled(frame, CornerRadius::ZERO, p().accent.gamma_multiply(0.12));
            painter.rect_stroke(
                frame,
                CornerRadius::ZERO,
                Stroke::new(1.0, p().accent),
                StrokeKind::Middle,
            );
        }
    }
}

/// The component kind the editor works in: the mode's, Polygons outside the
/// component modes.
fn component(editor: &EditorState) -> Component {
    editor.mode.component().unwrap_or(Component::Polygon)
}

/// The UV Editor's options: the UV tools' settings.
fn options(ui: &mut Ui, editor: &mut EditorState) {
    let settings = &mut editor.uv_settings;
    egui::Grid::new("grid4d.uv_editor.options")
        .num_columns(2)
        .show(ui, |ui| {
            let row = |ui: &mut Ui, label: &str, add: &mut dyn FnMut(&mut Ui, egui::Id) -> Response| {
                let label = ui.label(tr(label));
                add(ui, label.id).labelled_by(label.id);
                ui.end_row();
            };
            row(ui, key("Seam Angle"), &mut |ui, _| {
                ui.add(
                    egui::DragValue::new(&mut settings.seam_angle)
                        .range(1.0..=179.0)
                        .suffix("°"),
                )
            });
            row(ui, key("Margin"), &mut |ui, _| {
                ui.add(
                    egui::DragValue::new(&mut settings.margin_pixels)
                        .range(0.0..=64.0)
                        .suffix(" px"),
                )
            });
            row(ui, key("Texture Size"), &mut |ui, _| {
                ui.add(
                    egui::DragValue::new(&mut settings.texture_size)
                        .range(16..=16_384)
                        .suffix(" px"),
                )
            });
            row(ui, key("Rotate Islands"), &mut |ui, _| {
                ui.checkbox(&mut settings.rotate, "")
            });
            row(ui, key("Relax Iterations"), &mut |ui, _| {
                ui.add(egui::DragValue::new(&mut settings.relax.iterations).range(1..=100))
            });
            row(ui, key("Pin Borders"), &mut |ui, _| {
                ui.checkbox(&mut settings.relax.pin_borders, "")
            });
        });
}

/// The corners a transform moves: those of the selected components, and every
/// corner joined to them in UV (so islands stretch rather than tear).
fn selected_corners(
    mesh: &PolyMesh,
    topology: &Topology,
    state: &ComponentState,
    component: Component,
    layout: &uv::UvLayout,
) -> Vec<usize> {
    let hidden = &state.hidden.polygons;
    let mut vertices = BitSet::new();
    let corner_points = mesh.corner_points();
    for f in 0..mesh.face_count() {
        if hidden.contains(f) {
            continue;
        }
        let range = mesh.corner_range(f);
        for c in range.clone() {
            let take = match component {
                Component::Point => state.selected.points.contains(corner_points[c] as usize),
                Component::Polygon => state.selected.polygons.contains(f),
                Component::Edge => {
                    let previous = if c == range.start { range.end - 1 } else { c - 1 };
                    let edge = |corner: usize| topology.corner_edge(corner).map(|e| e as usize);
                    let flagged = |corner: usize| {
                        state.selected.edges.contains(corner)
                            || edge(corner).is_some_and(|e| {
                                topology
                                    .edge_corners(e)
                                    .iter()
                                    .any(|&o| state.selected.edges.contains(o as usize))
                            })
                    };
                    flagged(c) || flagged(previous)
                }
            };
            if take {
                vertices.insert(layout.vertex_of[c] as usize);
            }
        }
    }
    (0..mesh.corner_count())
        .filter(|&c| vertices.contains(layout.vertex_of[c] as usize))
        .collect()
}

/// The middle of the box around `corners`' UVs.
fn centre(uvs: &[Vec2], corners: &[usize]) -> Vec2 {
    let mut min = Vec2::splat(f32::INFINITY);
    let mut max = Vec2::splat(f32::NEG_INFINITY);
    for &c in corners {
        if let Some(&uv) = uvs.get(c) {
            min = min.min(uv);
            max = max.max(uv);
        }
    }
    if min.is_finite() { (min + max) * 0.5 } else { Vec2::ZERO }
}

/// The part of `tool`'s handle at `pivot` under `at`.
fn handle_at(tool: Tool, pivot: Pos2, at: Pos2) -> Option<Handle> {
    let d = at - pivot;
    if tool == Tool::Rotate {
        return ((d.length() - HANDLE).abs() <= PICK || d.length() <= PICK).then_some(Handle::Free);
    }
    if d.x.abs() <= PICK && d.y.abs() <= PICK {
        return Some(Handle::Free);
    }
    if d.y.abs() <= PICK && d.x > 0.0 && d.x <= HANDLE + PICK {
        return Some(Handle::U);
    }
    if d.x.abs() <= PICK && d.y < 0.0 && -d.y <= HANDLE + PICK {
        return Some(Handle::V);
    }
    None
}

fn paint_handle(painter: &egui::Painter, tool: Tool, pivot: Pos2) {
    let (u, v) = (p().axis_x, p().axis_y);
    if tool == Tool::Rotate {
        painter.circle_stroke(pivot, HANDLE, Stroke::new(2.0, p().axis_z));
        painter.circle_filled(pivot, 3.0, p().text_strong);
        return;
    }
    let tip_u = pivot + vec2(HANDLE, 0.0);
    let tip_v = pivot + vec2(0.0, -HANDLE);
    painter.line_segment([pivot, tip_u], Stroke::new(2.0, u));
    painter.line_segment([pivot, tip_v], Stroke::new(2.0, v));
    if tool == Tool::Scale {
        painter.rect_filled(Rect::from_center_size(tip_u, vec2(8.0, 8.0)), CornerRadius::ZERO, u);
        painter.rect_filled(Rect::from_center_size(tip_v, vec2(8.0, 8.0)), CornerRadius::ZERO, v);
    } else {
        painter.add(Shape::convex_polygon(
            vec![
                tip_u + vec2(8.0, 0.0),
                tip_u + vec2(-2.0, -5.0),
                tip_u + vec2(-2.0, 5.0),
            ],
            u,
            Stroke::NONE,
        ));
        painter.add(Shape::convex_polygon(
            vec![tip_v + vec2(0.0, -8.0), tip_v + vec2(-5.0, 2.0), tip_v + vec2(5.0, 2.0)],
            v,
            Stroke::NONE,
        ));
    }
    painter.rect_stroke(
        Rect::from_center_size(pivot, vec2(10.0, 10.0)),
        CornerRadius::ZERO,
        Stroke::new(1.5, p().text_strong),
        StrokeKind::Middle,
    );
}

/// The UVs of `drag`'s corners with the pointer at `at` (`zoom` points per UV
/// unit, the pivot drawn at `pivot`).
fn transformed(drag: &Transform, at: Pos2, zoom: f32, pivot: Pos2) -> Vec<Vec2> {
    let d = at - drag.start;
    let mut uvs = drag.uvs.clone();
    let apply = |f: &dyn Fn(Vec2) -> Vec2, uvs: &mut Vec<Vec2>| {
        for &c in &drag.corners {
            if let Some(uv) = uvs.get_mut(c) {
                *uv = f(*uv);
            }
        }
    };
    match drag.tool {
        Tool::Scale => {
            let factor = |amount: f32| (1.0 + amount / 100.0).max(0.01);
            let s = match drag.handle {
                Handle::Free => Vec2::splat(factor(d.x - d.y)),
                Handle::U => Vec2::new(factor(d.x), 1.0),
                Handle::V => Vec2::new(1.0, factor(-d.y)),
            };
            apply(&|uv| drag.pivot + (uv - drag.pivot) * s, &mut uvs);
        }
        Tool::Rotate => {
            let from = drag.start - pivot;
            let to = at - pivot;
            let angle = to.y.atan2(to.x) - from.y.atan2(from.x);
            let rotation = Vec2::from_angle(angle);
            apply(&|uv| drag.pivot + rotation.rotate(uv - drag.pivot), &mut uvs);
        }
        _ => {
            let shift = match drag.handle {
                Handle::Free => Vec2::new(d.x, d.y),
                Handle::U => Vec2::new(d.x, 0.0),
                Handle::V => Vec2::new(0.0, d.y),
            } / zoom;
            apply(&|uv| uv + shift, &mut uvs);
        }
    }
    uvs
}

/// Blue for compressed, clear for even, red for stretched.
fn stretch_colour(stretch: f32) -> Color32 {
    let t = (stretch.abs() / 2.0).clamp(0.0, 1.0);
    let alpha = (t * 150.0) as u8;
    if stretch < 0.0 {
        Color32::from_rgba_unmultiplied(60, 120, 255, alpha)
    } else {
        Color32::from_rgba_unmultiplied(255, 70, 50, alpha)
    }
}

/// The visible polygon whose UVs contain `uv`.
fn polygon_at(mesh: &PolyMesh, state: &ComponentState, uvs: &[Vec2], uv: Vec2) -> Option<usize> {
    (0..mesh.face_count()).rev().find(|&f| {
        if state.hidden.polygons.contains(f) {
            return false;
        }
        let range = mesh.corner_range(f);
        let mut inside = false;
        for c in range.clone() {
            let n = if c + 1 < range.end { c + 1 } else { range.start };
            let (a, b) = (uvs[c], uvs[n]);
            if (a.y > uv.y) != (b.y > uv.y) && uv.x < a.x + (uv.y - a.y) / (b.y - a.y) * (b.x - a.x) {
                inside = !inside;
            }
        }
        inside
    })
}

/// The point of the visible UV corner nearest `at` on screen, within reach.
fn point_at(
    mesh: &PolyMesh,
    state: &ComponentState,
    uvs: &[Vec2],
    screen: impl Fn(Vec2) -> Pos2,
    at: Pos2,
) -> Option<usize> {
    let mut best: Option<(f32, usize)> = None;
    for f in (0..mesh.face_count()).filter(|&f| !state.hidden.polygons.contains(f)) {
        for c in mesh.corner_range(f) {
            let distance = screen(uvs[c]).distance(at);
            if distance <= PICK && best.is_none_or(|(d, _)| distance < d) {
                best = Some((distance, mesh.corner_points()[c] as usize));
            }
        }
    }
    best.map(|(_, point)| point)
}

/// The unique edge of the visible UV edge nearest `at` on screen, within reach.
fn edge_at(
    mesh: &PolyMesh,
    topology: &Topology,
    state: &ComponentState,
    uvs: &[Vec2],
    screen: impl Fn(Vec2) -> Pos2,
    at: Pos2,
) -> Option<usize> {
    let mut best: Option<(f32, usize)> = None;
    for f in (0..mesh.face_count()).filter(|&f| !state.hidden.polygons.contains(f)) {
        let range = mesh.corner_range(f);
        for c in range.clone() {
            let n = if c + 1 < range.end { c + 1 } else { range.start };
            let (a, b) = (screen(uvs[c]), screen(uvs[n]));
            let ab = b - a;
            let t = ((at - a).dot(ab) / ab.length_sq().max(f32::MIN_POSITIVE)).clamp(0.0, 1.0);
            let distance = (a + ab * t).distance(at);
            if distance <= EDGE_PICK
                && best.is_none_or(|(d, _)| distance < d)
                && let Some(edge) = topology.corner_edge(c)
            {
                best = Some((distance, edge as usize));
            }
        }
    }
    best.map(|(_, edge)| edge)
}

/// The components of `component` kind wholly inside a frame (`inside` says
/// for a UV): points with a corner inside, edges and polygons with every
/// corner inside.
fn inside(
    mesh: &PolyMesh,
    topology: &Topology,
    state: &ComponentState,
    uvs: &[Vec2],
    component: Component,
    inside: impl Fn(Vec2) -> bool,
) -> Vec<u32> {
    let mut hits = BitSet::new();
    for f in (0..mesh.face_count()).filter(|&f| !state.hidden.polygons.contains(f)) {
        let range = mesh.corner_range(f);
        match component {
            Component::Point => {
                for c in range.clone() {
                    if inside(uvs[c]) {
                        hits.insert(mesh.corner_points()[c] as usize);
                    }
                }
            }
            Component::Edge => {
                for c in range.clone() {
                    let n = if c + 1 < range.end { c + 1 } else { range.start };
                    if inside(uvs[c])
                        && inside(uvs[n])
                        && let Some(edge) = topology.corner_edge(c)
                    {
                        hits.insert(edge as usize);
                    }
                }
            }
            Component::Polygon => {
                if range.clone().all(|c| inside(uvs[c])) {
                    hits.insert(f);
                }
            }
        }
    }
    hits.iter().filter_map(|h| u32::try_from(h).ok()).collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test code")]

    use grid4d_geom::primitives;
    use grid4d_scene::Node;

    use super::*;

    fn drag(tool: Tool, handle: Handle, uvs: Vec<Vec2>) -> Transform {
        let data = Node::new("Plane", ObjectKind::Null).data;
        Transform {
            node: NodeId::default(),
            before: data,
            mesh: MeshHandle::new(primitives::plane(1.0, 1.0, 1, 1)),
            corners: (0..uvs.len()).collect(),
            uvs,
            pivot: Vec2::ZERO,
            tool,
            handle,
            start: pos2(100.0, 0.0),
        }
    }

    #[test]
    fn handles_move_scale_and_turn_about_the_pivot() {
        let uvs = vec![Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0)];
        let pivot = pos2(0.0, 0.0);
        // Moving 50 points right at 100 points per unit: half a unit, U only.
        let moved = transformed(
            &drag(Tool::Move, Handle::U, uvs.clone()),
            pos2(150.0, 30.0),
            100.0,
            pivot,
        );
        assert!(moved[0].abs_diff_eq(Vec2::new(1.5, 0.0), 1e-6) && moved[1].abs_diff_eq(Vec2::new(0.5, 1.0), 1e-6));
        // Scaling along U doubles u about the pivot.
        let scaled = transformed(
            &drag(Tool::Scale, Handle::U, uvs.clone()),
            pos2(200.0, 0.0),
            100.0,
            pivot,
        );
        assert!(scaled[0].abs_diff_eq(Vec2::new(2.0, 0.0), 1e-6) && scaled[1].abs_diff_eq(Vec2::new(0.0, 1.0), 1e-6));
        // A quarter turn (screen y runs down like v).
        let turned = transformed(&drag(Tool::Rotate, Handle::Free, uvs), pos2(0.0, 100.0), 100.0, pivot);
        assert!(turned[0].abs_diff_eq(Vec2::new(0.0, 1.0), 1e-5), "{}", turned[0]);
        // Handles are found where they are drawn.
        assert_eq!(handle_at(Tool::Move, pivot, pos2(40.0, 0.0)), Some(Handle::U));
        assert_eq!(handle_at(Tool::Move, pivot, pos2(0.0, -40.0)), Some(Handle::V));
        assert_eq!(handle_at(Tool::Scale, pivot, pos2(2.0, 2.0)), Some(Handle::Free));
        assert_eq!(handle_at(Tool::Rotate, pivot, pos2(HANDLE, 0.0)), Some(Handle::Free));
        assert_eq!(handle_at(Tool::Move, pivot, pos2(30.0, 30.0)), None);
    }

    #[test]
    fn a_selected_polygon_takes_the_corners_joined_to_it_along() {
        // A 2 × 1 plane: one island, the middle edge's corners shared in UV.
        let plane = primitives::plane(2.0, 1.0, 2, 1);
        let topology = Topology::build(&plane);
        let layout = uv::layout(&plane, &topology);
        let mut state = ComponentState::default();
        state.selected.polygons.insert(0);
        let corners = selected_corners(&plane, &topology, &state, Component::Polygon, &layout);
        // Its four corners, and the two of the other polygon on the shared edge.
        assert_eq!(corners.len(), 6);
        state.selected.points.insert(0);
        let points = selected_corners(&plane, &topology, &state, Component::Point, &layout);
        assert_eq!(points.len(), 1, "a corner point of the plane");
    }
}
