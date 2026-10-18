//! Animation timeline: a frame ruler with a scrubbable playhead, and the
//! transport row (frame field, play controls, preview range, fps).
//!
//! The current frame and playback are session state; the range, fps and loop
//! setting belong to the document, so their edits become undo steps.

use egui::{
    Align, Align2, ComboBox, CornerRadius, DragValue, FontId, Layout, Rect, RichText, Sense, Stroke, Ui, UiBuilder,
    pos2, vec2,
};
use oc4d_scene::{Playback, SceneEdit, TimeSettings, TimeState};

use crate::{
    EditorCommand, EditorState,
    icons::Icon,
    live_edit::{LiveEdit, interaction_ongoing},
    theme::palette as p,
    widgets::{IconStyle, icon_button},
};

const RULER_HEIGHT: f32 = 30.0;
const TRANSPORT_HEIGHT: f32 = 32.0;
const SIDE_MARGIN: f32 = 16.0;
const FPS_PRESETS: [u32; 6] = [24, 25, 30, 48, 50, 60];
const LEFT_GROUP_WIDTH: f32 = 135.0;
const RIGHT_GROUP_WIDTH: f32 = 235.0;

#[derive(Debug, Default)]
pub struct TimelinePanel {
    scrubbing: bool,
    settings_edit: LiveEdit<(), TimeSettings>,
}

impl TimelinePanel {
    pub fn show(&mut self, ui: &mut Ui, time: &mut TimeState, editor: &mut EditorState) {
        self.settings_edit.begin((), &time.settings());
        ui.spacing_mut().item_spacing.y = 0.0;
        self.ruler(ui, time, editor);
        transport(ui, time);
        if let Some(((), before)) = self.settings_edit.end((), &time.settings(), interaction_ongoing(ui)) {
            editor.commands.push(EditorCommand::CommitLive {
                inverse: SceneEdit::SetTime(before),
                label: "Project Settings",
            });
        }
    }

    fn ruler(&mut self, ui: &mut Ui, time: &mut TimeState, editor: &mut EditorState) {
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), RULER_HEIGHT), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::ZERO, p::BG_WELL);

        let track = rect.shrink2(vec2(SIDE_MARGIN, 0.0));
        let start = time.start();
        let end = time.end();
        let px_per_frame = track.width() / (end - start).max(1) as f32;
        let frame_x = |f: f64| track.left() + (f - f64::from(start)) as f32 * px_per_frame;

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
            let x = frame_x(f64::from(f));
            let major = f % label_step == 0;
            let tick_h = if major { 9.0 } else { 4.0 };
            let color = if major { p::TEXT_FAINT } else { p::SEPARATOR };
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
                    p::TEXT_DIM,
                );
            }
            f += tick_step;
        }

        // Playhead.
        let x = frame_x(time.frame_f64().clamp(f64::from(start), f64::from(end)));
        painter.line_segment(
            [pos2(x, rect.top()), pos2(x, rect.bottom())],
            Stroke::new(1.5, p::ACCENT),
        );
        let label = painter.layout_no_wrap(time.frame().to_string(), small, p::TEXT_STRONG);
        let tag = Rect::from_center_size(pos2(x, rect.bottom() - 8.0), label.size() + vec2(10.0, 3.0));
        painter.rect_filled(tag, CornerRadius::same(2), p::ACCENT);
        painter.galley(tag.center() - label.size() * 0.5, label, p::TEXT_STRONG);

        // Scrubbing.
        if (response.clicked() || response.dragged())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            if !self.scrubbing {
                time.stop();
                self.scrubbing = true;
            }
            #[expect(clippy::cast_possible_truncation, reason = "pointer maps into the i32 frame range")]
            let frame = ((pointer.x - track.left()) / px_per_frame).round() as i32 + start;
            time.set_frame(frame);
        }
        if !response.is_pointer_button_down_on() {
            self.scrubbing = false;
        }
        if response.hovered() {
            editor.status_hint = Some("Timeline: click or drag to scrub · F / G step frames · F8 play");
        }
    }
}

fn transport(ui: &mut Ui, time: &mut TimeState) {
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
            .on_hover_text("Current frame")
            .changed()
        {
            time.stop();
            time.set_frame(frame);
        }
        ui.label(
            RichText::new(format!("{:.2} s", time.seconds()))
                .color(p::TEXT_DIM)
                .size(11.5),
        );
    });

    // Centre: transport buttons.
    ui.scope_builder(UiBuilder::new().max_rect(center).layout(row_layout), |ui| {
        ui.spacing_mut().item_spacing.x = 1.0;
        const BUTTONS: usize = 8;
        const GAP: f32 = 6.0;
        let size = 25.0;
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
            (Icon::Pause, "Stop (F7)")
        } else {
            (Icon::PlayForward, "Play Forwards (F8)")
        };
        let play_style = IconStyle {
            selected: forward,
            tint: Some(if forward { p::TEXT_STRONG } else { p::ENABLED }),
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
            enabled: false,
            tint: Some(p::RECORD),
            ..style
        };
        icon_button(
            ui,
            Icon::Record,
            record,
            "Record Active Objects (F9) — keyframes planned",
        );
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
            ComboBox::from_id_salt("oc4d.timeline.fps")
                .width(64.0)
                .selected_text(format!("{fps} fps"))
                .show_ui(ui, |ui| {
                    for preset in FPS_PRESETS {
                        ui.selectable_value(&mut fps, preset, format!("{preset} fps"));
                    }
                });
            if fps != time.fps() {
                time.set_fps(fps);
            }
            ui.add_space(8.0);

            let (mut start, mut end) = (time.start(), time.end());
            let end_changed = ui
                .add_sized([58.0, 20.0], DragValue::new(&mut end).speed(0.25).suffix(" F"))
                .on_hover_text("Preview max")
                .changed();
            ui.label(RichText::new("to").color(p::TEXT_DIM));
            let start_changed = ui
                .add_sized([58.0, 20.0], DragValue::new(&mut start).speed(0.25).suffix(" F"))
                .on_hover_text("Preview min")
                .changed();
            if start_changed || end_changed {
                time.set_range(start, end);
            }
        },
    );
}
