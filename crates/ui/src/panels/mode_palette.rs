//! The vertical palette on the left edge: Make Editable, then the modes
//! (Model / Points / Edges / Polygons) and snapping.

use egui::Ui;
use grid4d_scene::Document;

use crate::{
    EditMode, EditorCommand, EditorState, commands,
    icons::Icon,
    widgets::{IconStyle, icon_button, snap_menu},
};

pub fn show(ui: &mut Ui, doc: &Document, editor: &mut EditorState) {
    ui.vertical_centered(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.add_space(6.0);
        // Like C4D, the palette starts with Make Editable.
        let convert = IconStyle {
            enabled: commands::can_make_editable(doc, &editor.selection),
            ..IconStyle::default()
        };
        if icon_button(ui, Icon::MakeEditable, convert, "Make Editable (C)").clicked() {
            editor.commands.push(EditorCommand::MakeEditable);
        }
        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);
        for mode in EditMode::ALL {
            let style = IconStyle {
                selected: editor.mode == mode,
                ..IconStyle::default()
            };
            if icon_button(ui, mode_icon(mode), style, mode_tooltip(mode)).clicked() {
                // Shift/Ctrl+click from one component mode to another converts the selection.
                let modifiers = ui.input(|i| i.modifiers);
                let converts = (modifiers.shift || modifiers.command)
                    && mode != editor.mode
                    && mode.component().is_some()
                    && editor.mode.component().is_some();
                if converts {
                    editor.commands.push(EditorCommand::ConvertSelection {
                        to: mode,
                        tolerant: modifiers.shift,
                    });
                } else {
                    editor.mode = mode;
                }
            }
        }
        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);
        let snap = IconStyle {
            selected: editor.snap.enabled,
            ..IconStyle::default()
        };
        let button = icon_button(
            ui,
            Icon::Snap,
            snap,
            "Enable Snapping (Shift+S) · right-click: snap settings",
        );
        if button.clicked() {
            editor.snap.enabled = !editor.snap.enabled;
        }
        button.context_menu(|ui| snap_menu(ui, &mut editor.snap));
    });
}

const fn mode_icon(mode: EditMode) -> Icon {
    match mode {
        EditMode::Model => Icon::ModelMode,
        EditMode::Points => Icon::PointsMode,
        EditMode::Edges => Icon::EdgesMode,
        EditMode::Polygons => Icon::PolygonsMode,
    }
}

const fn mode_tooltip(mode: EditMode) -> &'static str {
    match mode {
        EditMode::Model => "Model mode",
        EditMode::Points => {
            "Points mode · Shift+click converts the selection (touching), Ctrl+click strictly (fully covered)"
        }
        EditMode::Edges => {
            "Edges mode · Shift+click converts the selection (touching), Ctrl+click strictly (fully covered)"
        }
        EditMode::Polygons => {
            "Polygons mode · Shift+click converts the selection (touching), Ctrl+click strictly (fully covered)"
        }
    }
}

#[cfg(test)]
mod tests {
    use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, vec2};

    use super::*;

    fn frame(ctx: &egui::Context, doc: &Document, editor: &mut EditorState, events: Vec<Event>) {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(60.0, 500.0))),
            events,
            ..RawInput::default()
        };
        ctx.run_ui(input, |ui| show(ui, doc, editor))
            .drop_without_applying_deltas();
    }

    #[test]
    fn a_right_click_on_the_snap_button_shows_the_snap_settings() {
        let ctx = egui::Context::default();
        let doc = Document::default();
        let mut editor = EditorState::default();
        frame(&ctx, &doc, &mut editor, vec![]);
        // Right-click down the palette until a menu opens: only the snap button has one.
        let opened = (0..120).map(|i| 4.0 * f32::from(u8::try_from(i).unwrap())).find(|&y| {
            let pos = Pos2::new(30.0, y);
            let button = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Secondary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            frame(&ctx, &doc, &mut editor, vec![Event::PointerMoved(pos)]);
            frame(&ctx, &doc, &mut editor, vec![button(true)]);
            frame(&ctx, &doc, &mut editor, vec![button(false)]);
            frame(&ctx, &doc, &mut editor, vec![]);
            egui::Popup::is_any_open(&ctx)
        });
        assert!(opened.is_some(), "the snap button has a context menu");
        assert!(!editor.snap.enabled, "a right-click does not toggle snapping");
    }
}
