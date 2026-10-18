//! The vertical palette on the left edge: Make Editable, then the modes
//! (Model / Points / Edges / Polygons) and snapping.

use egui::Ui;
use grid4d_scene::Document;

use crate::{
    EditMode, EditorCommand, EditorState, commands,
    icons::Icon,
    widgets::{IconStyle, icon_button},
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
            selected: editor.snapping,
            ..IconStyle::default()
        };
        if icon_button(
            ui,
            Icon::Snap,
            snap,
            "Enable Snapping (Shift+S) — snapping logic planned",
        )
        .clicked()
        {
            editor.snapping = !editor.snapping;
        }
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
