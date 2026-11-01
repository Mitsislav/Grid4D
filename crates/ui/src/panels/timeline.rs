//! Animation timeline: a frame ruler with a scrubbable playhead, the key strip
//! (keys of the selected objects: select, move, delete, Ctrl+click records) and
//! the transport row (frame field, play controls, Record and Autokeying,
//! preview range, fps).
//!
//! The current frame and playback are session state; the range, fps and loop
//! setting belong to the document, so their edits become undo steps. Key edits
//! are commands (one undo step each), applied after layout.

use egui::{
    Align, Align2, Color32, ComboBox, CornerRadius, DragValue, FontId, Layout, Rect, Response, RichText, Sense, Stroke,
    StrokeKind, Ui, UiBuilder, pos2, vec2,
};
use grid4d_scene::{Document, NodeId, Playback, SceneEdit, TimeSettings, TimeState};

use crate::i18n::{key, tr};
use crate::{
    EditorCommand, EditorState,
    icons::Icon,
    keys,
    live_edit::{LiveEdit, interaction_ongoing},
    theme::{metrics, palette as p},
    widgets::{IconStyle, icon_button},
};

const RULER_HEIGHT: f32 = 30.0;
const KEY_STRIP_HEIGHT: f32 = 16.0;
const TRANSPORT_HEIGHT: f32 = 32.0;
/// Height of the whole panel.
pub const HEIGHT: f32 = metrics::HEADER_HEIGHT + RULER_HEIGHT + KEY_STRIP_HEIGHT + TRANSPORT_HEIGHT + 4.0;
const SIDE_MARGIN: f32 = 16.0;
const FPS_PRESETS: [u32; 6] = [24, 25, 30, 48, 50, 60];
const LEFT_GROUP_WIDTH: f32 = 135.0;
const RIGHT_GROUP_WIDTH: f32 = 235.0;
/// A key mark is hit within this many points of the pointer.
const KEY_PICK: f32 = 5.0;

/// Frames ↔ x positions of the ruler and the key strip.
#[derive(Clone, Copy, Debug)]
struct FrameScale {
    left: f32,
    start: i32,
    end: i32,
    px_per_frame: f32,
}

impl FrameScale {
    fn new(rect: Rect, time: &TimeState) -> Self {
        let track = rect.shrink2(vec2(SIDE_MARGIN, 0.0));
        let (start, end) = (time.start(), time.end());
        Self {
            left: track.left(),
            start,
            end,
            px_per_frame: track.width() / (end - start).max(1) as f32,
        }
    }

    fn x(&self, frame: f64) -> f32 {
        self.left + (frame - f64::from(self.start)) as f32 * self.px_per_frame
    }

    /// The whole frame nearest to `x`.
    fn frame(&self, x: f32) -> i32 {
        #[expect(clippy::cast_possible_truncation, reason = "pointer maps into the i32 frame range")]
        let offset = ((x - self.left) / self.px_per_frame).round() as i32;
        self.start.saturating_add(offset)
    }

    /// `frame` lies in the shown range.
    fn shows(&self, frame: f64) -> bool {
        frame >= f64::from(self.start) && frame <= f64::from(self.end)
    }
}

/// A drag in the key strip.
#[derive(Clone, Copy, Debug, PartialEq)]
enum KeyDrag {
    /// Moving the selected keys: pointer x where it started, whole frames so far.
    Move { from: f32, by: i32 },
    /// Selecting the keys between two x positions; `add` keeps the selection.
    Select { from: f32, to: f32, add: bool },
}

#[derive(Debug, Default)]
pub struct TimelinePanel {
    scrubbing: bool,
    settings_edit: LiveEdit<(), TimeSettings>,
    /// Frames with keys of the selected objects (refilled every frame).
    frames: Vec<f64>,
    /// The object selection the key selection belongs to.
    seen_selection: Vec<NodeId>,
    key_drag: Option<KeyDrag>,
    /// Where the key strip was last frame (tests point at keys with it).
    #[cfg(test)]
    strip: Option<Rect>,
}

impl TimelinePanel {
    pub fn show(&mut self, ui: &mut Ui, doc: &mut Document, editor: &mut EditorState) {
        self.settings_edit.begin((), &doc.time.settings());
        ui.spacing_mut().item_spacing.y = 0.0;
        let (ruler, key_strip) = {
            let width = ui.available_width();
            let (ruler, _) = ui.allocate_exact_size(vec2(width, RULER_HEIGHT), Sense::hover());
            let (strip, _) = ui.allocate_exact_size(vec2(width, KEY_STRIP_HEIGHT), Sense::hover());
            (ruler, strip)
        };
        let scale = FrameScale::new(ruler, &doc.time);
        #[cfg(test)]
        {
            self.strip = Some(key_strip);
        }
        self.ruler(ui, ruler, scale, &mut doc.time, editor);
        self.key_strip(ui, key_strip, scale, doc, editor);
        ui.add_space(4.0);
        transport(ui, &mut doc.time, editor);
        if let Some(((), before)) = self
            .settings_edit
            .end((), &doc.time.settings(), interaction_ongoing(ui))
        {
            editor.commands.push(EditorCommand::CommitLive {
                inverse: SceneEdit::SetTime(before),
                label: "Project Settings",
            });
        }
    }

    fn ruler(&mut self, ui: &mut Ui, rect: Rect, scale: FrameScale, time: &mut TimeState, editor: &mut EditorState) {
        let response = ui.interact(rect, egui::Id::unique("grid4d.timeline.ruler"), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::ZERO, p().bg_well);
        let (start, end, px_per_frame) = (scale.start, scale.end, scale.px_per_frame);

        // Pick a label step that keeps labels ≥ 44 px apart, and a tick step under it.
        let label_step = [1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000]
            .into_iter()
            .find(|&s| s as f32 * px_per_frame >= 44.0)
            .unwrap_or(10_000);
        let tick_step = if px_per_frame >= 6.0 {
            1
        } else {
            (label_step / 5).max(1)
        };

        let small = FontId::proportional(10.5);
        let mut f = start + (tick_step - start.rem_euclid(tick_step)) % tick_step;
        while f <= end {
            let x = scale.x(f64::from(f));
            let major = f % label_step == 0;
            let tick_h = if major { 9.0 } else { 4.0 };
            let color = if major { p().text_faint } else { p().separator };
            painter.line_segment(
                [pos2(x, rect.bottom() - tick_h), pos2(x, rect.bottom())],
                Stroke::new(1.0, color),
            );
            if major {
                painter.text(
                    pos2(x + 3.0, rect.top() + 4.0),
                    Align2::LEFT_TOP,
                    f,
                    small.clone(),
                    p().text_dim,
                );
            }
            f += tick_step;
        }

        // The simulated (cached) frames.
        if let Some((first, last)) = editor.simulation.cached() {
            let (from, to) = (first.max(start), last.min(end));
            if from <= to {
                let bar = Rect::from_x_y_ranges(
                    scale.x(f64::from(from))..=scale.x(f64::from(to)).max(scale.x(f64::from(from)) + 2.0),
                    rect.bottom() - 3.0..=rect.bottom(),
                );
                painter.rect_filled(bar, CornerRadius::ZERO, p().sim_cache);
            }
        }

        // Playhead.
        let x = scale.x(time.frame_f64().clamp(f64::from(start), f64::from(end)));
        painter.line_segment(
            [pos2(x, rect.top()), pos2(x, rect.bottom())],
            Stroke::new(1.5, p().accent),
        );
        let label = painter.layout_no_wrap(time.frame().to_string(), small, p().text_strong);
        let tag = Rect::from_center_size(pos2(x, rect.bottom() - 8.0), label.size() + vec2(10.0, 3.0));
        painter.rect_filled(tag, CornerRadius::same(2), p().accent);
        painter.galley(tag.center() - label.size() * 0.5, label, p().text_strong);

        // Scrubbing.
        if (response.clicked() || response.dragged())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            if !self.scrubbing {
                time.stop();
                self.scrubbing = true;
            }
            time.set_frame(scale.frame(pointer.x));
        }
        if !response.is_pointer_button_down_on() {
            self.scrubbing = false;
        }
        if response.hovered() {
            editor.status_hint = Some("Timeline: click or drag to scrub · F / G step frames · F8 play");
        }
    }

    /// The keys of the selected objects, one mark per frame.
    fn key_strip(&mut self, ui: &mut Ui, rect: Rect, scale: FrameScale, doc: &Document, editor: &mut EditorState) {
        let response = ui.interact(rect, egui::Id::unique("grid4d.timeline.keys"), Sense::click_and_drag());
        // A new object selection starts a new key selection.
        if self.seen_selection.as_slice() != editor.selection.as_slice() {
            self.seen_selection.clear();
            self.seen_selection.extend_from_slice(editor.selection.as_slice());
            editor.keys.frames.clear();
            self.key_drag = None;
        }
        keys::key_frames(&doc.scene, &editor.selection, &mut self.frames);
        // Keys that went away (undo, a deleted track) are no longer selected.
        if self.key_drag.is_none() && editor.keys.frames.iter().any(|f| !self.has_key(*f)) {
            let frames = &self.frames;
            editor
                .keys
                .frames
                .retain(|f| frames.binary_search_by(|k| k.total_cmp(f)).is_ok());
        }
        // Delete works on keys after a click in the strip, on objects after one elsewhere.
        if ui.input(|i| i.pointer.any_pressed()) {
            editor.keys.focused = response.hovered();
        }

        self.interact(ui, &response, scale, editor);
        // The playhead runs on through the strip, under the keys.
        let playhead = scale.x(doc.time.frame_f64().clamp(f64::from(scale.start), f64::from(scale.end)));
        self.paint_keys(ui, rect, scale, playhead, editor);

        response.context_menu(|ui| {
            let any = !editor.keys.frames.is_empty();
            if ui
                .add_enabled(any, egui::Button::new(tr("Delete Keys")).shortcut_text("Delete"))
                .clicked()
            {
                editor.commands.push(EditorCommand::DeleteKeys);
                ui.close();
            }
            if ui
                .add_enabled(
                    !editor.selection.is_empty(),
                    egui::Button::new(tr("Record Active Objects")).shortcut_text("F9"),
                )
                .clicked()
            {
                editor.commands.push(EditorCommand::RecordKeys { at: doc.time.frame() });
                ui.close();
            }
        });
        if response.hovered() {
            editor.status_hint = Some(
                "Keys of the selected objects: click or drag to select · drag a key to move · Delete removes · Ctrl+click records here",
            );
        }
    }

    fn has_key(&self, frame: f64) -> bool {
        self.frames.binary_search_by(|k| k.total_cmp(&frame)).is_ok()
    }

    /// The key mark under `x`, if any.
    fn key_under(&self, scale: FrameScale, x: f32) -> Option<f64> {
        self.frames
            .iter()
            .copied()
            .filter(|&f| scale.shows(f))
            .map(|f| (f, (scale.x(f) - x).abs()))
            .filter(|&(_, d)| d <= KEY_PICK)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(f, _)| f)
    }

    fn interact(&mut self, ui: &Ui, response: &Response, scale: FrameScale, editor: &mut EditorState) {
        let modifiers = ui.input(|i| i.modifiers);
        if response.drag_started_by(egui::PointerButton::Primary)
            && let Some(press) = ui.input(|i| i.pointer.press_origin())
        {
            self.key_drag = Some(match self.key_under(scale, press.x) {
                Some(frame) => {
                    if !editor.keys.contains(frame) {
                        if !modifiers.shift {
                            editor.keys.frames.clear();
                        }
                        editor.keys.insert(frame);
                    }
                    KeyDrag::Move { from: press.x, by: 0 }
                }
                None => KeyDrag::Select {
                    from: press.x,
                    to: press.x,
                    add: modifiers.shift,
                },
            });
        }
        if let (Some(drag), Some(pointer)) = (&mut self.key_drag, response.interact_pointer_pos()) {
            match drag {
                KeyDrag::Move { from, by } => {
                    #[expect(clippy::cast_possible_truncation, reason = "a drag across the strip")]
                    let frames = ((pointer.x - *from) / scale.px_per_frame).round() as i32;
                    *by = frames;
                }
                KeyDrag::Select { to, .. } => *to = pointer.x,
            }
        }
        if response.drag_stopped()
            && let Some(drag) = self.key_drag.take()
        {
            match drag {
                KeyDrag::Move { by, .. } => {
                    if by != 0 && !editor.keys.frames.is_empty() {
                        editor.commands.push(EditorCommand::MoveKeys { by });
                    }
                }
                KeyDrag::Select { from, to, add } => {
                    if !add {
                        editor.keys.frames.clear();
                    }
                    let (lo, hi) = (from.min(to) - KEY_PICK, from.max(to) + KEY_PICK);
                    for &frame in &self.frames {
                        let x = scale.x(frame);
                        if scale.shows(frame) && x >= lo && x <= hi {
                            editor.keys.insert(frame);
                        }
                    }
                }
            }
        }
        if response.clicked()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            if modifiers.command {
                // Records at the clicked frame; the time stays where it is.
                editor.commands.push(EditorCommand::RecordKeys {
                    at: scale.frame(pointer.x),
                });
            } else {
                match self.key_under(scale, pointer.x) {
                    Some(frame) if modifiers.shift => editor.keys.toggle(frame),
                    Some(frame) => {
                        editor.keys.frames.clear();
                        editor.keys.insert(frame);
                    }
                    None if !modifiers.shift => editor.keys.frames.clear(),
                    None => {}
                }
            }
        }
    }

    fn paint_keys(&self, ui: &Ui, rect: Rect, scale: FrameScale, playhead: f32, editor: &EditorState) {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::ZERO, p().bg_header);
        painter.line_segment(
            [pos2(playhead, rect.top()), pos2(playhead, rect.bottom())],
            Stroke::new(1.5, p().accent),
        );
        let moved = match self.key_drag {
            Some(KeyDrag::Move { by, .. }) => by,
            _ => 0,
        };
        let mark = |x: f32, color: Color32| {
            let r = Rect::from_center_size(pos2(x, rect.center().y), vec2(5.0, 10.0));
            painter.rect_filled(r, CornerRadius::same(1), color);
            painter.rect_stroke(
                r,
                CornerRadius::same(1),
                Stroke::new(1.0, p().border),
                StrokeKind::Outside,
            );
        };
        for &frame in &self.frames {
            let selected = editor.keys.contains(frame);
            if selected && moved != 0 {
                // Where it was, faintly; where it goes, below.
                mark(scale.x(frame), p().key.gamma_multiply(0.3));
                continue;
            }
            if scale.shows(frame) {
                mark(scale.x(frame), if selected { p().highlight } else { p().key });
            }
        }
        if moved != 0 {
            for &frame in &editor.keys.frames {
                let to = frame + f64::from(moved);
                if scale.shows(to) {
                    mark(scale.x(to), p().highlight);
                }
            }
        }
        if let Some(KeyDrag::Select { from, to, .. }) = self.key_drag {
            let band = Rect::from_min_max(pos2(from.min(to), rect.top()), pos2(from.max(to), rect.bottom()));
            painter.rect_filled(band, CornerRadius::ZERO, p().accent.gamma_multiply(0.25));
            painter.rect_stroke(
                band,
                CornerRadius::ZERO,
                Stroke::new(1.0, p().accent),
                StrokeKind::Inside,
            );
        }
    }
}

fn transport(ui: &mut Ui, time: &mut TimeState, editor: &mut EditorState) {
    // Fixed-width side groups; the transport buttons are centred in what remains,
    // so nothing overlaps down to the minimum window width.
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), TRANSPORT_HEIGHT), Sense::hover());
    let left = Rect::from_min_size(row.min + vec2(10.0, 0.0), vec2(LEFT_GROUP_WIDTH, row.height()));
    let right = Rect::from_min_max(
        pos2(row.right() - 10.0 - RIGHT_GROUP_WIDTH, row.top()),
        pos2(row.right() - 10.0, row.bottom()),
    );
    let center = Rect::from_min_max(
        pos2(left.right(), row.top()),
        pos2(right.left().max(left.right()), row.bottom()),
    );
    let row_layout = Layout::left_to_right(Align::Center);

    // Left: current frame + time readout.
    ui.scope_builder(UiBuilder::new().max_rect(left).layout(row_layout), |ui| {
        let mut frame = time.frame();
        let field = DragValue::new(&mut frame)
            .speed(0.25)
            .range(time.start()..=time.end())
            .suffix(" F");
        if ui
            .add_sized([72.0, 20.0], field)
            .on_hover_text(tr("Current frame"))
            .changed()
        {
            time.stop();
            time.set_frame(frame);
        }
        ui.label(
            RichText::new(format!("{:.2} s", time.seconds()))
                .color(p().text_dim)
                .size(11.5),
        );
    });

    // Centre: transport buttons.
    ui.scope_builder(UiBuilder::new().max_rect(center).layout(row_layout), |ui| {
        ui.spacing_mut().item_spacing.x = 1.0;
        const BUTTONS: usize = 9;
        const GAP: f32 = 6.0;
        // Smaller buttons in a narrow window, so they never run into the range fields.
        let size = ((ui.available_width() - GAP - (BUTTONS - 1) as f32) / BUTTONS as f32).clamp(18.0, 25.0);
        let group = BUTTONS as f32 * size + (BUTTONS - 1) as f32 + GAP;
        ui.add_space(((ui.available_width() - group) * 0.5).max(0.0));

        let style = IconStyle {
            size,
            ..IconStyle::default()
        };
        if icon_button(ui, Icon::GoToStart, style, "Go to Start").clicked() {
            time.go_to_start();
        }
        if icon_button(ui, Icon::StepBack, style, "Go to Previous Frame (F)").clicked() {
            time.stop();
            time.step(-1);
        }
        let backward = time.playback() == Playback::Backward;
        let back_style = IconStyle {
            selected: backward,
            ..style
        };
        if icon_button(ui, Icon::PlayBackward, back_style, "Play Backwards (F6)").clicked() {
            if backward { time.stop() } else { time.play_backward() }
        }
        let forward = time.playback() == Playback::Forward;
        let (icon, tip) = if forward {
            (Icon::Pause, key("Stop (F7)"))
        } else {
            (Icon::PlayForward, key("Play Forwards (F8)"))
        };
        let play_style = IconStyle {
            selected: forward,
            tint: Some(if forward { p().text_strong } else { p().enabled }),
            ..style
        };
        if icon_button(ui, icon, play_style, tip).clicked() {
            if forward { time.stop() } else { time.play_forward() }
        }
        if icon_button(ui, Icon::StepForward, style, "Go to Next Frame (G)").clicked() {
            time.stop();
            time.step(1);
        }
        if icon_button(ui, Icon::GoToEnd, style, "Go to End").clicked() {
            time.go_to_end();
        }
        ui.add_space(GAP);
        let record = IconStyle {
            enabled: !editor.selection.is_empty(),
            tint: Some(p().record),
            ..style
        };
        if icon_button(
            ui,
            Icon::Record,
            record,
            "Record Active Objects (F9): keys for the selected objects at this frame",
        )
        .clicked()
        {
            editor.commands.push(EditorCommand::RecordKeys { at: time.frame() });
        }
        let autokey = editor.animation.autokey;
        let autokey_style = IconStyle {
            selected: autokey,
            tint: Some(if autokey { p().record } else { p().text }),
            ..style
        };
        if icon_button(
            ui,
            Icon::Autokey,
            autokey_style,
            "Autokeying (Ctrl+F9): changes to animated parameters become keys",
        )
        .clicked()
        {
            editor.animation.autokey = !autokey;
        }
        let loop_style = IconStyle {
            selected: time.looping,
            ..style
        };
        if icon_button(ui, Icon::Loop, loop_style, "Loop playback").clicked() {
            time.looping = !time.looping;
        }
    });

    // Right: preview range and frame rate.
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(right)
            .layout(Layout::right_to_left(Align::Center)),
        |ui| {
            let mut fps = time.fps();
            let combo = ComboBox::from_id_salt("grid4d.timeline.fps")
                .width(64.0)
                .selected_text(format!("{fps} fps"))
                .show_ui(ui, |ui| {
                    for preset in FPS_PRESETS {
                        ui.selectable_value(&mut fps, preset, format!("{preset} fps"));
                    }
                })
                .response;
            // Named for screen readers (it shows only its value).
            ui.ctx()
                .accesskit_node_builder(combo.id, |node| node.set_label(tr("Frame Rate")));
            if fps != time.fps() {
                time.set_fps(fps);
            }
            ui.add_space(8.0);

            let (mut start, mut end) = (time.start(), time.end());
            let end_changed = ui
                .add_sized([58.0, 20.0], DragValue::new(&mut end).speed(0.25).suffix(" F"))
                .on_hover_text(tr("Preview max"))
                .changed();
            ui.label(RichText::new(tr("to")).color(p().text_dim));
            let start_changed = ui
                .add_sized([58.0, 20.0], DragValue::new(&mut start).speed(0.25).suffix(" F"))
                .on_hover_text(tr("Preview min"))
                .changed();
            if start_changed || end_changed {
                time.set_range(start, end);
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use egui::{Event, Modifiers, PointerButton, Pos2, RawInput};
    use grid4d_scene::{Axis, Param};

    use super::*;

    struct Harness {
        ctx: egui::Context,
        time: f64,
        doc: Document,
        editor: EditorState,
        panel: TimelinePanel,
    }

    impl Harness {
        /// The demo scene, its cube selected, with position X keys at 0 and 30.
        fn new() -> Self {
            let mut doc = Document::with_demo_scene();
            let mut editor = EditorState::default();
            let cube = doc.scene.roots()[0];
            editor.selection.set_single(cube);
            let data = &mut doc.scene.get_mut(cube).unwrap().data;
            for frame in [0.0, 30.0] {
                data.record([Param::Position(Axis::X)], 0.0, frame);
            }
            let mut h = Self {
                ctx: egui::Context::default(),
                time: 0.0,
                doc,
                editor,
                panel: TimelinePanel::default(),
            };
            h.frame(vec![]);
            h
        }

        fn frame(&mut self, events: Vec<Event>) {
            self.time += 1.0 / 60.0;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 200.0))),
                time: Some(self.time),
                events,
                ..RawInput::default()
            };
            let (doc, editor, panel) = (&mut self.doc, &mut self.editor, &mut self.panel);
            self.ctx
                .run_ui(input, |ui| panel.show(ui, doc, editor))
                .drop_without_applying_deltas();
        }

        /// The middle of the key strip at `frame`.
        fn at(&self, frame: f64) -> Pos2 {
            let strip = self.panel.strip.unwrap();
            let scale = FrameScale::new(strip, &self.doc.time);
            pos2(scale.x(frame), strip.center().y)
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

        fn drag(&mut self, from: Pos2, to: Pos2) {
            self.frame(vec![Event::PointerMoved(from)]);
            self.frame(vec![Self::button(from, true, Modifiers::NONE)]);
            for i in 1..=8 {
                let t = i as f32 / 8.0;
                self.frame(vec![Event::PointerMoved(from + (to - from) * t)]);
            }
            self.frame(vec![Self::button(to, false, Modifiers::NONE)]);
            self.frame(vec![]);
        }
    }

    #[test]
    fn keys_are_selected_moved_and_recorded_in_the_key_strip() {
        let mut h = Harness::new();
        assert_eq!(h.panel.frames, [0.0, 30.0], "the selected cube's keys");

        h.click(h.at(30.0), Modifiers::NONE);
        assert_eq!(h.editor.keys.frames, [30.0]);
        assert!(h.editor.keys.focused, "Delete now removes keys");
        h.click(h.at(0.0), Modifiers::SHIFT);
        assert_eq!(h.editor.keys.frames, [0.0, 30.0], "Shift adds");
        h.click(h.at(50.0), Modifiers::NONE);
        assert!(h.editor.keys.frames.is_empty(), "empty space clears");

        // Dragging a key moves the selection by whole frames, on release.
        h.drag(h.at(30.0), h.at(40.0));
        assert_eq!(h.editor.keys.frames, [30.0], "the dragged key was selected");
        assert_eq!(h.editor.commands.pending(), [EditorCommand::MoveKeys { by: 10 }]);

        // Dragging over empty space selects the keys inside.
        let mut h = Harness::new();
        h.drag(h.at(-1.0), h.at(45.0));
        assert_eq!(h.editor.keys.frames, [0.0, 30.0]);
        assert!(h.editor.commands.is_empty());

        // Ctrl+click records at the clicked frame without moving the time.
        h.click(h.at(60.0), Modifiers::COMMAND);
        assert_eq!(h.editor.commands.pending(), [EditorCommand::RecordKeys { at: 60 }]);
        assert_eq!(h.doc.time.frame(), 0);
    }

    #[test]
    fn a_click_on_the_ruler_sets_the_frame() {
        let mut h = Harness::new();
        let on_ruler = h.at(30.0) - vec2(0.0, KEY_STRIP_HEIGHT);
        h.click(on_ruler, Modifiers::NONE);
        assert_eq!(h.doc.time.frame(), 30);
    }

    #[test]
    fn a_new_object_selection_clears_the_key_selection() {
        let mut h = Harness::new();
        h.editor.keys.frames = vec![30.0];
        h.frame(vec![]);
        assert_eq!(h.editor.keys.frames, [30.0]);
        h.editor.selection.clear();
        h.frame(vec![]);
        assert!(h.editor.keys.frames.is_empty());
        assert!(h.panel.frames.is_empty());
    }
}
