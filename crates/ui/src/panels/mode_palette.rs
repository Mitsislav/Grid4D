//! The vertical mode palette on the left edge (Model / Points / Edges / Polygons).

use egui::Ui;

use crate::{
    EditMode, EditorState,
    icons::Icon,
    widgets::{IconStyle, icon_button},
};

pub fn show(ui: &mut Ui, editor: &mut EditorState) {
    ui.vertical_centered(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.add_space(6.0);
        for mode in EditMode::ALL {
            let style = IconStyle {
                selected: editor.mode == mode,
                ..IconStyle::default()
            };
            if icon_button(ui, mode_icon(mode), style, mode_tooltip(mode)).clicked() {
                editor.mode = mode;
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
        EditMode::Points => "Points mode — component editing planned",
        EditMode::Edges => "Edges mode — component editing planned",
        EditMode::Polygons => "Polygons mode — component editing planned",
    }
}
