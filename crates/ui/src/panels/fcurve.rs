//! The F-Curve editor (see `docs/reference/animation-fcurves.md`): the animated
//! values of the selected objects as curves of value over time. Keys are
//! selected, moved and given tangents here; the menu changes interpolation and
//! tangents. A tab beside the Timeline.
//!
//! Drags write the document live and record one undo step when they end
//! (`EditorCommand::CommitLiveAll`); menu commands and Delete are
//! `EditorCommand::EditCurveKeys`.

use egui::{
    Align2, Color32, CornerRadius, DragValue, FontId, PointerButton, Pos2, Rect, Response, RichText, ScrollArea, Sense,
    Shape, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2,
};
use grid4d_scene::{Curve, Document, Interpolation, NodeData, NodeId, Param, SceneEdit, TangentPreset, Tangents};

use crate::{
    CurveKey, EditorCommand, EditorState, FrameCurves,
    keys::CurveOp,
    live_edit::{LiveEdit, interaction_ongoing},
    theme::{metrics, palette as p},
    widgets::header_row,
};

/// Default height of the panel.
pub const DEFAULT_HEIGHT: f32 = 240.0;
const TRACK_LIST_WIDTH: f32 = 170.0;
const RULER_HEIGHT: f32 = 18.0;
const VALUE_GUTTER: f32 = 48.0;
/// Keys and handles are hit within this many points.
const PICK: f32 = 6.0;
/// A curve is hit within this many points.
const CURVE_PICK: f32 = 4.0;

/// The unit a parameter is shown in: rotations and stored angles in degrees,
/// fractions in percent.
fn display_scale(param: Param) -> f64 {
    match param {
        Param::Rotation(_) => f64::from(180.0_f32 / std::f32::consts::PI),
        Param::Position(_) | Param::Scale(_) => 1.0,
        Param::Object(p) => p.unit().display_scale(),
    }
}

fn color(param: Param) -> Color32 {
    let axis = match param {
        Param::Position(a) | Param::Rotation(a) | Param::Scale(a) => a,
        Param::Object(param) => return p().param_curves[param.index() % p().param_curves.len()],
    };
    let base = p().axis[axis.index()];
    match param {
        // Lighter / darker shades keep the channels apart.
        Param::Rotation(_) => base.lerp_to_gamma(Color32::WHITE, 0.35),
        Param::Scale(_) => base.lerp_to_gamma(Color32::BLACK, 0.3),
        Param::Position(_) | Param::Object(_) => base,
    }
}

/// `data`'s animated parameters in the order the editor lists them: the
/// transform first, then the object's own in Attribute Manager order, then any
/// others it has tracks for.
fn track_order(data: &NodeData) -> Vec<Param> {
    let mut order: Vec<Param> = Param::TRANSFORM
        .into_iter()
        .chain(data.kind.params().into_iter().map(Param::Object))
        .filter(|&param| data.tracks.iter().any(|t| t.param == param))
        .collect();
    for track in &data.tracks {
        if !order.contains(&track.param) {
            order.push(track.param);
        }
    }
    order
}

/// Which side of a key a tangent handle is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Left,
    Right,
}

/// The visible part of the graph: frames and display values per point.
#[derive(Clone, Copy, Debug, PartialEq)]
struct CurveView {
    /// Frame at the left edge, display value at the bottom edge.
    frame: f64,
    value: f64,
    frames_per_pt: f64,
    values_per_pt: f64,
}

impl Default for CurveView {
    fn default() -> Self {
        Self {
            frame: -5.0,
            value: -50.0,
            frames_per_pt: 0.2,
            values_per_pt: 1.0,
        }
    }
}

impl CurveView {
    fn x(&self, rect: Rect, frame: f64) -> f32 {
        rect.left() + ((frame - self.frame) / self.frames_per_pt) as f32
    }

    fn y(&self, rect: Rect, value: f64) -> f32 {
        rect.bottom() - ((value - self.value) / self.values_per_pt) as f32
    }

    fn pos(&self, rect: Rect, frame: f64, value: f64) -> Pos2 {
        pos2(self.x(rect, frame), self.y(rect, value))
    }

    fn frame_at(&self, rect: Rect, x: f32) -> f64 {
        self.frame + f64::from(x - rect.left()) * self.frames_per_pt
    }

    fn value_at(&self, rect: Rect, y: f32) -> f64 {
        self.value + f64::from(rect.bottom() - y) * self.values_per_pt
    }

    /// Fits frames `f0..f1` and display values `v0..v1` into `rect`, with a margin.
    fn fit(&mut self, rect: Rect, (f0, f1): (f64, f64), (v0, v1): (f64, f64)) {
        let (f0, f1) = if f1 - f0 < 1.0 { (f0 - 5.0, f1 + 5.0) } else { (f0, f1) };
        let (v0, v1) = if v1 - v0 < 1e-3 {
            (v0 - 10.0, v1 + 10.0)
        } else {
            (v0, v1)
        };
        let (w, h) = (f64::from(rect.width().max(1.0)), f64::from(rect.height().max(1.0)));
        self.frames_per_pt = (f1 - f0) * 1.16 / w;
        self.values_per_pt = (v1 - v0) * 1.2 / h;
        self.frame = f0 - (f1 - f0) * 0.08;
        self.value = v0 - (v1 - v0) * 0.1;
    }

    /// Zooms by `fx` in time and `fy` in value around the point under `at`.
    fn zoom(&mut self, rect: Rect, at: Pos2, fx: f64, fy: f64) {
        let (frame, value) = (self.frame_at(rect, at.x), self.value_at(rect, at.y));
        self.frames_per_pt = (self.frames_per_pt * fx).clamp(1e-4, 1e5);
        self.values_per_pt = (self.values_per_pt * fy).clamp(1e-6, 1e7);
        self.frame = frame - f64::from(at.x - rect.left()) * self.frames_per_pt;
        self.value = value - f64::from(rect.bottom() - at.y) * self.values_per_pt;
    }
}

/// A shown curve: whose and which.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Shown {
    node: NodeId,
    param: Param,
}

/// A drag in the graph.
#[derive(Clone, Debug)]
enum CurveDrag {
    /// Moving the selected keys: their objects' data before, the keys before,
    /// where the pointer started.
    Keys {
        before: Vec<(NodeId, NodeData)>,
        keys: Vec<CurveKey>,
        start: Pos2,
    },
    /// A tangent handle of one key (the object's data boxed: it is large).
    Handle {
        key: CurveKey,
        side: Side,
        before: Box<NodeData>,
    },
    /// Selecting the keys in a box; `toggle` with Ctrl.
    Box { from: Pos2, to: Pos2, toggle: bool },
}

/// What is under the pointer in the graph.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Hit {
    Handle(CurveKey, Side),
    Key(CurveKey),
    Curve(Shown),
}

#[derive(Debug, Default)]
pub struct FCurvePanel {
    view: CurveView,
    /// The tracks the view was last framed for (it frames again when they change).
    framed_for: Vec<(NodeId, Param)>,
    /// The object selection the key selection belongs to.
    seen_selection: Vec<NodeId>,
    drag: Option<CurveDrag>,
    /// The header's key fields.
    key_edit: LiveEdit<NodeId, NodeData>,
    /// Where the graph was last frame (tests point at keys with it).
    graph: Option<Rect>,
}

impl FCurvePanel {
    pub fn show(&mut self, ui: &mut Ui, doc: &mut Document, editor: &mut EditorState) {
        ui.spacing_mut().item_spacing.y = 0.0;
        // A new object selection starts a new key selection and track filter.
        if self.seen_selection.as_slice() != editor.selection.as_slice() {
            self.seen_selection.clear();
            self.seen_selection.extend_from_slice(editor.selection.as_slice());
            editor.curves.keys.clear();
            editor.curves.tracks.clear();
            self.drag = None;
        }
        let shown = shown_tracks(doc, editor);
        // Keys that went away (undo, a deleted track) are no longer selected.
        if self.drag.is_none() {
            editor.curves.keys.retain(|k| key_exists(doc, *k));
        }

        let header = header_row(ui);
        self.key_fields(ui, header, doc, editor);
        let body = ui.available_rect_before_wrap();
        ui.allocate_rect(body, Sense::hover());
        let list = Rect::from_min_size(body.min, vec2(TRACK_LIST_WIDTH.min(body.width() * 0.4), body.height()));
        let graph = Rect::from_min_max(pos2(list.right() + 1.0, body.top()), body.max);
        track_list(ui, list, doc, editor);
        if graph.width() > VALUE_GUTTER + 10.0 && graph.height() > RULER_HEIGHT + 10.0 {
            self.graph(ui, graph, doc, editor, &shown);
        }
    }

    /// The selected key's frame and value, editable when there is exactly one.
    fn key_fields(&mut self, ui: &mut Ui, header: Rect, doc: &mut Document, editor: &mut EditorState) {
        let fields = Rect::from_min_max(pos2(header.right() - 330.0, header.top()), header.max);
        let single = match editor.curves.keys.as_slice() {
            [key] => Some(*key),
            _ => None,
        };
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(fields)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let Some(key) = single else {
                    let n = editor.curves.keys.len();
                    let text = if n == 0 {
                        "No key selected".to_owned()
                    } else {
                        format!("{n} keys selected")
                    };
                    ui.label(RichText::new(text).color(p().text_dim).size(11.5));
                    return;
                };
                let Some(mut data) = doc.scene.get(key.node).map(|n| n.data.clone()) else {
                    return;
                };
                self.key_edit.begin(key.node, &data);
                let k = display_scale(key.param);
                let Some(found) = data
                    .tracks
                    .iter()
                    .find(|t| t.param == key.param)
                    .and_then(|t| t.curve.key_at(key.frame).copied())
                else {
                    return;
                };
                ui.label(RichText::new(key.param.label()).color(p().text_dim).size(11.5));
                #[expect(clippy::cast_possible_truncation, reason = "key frames are within the i32 range")]
                let mut frame = found.frame.round() as i32;
                let mut value = f64::from(found.value) * k;
                let frame_changed = ui
                    .add_sized([64.0, 18.0], DragValue::new(&mut frame).speed(0.2).suffix(" F"))
                    .on_hover_text("The key's frame")
                    .changed();
                let value_changed = ui
                    .add_sized([80.0, 18.0], DragValue::new(&mut value).speed(0.5).max_decimals(3))
                    .on_hover_text("The key's value")
                    .changed();
                if (frame_changed || value_changed)
                    && let Some(track) = data.tracks.iter_mut().find(|t| t.param == key.param)
                {
                    let new_frame = f64::from(frame);
                    track.curve.remove_key(found.frame);
                    track.curve.set_key(grid4d_scene::Key {
                        frame: new_frame,
                        value: (value / k) as f32,
                        ..found
                    });
                    if let Some(node) = doc.scene.get_mut(key.node) {
                        node.data.clone_from(&data);
                    }
                    if let Some(selected) = editor.curves.keys.first_mut() {
                        selected.frame = new_frame;
                    }
                }
                if let Some((id, before)) = self.key_edit.end(key.node, &data, interaction_ongoing(ui)) {
                    editor.commands.push(EditorCommand::CommitLive {
                        inverse: SceneEdit::SetData { id, data: before },
                        label: "Edit Key",
                    });
                }
            },
        );
    }

    fn graph(&mut self, ui: &mut Ui, rect: Rect, doc: &mut Document, editor: &mut EditorState, shown: &[Shown]) {
        self.graph = Some(rect);
        let ruler = Rect::from_min_size(rect.min, vec2(rect.width(), RULER_HEIGHT));
        let plot = Rect::from_min_max(pos2(rect.left() + VALUE_GUTTER, ruler.bottom()), rect.max);

        // Frame all curves when the shown tracks change.
        let tracks: Vec<(NodeId, Param)> = shown.iter().map(|s| (s.node, s.param)).collect();
        if tracks != self.framed_for {
            self.framed_for = tracks;
            self.frame_keys(plot, doc, shown, false, editor);
        }
        if let Some(request) = editor.curves.frame.take() {
            self.frame_keys(plot, doc, shown, request == FrameCurves::Selected, editor);
        }

        let response = ui.interact(plot, egui::Id::unique("grid4d.fcurve.graph"), Sense::click_and_drag());
        editor.curves.hovered = response.hovered();
        if ui.input(|i| i.pointer.any_pressed()) {
            editor.curves.focused = response.hovered();
        }
        self.navigate(ui, &response, plot);
        self.ruler(ui, ruler, plot, doc);
        self.interact(ui, &response, plot, doc, editor, shown);
        self.paint(ui, rect, plot, doc, editor, shown);
        self.context_menu(&response, plot, doc, editor, shown);
        if response.hovered() {
            editor.status_hint = Some(
                "F-Curve: click/drag keys · Ctrl toggles · drag a handle (Shift: one side) · Ctrl+click on a curve adds a key · wheel zoom (Ctrl time, Shift value) · middle drag pans · H / S frame",
            );
        }
    }

    /// Fits the view to every shown key, or (`selected`) to the selected ones.
    fn frame_keys(&mut self, plot: Rect, doc: &Document, shown: &[Shown], selected: bool, editor: &EditorState) {
        let mut frames = (f64::INFINITY, f64::NEG_INFINITY);
        let mut values = (f64::INFINITY, f64::NEG_INFINITY);
        for s in shown {
            let Some(curve) = curve_of(doc, s.node, s.param) else {
                continue;
            };
            let k = display_scale(s.param);
            for key in curve.keys() {
                if selected && !editor.curves.contains(s.node, s.param, key.frame) {
                    continue;
                }
                let v = f64::from(key.value) * k;
                frames = (frames.0.min(key.frame), frames.1.max(key.frame));
                values = (values.0.min(v), values.1.max(v));
            }
        }
        if frames.0.is_finite() {
            self.view.fit(plot, frames, values);
        }
    }

    fn navigate(&mut self, ui: &Ui, response: &Response, plot: Rect) {
        let alt = ui.input(|i| i.modifiers.alt);
        if response.dragged_by(PointerButton::Middle) || (alt && response.dragged_by(PointerButton::Primary)) {
            let d = response.drag_delta();
            self.view.frame -= f64::from(d.x) * self.view.frames_per_pt;
            self.view.value += f64::from(d.y) * self.view.values_per_pt;
        }
        let Some(at) = response.hover_pos() else { return };
        let (scroll, zoom, shift) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.modifiers.shift));
        // Ctrl+wheel arrives as a zoom: time only. Shift+wheel scrolls sideways: values only.
        if zoom != 1.0 {
            self.view.zoom(plot, at, 1.0 / f64::from(zoom), 1.0);
        }
        let amount = f64::from(scroll.y + scroll.x);
        if amount != 0.0 {
            let factor = (-amount * 0.002).exp();
            if shift {
                self.view.zoom(plot, at, 1.0, factor);
            } else {
                self.view.zoom(plot, at, factor, factor);
            }
        }
    }

    /// The frame numbers above the graph; click or drag sets the current frame.
    fn ruler(&self, ui: &Ui, ruler: Rect, plot: Rect, doc: &mut Document) {
        let response = ui.interact(ruler, egui::Id::unique("grid4d.fcurve.ruler"), Sense::click_and_drag());
        if (response.clicked() || response.dragged())
            && let Some(pointer) = response.interact_pointer_pos()
            && let Some(frame) = crate::keys::whole_frame(self.view.frame_at(plot, pointer.x))
        {
            doc.time.stop();
            doc.time.set_frame(frame);
        }
    }

    /// What is under `at`: a handle of a selected key, a key, or a curve.
    fn hit(&self, plot: Rect, at: Pos2, doc: &Document, editor: &EditorState, shown: &[Shown]) -> Option<Hit> {
        let mut best: Option<(f32, Hit)> = None;
        let consider = |best: &mut Option<(f32, Hit)>, d: f32, hit: Hit, limit: f32| {
            if d <= limit && best.is_none_or(|(bd, _)| d < bd) {
                *best = Some((d, hit));
            }
        };
        for s in shown {
            let Some(curve) = curve_of(doc, s.node, s.param) else {
                continue;
            };
            let k = display_scale(s.param);
            for (i, key) in curve.keys().iter().enumerate() {
                let ck = CurveKey {
                    node: s.node,
                    param: s.param,
                    frame: key.frame,
                };
                let pos = self.view.pos(plot, key.frame, f64::from(key.value) * k);
                if editor.curves.contains(s.node, s.param, key.frame) {
                    for (side, end) in handle_ends(curve, i, k) {
                        let end = self.view.pos(plot, end.0, end.1);
                        // Handles win over their key when the pointer is on them.
                        consider(&mut best, end.distance(at) - 1.0, Hit::Handle(ck, side), PICK);
                    }
                }
                consider(&mut best, pos.distance(at), Hit::Key(ck), PICK);
            }
        }
        if best.is_some() {
            return best.map(|(_, hit)| hit);
        }
        // Curves: the distance at the pointer's frame.
        let frame = self.view.frame_at(plot, at.x);
        for s in shown {
            let Some(curve) = curve_of(doc, s.node, s.param) else {
                continue;
            };
            if let Some(v) = curve.value_at(frame) {
                let y = self.view.y(plot, f64::from(v) * display_scale(s.param));
                consider(&mut best, (y - at.y).abs(), Hit::Curve(*s), CURVE_PICK);
            }
        }
        best.map(|(_, hit)| hit)
    }

    fn interact(
        &mut self,
        ui: &Ui,
        response: &Response,
        plot: Rect,
        doc: &mut Document,
        editor: &mut EditorState,
        shown: &[Shown],
    ) {
        let (ctrl, shift, alt) = ui.input(|i| (i.modifiers.command, i.modifiers.shift, i.modifiers.alt));
        if alt {
            return; // Alt+drag pans.
        }
        if response.drag_started_by(PointerButton::Primary)
            && let Some(press) = ui.input(|i| i.pointer.press_origin())
        {
            self.drag = Some(match self.hit(plot, press, doc, editor, shown) {
                Some(Hit::Handle(key, side)) => match doc.scene.get(key.node) {
                    Some(node) => CurveDrag::Handle {
                        key,
                        side,
                        before: Box::new(node.data.clone()),
                    },
                    None => return,
                },
                Some(Hit::Key(key)) => {
                    if !editor.curves.contains(key.node, key.param, key.frame) {
                        if !ctrl {
                            editor.curves.keys.clear();
                        }
                        editor.curves.insert(key);
                    }
                    let keys = editor.curves.keys.clone();
                    let mut before: Vec<(NodeId, NodeData)> = Vec::new();
                    for k in &keys {
                        if !before.iter().any(|(id, _)| *id == k.node)
                            && let Some(node) = doc.scene.get(k.node)
                        {
                            before.push((k.node, node.data.clone()));
                        }
                    }
                    CurveDrag::Keys {
                        before,
                        keys,
                        start: press,
                    }
                }
                Some(Hit::Curve(_)) | None => CurveDrag::Box {
                    from: press,
                    to: press,
                    toggle: ctrl,
                },
            });
        }
        if let (Some(drag), Some(pointer)) = (&mut self.drag, response.interact_pointer_pos()) {
            match drag {
                CurveDrag::Keys { before, keys, start } => {
                    let mut d = pointer - *start;
                    if shift {
                        // Horizontal or vertical, whichever is larger.
                        if d.x.abs() >= d.y.abs() {
                            d.y = 0.0;
                        } else {
                            d.x = 0.0;
                        }
                    }
                    let frames = (f64::from(d.x) * self.view.frames_per_pt).round();
                    let value = -f64::from(d.y) * self.view.values_per_pt;
                    move_keys(doc, before, keys, frames, value);
                    editor.curves.keys = keys
                        .iter()
                        .map(|k| CurveKey {
                            frame: k.frame + frames,
                            ..*k
                        })
                        .collect();
                }
                CurveDrag::Handle { key, side, before } => {
                    let at = (
                        self.view.frame_at(plot, pointer.x),
                        self.view.value_at(plot, pointer.y) / display_scale(key.param),
                    );
                    let mut data = NodeData::clone(before);
                    if let Some(track) = data.tracks.iter_mut().find(|t| t.param == key.param) {
                        drag_handle(&mut track.curve, key.frame, *side, at, !shift);
                    }
                    if let Some(node) = doc.scene.get_mut(key.node)
                        && node.data != data
                    {
                        node.data = data;
                    }
                }
                CurveDrag::Box { to, .. } => *to = pointer,
            }
        }
        if response.drag_stopped()
            && let Some(drag) = self.drag.take()
        {
            match drag {
                CurveDrag::Keys { before, .. } => editor.commands.push(EditorCommand::CommitLiveAll {
                    inverses: before
                        .into_iter()
                        .map(|(id, data)| SceneEdit::SetData { id, data })
                        .collect(),
                    label: "Move Keys",
                }),
                CurveDrag::Handle { key, before, .. } => editor.commands.push(EditorCommand::CommitLiveAll {
                    inverses: vec![SceneEdit::SetData {
                        id: key.node,
                        data: *before,
                    }],
                    label: "Edit Tangents",
                }),
                CurveDrag::Box { from, to, toggle } => {
                    if !toggle {
                        editor.curves.keys.clear();
                    }
                    let area = Rect::from_two_pos(from, to);
                    for s in shown {
                        let Some(curve) = curve_of(doc, s.node, s.param) else {
                            continue;
                        };
                        let k = display_scale(s.param);
                        for key in curve.keys() {
                            if area.contains(self.view.pos(plot, key.frame, f64::from(key.value) * k)) {
                                let ck = CurveKey {
                                    node: s.node,
                                    param: s.param,
                                    frame: key.frame,
                                };
                                if toggle {
                                    editor.curves.toggle(ck);
                                } else {
                                    editor.curves.insert(ck);
                                }
                            }
                        }
                    }
                }
            }
        }
        if response.clicked()
            && let Some(at) = response.interact_pointer_pos()
        {
            match self.hit(plot, at, doc, editor, shown) {
                Some(Hit::Key(key) | Hit::Handle(key, _)) => {
                    if ctrl {
                        editor.curves.toggle(key);
                    } else {
                        editor.curves.keys.clear();
                        editor.curves.keys.push(key);
                    }
                }
                Some(Hit::Curve(s)) if ctrl => {
                    // A new key on the curve, at the whole frame under the pointer.
                    let frame = self.view.frame_at(plot, at.x).round();
                    if let Some(node) = doc.scene.get(s.node) {
                        let mut data = node.data.clone();
                        if let Some(track) = data.tracks.iter_mut().find(|t| t.param == s.param)
                            && track.curve.key_at(frame).is_none()
                            && let Some(value) = track.curve.value_at(frame)
                        {
                            track.curve.set_key(grid4d_scene::Key::new(frame, value));
                            editor.commands.push(EditorCommand::Edit {
                                edit: SceneEdit::SetData { id: s.node, data },
                                label: "Add Key",
                            });
                            editor.curves.keys.clear();
                            editor.curves.keys.push(CurveKey {
                                node: s.node,
                                param: s.param,
                                frame,
                            });
                        }
                    }
                }
                Some(Hit::Curve(s)) => {
                    // The whole curve's keys.
                    editor.curves.keys.clear();
                    if let Some(curve) = curve_of(doc, s.node, s.param) {
                        for key in curve.keys() {
                            editor.curves.keys.push(CurveKey {
                                node: s.node,
                                param: s.param,
                                frame: key.frame,
                            });
                        }
                    }
                }
                None if !ctrl => editor.curves.keys.clear(),
                None => {}
            }
        }
        // A right-click on an unselected key selects it for the menu.
        if response.secondary_clicked()
            && let Some(at) = response.interact_pointer_pos()
            && let Some(Hit::Key(key)) = self.hit(plot, at, doc, editor, shown)
            && !editor.curves.contains(key.node, key.param, key.frame)
        {
            editor.curves.keys.clear();
            editor.curves.keys.push(key);
        }
    }

    fn context_menu(
        &mut self,
        response: &Response,
        plot: Rect,
        doc: &Document,
        editor: &mut EditorState,
        shown: &[Shown],
    ) {
        response.context_menu(|ui| {
            let any = !editor.curves.keys.is_empty();
            ui.add_enabled_ui(any, |ui| {
                ui.menu_button("Interpolation", |ui| {
                    for interpolation in Interpolation::ALL {
                        if ui.button(interpolation.label()).clicked() {
                            editor.commands.push(EditorCommand::EditCurveKeys {
                                op: CurveOp::Interpolation(interpolation),
                            });
                            ui.close();
                        }
                    }
                });
                ui.menu_button("Tangents", |ui| {
                    for preset in TangentPreset::ALL {
                        if ui.button(preset.label()).clicked() {
                            editor.commands.push(EditorCommand::EditCurveKeys {
                                op: CurveOp::Tangents(preset),
                            });
                            ui.close();
                        }
                    }
                });
                if ui
                    .add(egui::Button::new("Delete Keys").shortcut_text("Delete"))
                    .clicked()
                {
                    editor
                        .commands
                        .push(EditorCommand::EditCurveKeys { op: CurveOp::Delete });
                    ui.close();
                }
            });
            ui.separator();
            if ui.add(egui::Button::new("Frame All").shortcut_text("H")).clicked() {
                self.frame_keys(plot, doc, shown, false, editor);
                ui.close();
            }
            if ui
                .add_enabled(any, egui::Button::new("Frame Selected").shortcut_text("S"))
                .clicked()
            {
                self.frame_keys(plot, doc, shown, true, editor);
                ui.close();
            }
        });
    }

    fn paint(&self, ui: &Ui, rect: Rect, plot: Rect, doc: &Document, editor: &EditorState, shown: &[Shown]) {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::ZERO, p().bg_well);
        let small = FontId::proportional(10.5);
        let view = &self.view;

        // Grid and numbers.
        let frame_step = nice_step(view.frames_per_pt * 60.0).max(1.0);
        let mut f = (view.frame_at(plot, plot.left()) / frame_step).ceil() * frame_step;
        while f <= view.frame_at(plot, plot.right()) {
            let x = view.x(plot, f);
            painter.line_segment(
                [pos2(x, plot.top()), pos2(x, plot.bottom())],
                Stroke::new(1.0, p().bg_header),
            );
            painter.text(
                pos2(x + 3.0, rect.top() + 2.0),
                Align2::LEFT_TOP,
                format_value(f, frame_step),
                small.clone(),
                p().text_dim,
            );
            f += frame_step;
        }
        let value_step = nice_step(view.values_per_pt * 40.0);
        let mut v = (view.value_at(plot, plot.bottom()) / value_step).ceil() * value_step;
        while v <= view.value_at(plot, plot.top()) {
            let y = view.y(plot, v);
            let zero = v.abs() < value_step * 1e-6;
            let stroke = Stroke::new(1.0, if zero { p().separator } else { p().bg_header });
            painter.line_segment([pos2(plot.left(), y), pos2(plot.right(), y)], stroke);
            painter.text(
                pos2(plot.left() - 4.0, y),
                Align2::RIGHT_CENTER,
                format_value(v, value_step),
                small.clone(),
                p().text_dim,
            );
            v += value_step;
        }
        painter.line_segment(
            [pos2(rect.left(), plot.top()), pos2(rect.right(), plot.top())],
            Stroke::new(1.0, p().separator),
        );

        // The current frame.
        let x = view.x(plot, f64::from(doc.time.frame()));
        if x >= plot.left() {
            painter.line_segment(
                [pos2(x, rect.top()), pos2(x, plot.bottom())],
                Stroke::new(1.5, p().accent),
            );
        }

        let plot_painter = ui.painter_at(plot);
        for s in shown {
            let Some(curve) = curve_of(doc, s.node, s.param) else {
                continue;
            };
            let (Some(first), Some(last)) = (curve.keys().first(), curve.keys().last()) else {
                continue;
            };
            let k = display_scale(s.param);
            let colour = color(s.param);
            // Before the first and after the last key: the held value, dimmed.
            let dim = Stroke::new(1.0, colour.gamma_multiply(0.45));
            let (y0, y1) = (
                view.y(plot, f64::from(first.value) * k),
                view.y(plot, f64::from(last.value) * k),
            );
            plot_painter.line_segment([pos2(plot.left(), y0), pos2(view.x(plot, first.frame), y0)], dim);
            plot_painter.line_segment([pos2(view.x(plot, last.frame), y1), pos2(plot.right(), y1)], dim);
            // Between the keys: sampled every two points.
            let (x0, x1) = (
                view.x(plot, first.frame).max(plot.left()),
                view.x(plot, last.frame).min(plot.right()),
            );
            if x1 > x0 {
                let mut points = Vec::new();
                let mut x = x0;
                while x < x1 + 2.0 {
                    let xc = x.min(x1);
                    if let Some(v) = curve.value_at(view.frame_at(plot, xc)) {
                        points.push(pos2(xc, view.y(plot, f64::from(v) * k)));
                    }
                    x += 2.0;
                }
                plot_painter.add(Shape::line(points, Stroke::new(1.6, colour)));
            }
            // Keys, and the handles of selected ones.
            for (i, key) in curve.keys().iter().enumerate() {
                let pos = view.pos(plot, key.frame, f64::from(key.value) * k);
                let selected = editor.curves.contains(s.node, s.param, key.frame);
                if selected {
                    for (_, end) in handle_ends(curve, i, k) {
                        let end = view.pos(plot, end.0, end.1);
                        plot_painter.line_segment([pos, end], Stroke::new(1.0, p().text_dim));
                        plot_painter.rect_filled(
                            Rect::from_center_size(end, vec2(5.0, 5.0)),
                            CornerRadius::ZERO,
                            p().text,
                        );
                    }
                }
                let fill = if selected { p().highlight } else { p().key };
                plot_painter.circle_filled(pos, 3.5, fill);
                plot_painter.circle_stroke(pos, 3.5, Stroke::new(1.0, p().border));
            }
        }
        if let Some(CurveDrag::Box { from, to, .. }) = &self.drag {
            let area = Rect::from_two_pos(*from, *to);
            plot_painter.rect_filled(area, CornerRadius::ZERO, p().accent.gamma_multiply(0.2));
            plot_painter.rect_stroke(
                area,
                CornerRadius::ZERO,
                Stroke::new(1.0, p().accent),
                StrokeKind::Inside,
            );
        }
        if shown.is_empty() {
            let text = if editor.selection.is_empty() {
                "Select animated objects to see their curves"
            } else {
                "The selected objects are not animated: record keys with F9"
            };
            painter.text(
                plot.center(),
                Align2::CENTER_CENTER,
                text,
                FontId::proportional(13.0),
                p().text_dim,
            );
        }
    }
}

/// The curves the editor shows: the selected objects' tracks, filtered by the
/// track list.
fn shown_tracks(doc: &Document, editor: &EditorState) -> Vec<Shown> {
    let mut shown = Vec::new();
    for &node in editor.selection.as_slice() {
        let Some(n) = doc.scene.get(node) else { continue };
        for param in track_order(&n.data) {
            if editor.curves.tracks.is_empty() || editor.curves.tracks.contains(&(node, param)) {
                shown.push(Shown { node, param });
            }
        }
    }
    shown
}

fn curve_of(doc: &Document, node: NodeId, param: Param) -> Option<&Curve> {
    doc.scene
        .get(node)?
        .data
        .tracks
        .iter()
        .find(|t| t.param == param)
        .map(|t| &t.curve)
}

fn key_exists(doc: &Document, key: CurveKey) -> bool {
    curve_of(doc, key.node, key.param).is_some_and(|c| c.key_at(key.frame).is_some())
}

/// The ends of key `i`'s handles (frame, display value) on the sides where a
/// Spline segment uses them.
fn handle_ends(curve: &Curve, i: usize, k: f64) -> impl Iterator<Item = (Side, (f64, f64))> {
    let keys = curve.keys();
    let key = keys[i];
    let ((lf, lv), (rf, rv)) = curve.handles(i);
    let left = (i > 0 && keys[i - 1].interpolation == Interpolation::Spline)
        .then(|| (Side::Left, (key.frame - lf, (f64::from(key.value) - f64::from(lv)) * k)));
    let right = (i + 1 < keys.len() && key.interpolation == Interpolation::Spline).then(|| {
        (
            Side::Right,
            (key.frame + rf, (f64::from(key.value) + f64::from(rv)) * k),
        )
    });
    left.into_iter().chain(right)
}

/// Sets the `side` handle of the key at `frame` to end at `at` (frame, value):
/// the tangents become manual; with `smooth` the other handle keeps its length
/// and takes the same slope.
fn drag_handle(curve: &mut Curve, frame: f64, side: Side, at: (f64, f64), smooth: bool) {
    let Some(i) = curve.keys().iter().position(|k| k.frame == frame) else {
        return;
    };
    let key = curve.keys()[i];
    let (left, right) = curve.handles(i);
    let value = f64::from(key.value);
    // (frames ≥ a hair, value change), pointing away from the key.
    let (frames, change) = match side {
        Side::Left => ((key.frame - at.0).max(0.01), value - at.1),
        Side::Right => ((at.0 - key.frame).max(0.01), at.1 - value),
    };
    #[expect(clippy::cast_possible_truncation, reason = "values are f32")]
    let dragged = (frames, change as f32);
    let slope = change / frames;
    #[expect(clippy::cast_possible_truncation, reason = "values are f32")]
    let follow = |(length, _): (f64, f32)| (length, (slope * length) as f32);
    let (left, right) = match (side, smooth) {
        (Side::Left, true) => (dragged, follow(right)),
        (Side::Left, false) => (dragged, right),
        (Side::Right, true) => (follow(left), dragged),
        (Side::Right, false) => (left, dragged),
    };
    curve.set_tangents(frame, Tangents::Manual { left, right });
}

/// Writes `before` with `keys` moved by `frames` and `value` (display units) to
/// the document.
fn move_keys(doc: &mut Document, before: &[(NodeId, NodeData)], keys: &[CurveKey], frames: f64, value: f64) {
    for (id, data) in before {
        let mut moved = data.clone();
        for track in &mut moved.tracks {
            let param = track.param;
            #[expect(clippy::cast_possible_truncation, reason = "values are f32")]
            let by = (value / display_scale(param)) as f32;
            track.curve.shift_keys(
                |f| keys.iter().any(|k| k.node == *id && k.param == param && k.frame == f),
                frames,
                by,
            );
        }
        if let Some(node) = doc.scene.get_mut(*id)
            && node.data != moved
        {
            node.data = moved;
        }
    }
}

/// A round step (1, 2 or 5 × a power of ten) at least `at_least`.
fn nice_step(at_least: f64) -> f64 {
    if at_least.is_nan() || at_least <= 0.0 || !at_least.is_finite() {
        return 1.0;
    }
    let magnitude = 10_f64.powf(at_least.log10().floor());
    [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|m| m * magnitude)
        .find(|&s| s >= at_least)
        .unwrap_or(10.0 * magnitude)
}

/// `value` with as many decimals as `step` needs.
fn format_value(value: f64, step: f64) -> String {
    let decimals = (-step.log10().floor()).clamp(0.0, 6.0);
    #[expect(clippy::cast_possible_truncation, reason = "clamped to 0..6")]
    let decimals = decimals as usize;
    // No "-0" for a value that is zero up to rounding.
    let value = if value.abs() < step * 1e-6 { 0.0 } else { value };
    format!("{value:.decimals$}")
}

/// The track list: the selected objects and their animated values. A click
/// shows only that track, Shift+click adds or removes it, a click on an object
/// shows all tracks again.
fn track_list(ui: &mut Ui, rect: Rect, doc: &Document, editor: &mut EditorState) {
    ui.painter().rect_filled(rect, CornerRadius::ZERO, p().bg_panel);
    ui.scope_builder(UiBuilder::new().max_rect(rect.shrink2(vec2(4.0, 2.0))), |ui| {
        ScrollArea::vertical()
            .id_salt("grid4d.fcurve.tracks")
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                let shift = ui.input(|i| i.modifiers.shift);
                for &node in editor.selection.as_slice() {
                    let Some(n) = doc.scene.get(node) else { continue };
                    if n.data.tracks.is_empty() {
                        continue;
                    }
                    let name = ui.add(
                        egui::Label::new(RichText::new(&n.data.name).strong().color(p().text_strong))
                            .sense(Sense::click()),
                    );
                    if name.clicked() {
                        editor.curves.tracks.clear();
                    }
                    for param in track_order(&n.data) {
                        let picked = editor.curves.tracks.contains(&(node, param));
                        let (row, response) = ui
                            .allocate_exact_size(vec2(ui.available_width(), metrics::ROW_HEIGHT - 3.0), Sense::click());
                        if picked {
                            ui.painter().rect_filled(row, CornerRadius::same(2), p().accent_bg);
                        } else if response.hovered() {
                            ui.painter().rect_filled(row, CornerRadius::same(2), p().bg_hover);
                        }
                        let swatch = Rect::from_center_size(pos2(row.left() + 14.0, row.center().y), vec2(10.0, 3.0));
                        ui.painter().rect_filled(swatch, CornerRadius::ZERO, color(param));
                        ui.painter().text(
                            pos2(row.left() + 26.0, row.center().y),
                            Align2::LEFT_CENTER,
                            param.label(),
                            FontId::proportional(12.0),
                            p().text,
                        );
                        if response.clicked() {
                            if shift {
                                if picked {
                                    editor.curves.tracks.retain(|&t| t != (node, param));
                                } else {
                                    editor.curves.tracks.push((node, param));
                                }
                            } else {
                                editor.curves.tracks.clear();
                                editor.curves.tracks.push((node, param));
                            }
                        }
                    }
                    ui.add_space(4.0);
                }
            });
    });
}

#[cfg(test)]
mod tests {
    use egui::{Event, Modifiers, RawInput};
    use grid4d_scene::{Axis, Interpolation, Param, Tangents};

    use super::*;

    const PX: Param = Param::Position(Axis::X);

    struct Harness {
        ctx: egui::Context,
        time: f64,
        doc: Document,
        editor: EditorState,
        panel: FCurvePanel,
        cube: NodeId,
    }

    impl Harness {
        /// The demo scene's cube selected, its position X keyed 0 → 0, 15 → 100, 30 → 0.
        fn new() -> Self {
            let mut doc = Document::with_demo_scene();
            let mut editor = EditorState::default();
            let cube = doc.scene.roots()[0];
            editor.selection.set_single(cube);
            editor.panes.mark_shown(crate::workspace::Pane::FCurve);
            let data = &mut doc.scene.get_mut(cube).unwrap().data;
            data.record([PX], 0.0, 0.0);
            data.set_value_at(PX, 100.0, 15.0, true);
            data.set_value_at(PX, 0.0, 30.0, true);
            let mut h = Self {
                ctx: egui::Context::default(),
                time: 0.0,
                doc,
                editor,
                panel: FCurvePanel::default(),
                cube,
            };
            h.frame(vec![]);
            h.frame(vec![]);
            h
        }

        fn frame(&mut self, events: Vec<Event>) {
            self.time += 1.0 / 60.0;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 400.0))),
                time: Some(self.time),
                events,
                ..RawInput::default()
            };
            let (doc, editor, panel) = (&mut self.doc, &mut self.editor, &mut self.panel);
            self.ctx
                .run_ui(input, |ui| panel.show(ui, doc, editor))
                .drop_without_applying_deltas();
            crate::commands::apply_pending(doc, editor, &mut crate::ViewportPanel::new(None));
        }

        fn plot(&self) -> Rect {
            let graph = self.panel.graph.unwrap();
            Rect::from_min_max(pos2(graph.left() + VALUE_GUTTER, graph.top() + RULER_HEIGHT), graph.max)
        }

        /// Where (frame, value) is drawn.
        fn at(&self, frame: f64, value: f64) -> Pos2 {
            self.panel.view.pos(self.plot(), frame, value)
        }

        fn curve(&self) -> Curve {
            curve_of(&self.doc, self.cube, PX).unwrap().clone()
        }

        fn button(pos: Pos2, pressed: bool, modifiers: Modifiers) -> Event {
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers,
            }
        }

        fn click(&mut self, pos: Pos2, modifiers: Modifiers) {
            self.frame(vec![Event::ModifiersChanged(modifiers), Event::PointerMoved(pos)]);
            self.frame(vec![Self::button(pos, true, modifiers)]);
            self.frame(vec![Self::button(pos, false, modifiers)]);
            self.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
        }

        fn drag(&mut self, from: Pos2, to: Pos2, modifiers: Modifiers) {
            self.frame(vec![Event::ModifiersChanged(modifiers), Event::PointerMoved(from)]);
            self.frame(vec![Self::button(from, true, modifiers)]);
            for i in 1..=8 {
                let t = i as f32 / 8.0;
                self.frame(vec![Event::PointerMoved(from + (to - from) * t)]);
            }
            self.frame(vec![Self::button(to, false, modifiers)]);
            self.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
        }
    }

    #[test]
    fn the_curves_of_the_selected_objects_are_shown_and_framed() {
        let h = Harness::new();
        let plot = h.plot();
        for key in h.curve().keys() {
            let pos = h.at(key.frame, f64::from(key.value));
            assert!(plot.contains(pos), "key at {} is in view: {pos:?}", key.frame);
        }
        assert_eq!(h.panel.framed_for, [(h.cube, PX)]);
    }

    #[test]
    fn keys_are_selected_and_moved_as_one_undo_step() {
        let mut h = Harness::new();
        let middle = h.at(15.0, 100.0);
        h.click(middle, Modifiers::NONE);
        let key = CurveKey {
            node: h.cube,
            param: PX,
            frame: 15.0,
        };
        assert_eq!(h.editor.curves.keys, [key]);
        assert!(h.editor.curves.focused, "Delete now removes keys");

        // Five frames later and 20 higher.
        let to = h.at(20.0, 120.0);
        let steps = h.editor.history.undo_len();
        h.drag(middle, to, Modifiers::NONE);
        let moved = h.curve().keys()[1];
        assert_eq!(moved.frame, 20.0, "whole frames");
        assert!((moved.value - 120.0).abs() < 1.0, "{}", moved.value);
        assert_eq!(h.editor.curves.keys[0].frame, 20.0, "the selection follows");
        assert_eq!(h.editor.history.undo_len(), steps + 1);
        assert_eq!(h.editor.history.undo_label(), Some("Move Keys"));
        h.editor.commands.push(EditorCommand::Undo);
        h.frame(vec![]);
        assert_eq!(h.curve().keys()[1].frame, 15.0);

        // Shift keeps a mostly sideways drag horizontal.
        h.click(middle, Modifiers::NONE);
        h.drag(middle, h.at(25.0, 110.0), Modifiers::SHIFT);
        let moved = h.curve().keys()[1];
        assert_eq!((moved.frame, moved.value), (25.0, 100.0));
    }

    #[test]
    fn clicks_on_curves_select_or_add_keys_and_boxes_select() {
        let mut h = Harness::new();
        let on_curve = |h: &Harness, frame: f64| {
            let value = h.curve().value_at(frame).unwrap();
            h.at(frame, f64::from(value))
        };
        h.click(on_curve(&h, 7.0), Modifiers::NONE);
        assert_eq!(h.editor.curves.keys.len(), 3, "the whole curve");
        let before = h.curve().value_at(7.0).unwrap();
        h.click(on_curve(&h, 7.0), Modifiers::COMMAND);
        let curve = h.curve();
        assert_eq!(curve.keys().len(), 4, "Ctrl+click adds a key");
        assert_eq!(curve.key_at(7.0).unwrap().value, before, "on the curve");
        assert_eq!(h.editor.history.undo_label(), Some("Add Key"));
        // A box around the first two keys.
        h.drag(h.at(-2.0, -10.0), h.at(8.0, 110.0), Modifiers::NONE);
        let frames: Vec<f64> = h.editor.curves.keys.iter().map(|k| k.frame).collect();
        assert_eq!(frames, [0.0, 7.0]);
        h.click(h.at(5.0, 90.0), Modifiers::NONE);
        assert!(h.editor.curves.keys.is_empty(), "a click on empty space clears");
    }

    #[test]
    fn dragging_a_handle_makes_smooth_or_broken_manual_tangents() {
        let mut h = Harness::new();
        h.click(h.at(15.0, 100.0), Modifiers::NONE);
        // The peak's auto handles are flat, 5 frames long.
        let ((length, _), _) = h.curve().handles(1);
        let handle = h.at(15.0 - length, 100.0);
        h.drag(handle, h.at(15.0 - length, 95.0), Modifiers::NONE);
        let key = h.curve().keys()[1];
        let Tangents::Manual { left, right } = key.tangents else {
            panic!("manual tangents");
        };
        let (sl, sr) = (f64::from(left.1) / left.0, f64::from(right.1) / right.0);
        assert!(sl > 0.5, "the curve now rises into the key: {sl}");
        assert!((sl - sr).abs() < 1e-3, "smooth: {sl} vs {sr}");
        assert_eq!(h.editor.history.undo_label(), Some("Edit Tangents"));

        // Shift moves one side only.
        let handle = h.at(15.0 + right.0, 100.0 + f64::from(right.1));
        h.drag(handle, h.at(15.0 + right.0, 50.0), Modifiers::SHIFT);
        let Tangents::Manual {
            left: after,
            right: moved,
        } = h.curve().keys()[1].tangents
        else {
            panic!("manual tangents");
        };
        assert_eq!(after, left, "the other side stays");
        assert!(moved.1 < right.1 - 10.0);
    }

    #[test]
    fn the_track_list_filters_the_curves() {
        let mut h = Harness::new();
        let data = &mut h.doc.scene.get_mut(h.cube).unwrap().data;
        data.record([Param::Scale(Axis::Y)], 0.0, 0.0);
        h.frame(vec![]);
        assert_eq!(shown_tracks(&h.doc, &h.editor).len(), 2);
        h.editor.curves.tracks = vec![(h.cube, PX)];
        let shown = shown_tracks(&h.doc, &h.editor);
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].param, PX);
        h.editor.selection.clear();
        h.frame(vec![]);
        assert!(h.editor.curves.tracks.is_empty(), "a new selection resets the filter");
    }

    #[test]
    fn interpolation_and_delete_through_the_command() {
        let mut h = Harness::new();
        h.click(h.at(0.0, 0.0), Modifiers::NONE);
        h.editor.commands.push(EditorCommand::EditCurveKeys {
            op: CurveOp::Interpolation(Interpolation::Linear),
        });
        h.frame(vec![]);
        assert_eq!(h.curve().keys()[0].interpolation, Interpolation::Linear);
        assert!((h.curve().value_at(7.5).unwrap() - 50.0).abs() < 1e-3);
        assert_eq!(h.editor.history.undo_label(), Some("Set Interpolation"));
        h.editor
            .commands
            .push(EditorCommand::EditCurveKeys { op: CurveOp::Delete });
        h.frame(vec![]);
        assert_eq!(h.curve().keys().len(), 2);
        assert!(h.editor.curves.keys.is_empty());
    }

    #[test]
    fn nice_steps_and_labels() {
        assert_eq!(nice_step(0.7), 1.0);
        assert_eq!(nice_step(3.0), 5.0);
        assert_eq!(nice_step(12.0), 20.0);
        assert!((nice_step(0.03) - 0.05).abs() < 1e-12);
        assert_eq!(format_value(-1e-12, 10.0), "0");
        assert_eq!(format_value(0.25, 0.05), "0.25");
    }

    #[test]
    fn object_parameter_tracks_follow_the_transform_ones() {
        use grid4d_scene::{Key, ObjectParam, Track};
        let mut data = NodeData::new("c", grid4d_scene::ObjectKindTag::Cylinder.default_kind());
        for param in [
            Param::Object(ObjectParam::Height),
            Param::Scale(Axis::Y),
            Param::Object(ObjectParam::Radius),
        ] {
            data.tracks.push(Track {
                param,
                curve: Curve::from_keys([Key::new(0.0, 1.0)]),
            });
        }
        assert_eq!(
            track_order(&data),
            [
                Param::Scale(Axis::Y),
                Param::Object(ObjectParam::Radius),
                Param::Object(ObjectParam::Height),
            ]
        );
        assert!((display_scale(Param::Object(ObjectParam::EndRotation)) - 57.29578).abs() < 1e-3);
        assert_eq!(display_scale(Param::Object(ObjectParam::EndGrowth)), 100.0);
    }
}
