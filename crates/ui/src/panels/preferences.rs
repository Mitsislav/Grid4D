//! Edit ▸ Preferences…: the user's settings, written at once (they are not
//! document data, so not undoable). Spec: `docs/reference/workspace-preferences.md`.

use egui::{ComboBox, DragValue, Grid, Id, Ui, Window};
use grid4d_render::ShadingMode;

use crate::{
    EditorCommand, EditorState,
    panels::viewport::shading_label,
    preferences::{Preferences, Units},
    widgets::{attr_label, section_heading},
};

const LABEL_WIDTH: f32 = 120.0;
const WINDOW_WIDTH: f32 = 340.0;

pub fn show(ctx: &egui::Context, editor: &mut EditorState) {
    if !editor.show_preferences {
        return;
    }
    let mut open = true;
    let mut apply_views = false;
    Window::new("Preferences")
        .id(Id::unique("grid4d.preferences"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(WINDOW_WIDTH)
        .default_pos(ctx.content_rect().center() - egui::vec2(WINDOW_WIDTH * 0.5, 200.0))
        .show(ctx, |ui| {
            ui.set_width(WINDOW_WIDTH);
            apply_views = contents(ui, &mut editor.preferences);
        });
    if apply_views {
        editor.commands.push(EditorCommand::ApplyViewDefaults);
    }
    editor.show_preferences = open;
}

/// The settings; returns whether Apply to Views was clicked.
fn contents(ui: &mut Ui, prefs: &mut Preferences) -> bool {
    section_heading(ui, "Units");
    Grid::new("grid4d.preferences.units").num_columns(2).show(ui, |ui| {
        attr_label(ui, "Length Unit", LABEL_WIDTH);
        ComboBox::from_id_salt("grid4d.preferences.length_unit")
            .selected_text(format!("{} ({})", prefs.units.label(), prefs.units.symbol()))
            .show_ui(ui, |ui| {
                for units in Units::ALL {
                    ui.selectable_value(
                        &mut prefs.units,
                        units,
                        format!("{} ({})", units.label(), units.symbol()),
                    );
                }
            });
        ui.end_row();
    });

    section_heading(ui, "Undo");
    Grid::new("grid4d.preferences.undo").num_columns(2).show(ui, |ui| {
        attr_label(ui, "Undo Depth", LABEL_WIDTH);
        ui.add(
            DragValue::new(&mut prefs.undo_depth)
                .speed(1.0)
                .range(Preferences::UNDO_DEPTH)
                .suffix(" steps"),
        )
        .on_hover_text("How many steps Undo can go back; lowering it drops the oldest at once");
        ui.end_row();
    });

    section_heading(ui, "Autosave");
    Grid::new("grid4d.preferences.autosave").num_columns(2).show(ui, |ui| {
        attr_label(ui, "Autosave", LABEL_WIDTH);
        ui.checkbox(&mut prefs.autosave, "")
            .on_hover_text("Keep a recovery copy of unsaved work, offered after a crash");
        ui.end_row();
        attr_label(ui, "Every", LABEL_WIDTH);
        ui.add_enabled(
            prefs.autosave,
            DragValue::new(&mut prefs.autosave_minutes)
                .speed(0.2)
                .range(Preferences::AUTOSAVE_MINUTES)
                .suffix(" min"),
        );
        ui.end_row();
    });

    section_heading(ui, "Views");
    let views = &mut prefs.views;
    Grid::new("grid4d.preferences.views").num_columns(2).show(ui, |ui| {
        attr_label(ui, "Shading", LABEL_WIDTH);
        ComboBox::from_id_salt("grid4d.preferences.shading")
            .selected_text(shading_label(views.shading))
            .width(180.0)
            .show_ui(ui, |ui| {
                for mode in ShadingMode::ALL {
                    ui.selectable_value(&mut views.shading, mode, shading_label(mode));
                }
            });
        ui.end_row();
        for (label, value) in [
            ("Grid", &mut views.grid),
            ("World Axis", &mut views.world_axis),
            ("HUD", &mut views.hud),
        ] {
            attr_label(ui, label, LABEL_WIDTH);
            ui.checkbox(value, "");
            ui.end_row();
        }
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(LABEL_WIDTH + 8.0);
        ui.button("Apply to Views")
            .on_hover_text("Views get these settings when Grid4D starts; this applies them now")
            .clicked()
    })
    .inner
}
