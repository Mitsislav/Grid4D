//! Mesh ▸ Optimize… dialog: the options, then OK runs the command.

use egui::{Grid, Id, Modal, RichText};

use crate::i18n::tr;
use crate::{EditorCommand, EditorState, theme::palette as p};

pub fn show(ctx: &egui::Context, editor: &mut EditorState) {
    if !editor.show_optimize {
        return;
    }
    let mut run = false;
    let response = Modal::new(Id::unique("grid4d.optimize")).show(ctx, |ui| {
        ui.set_width(320.0);
        ui.label(RichText::new(tr("Optimize")).size(16.0).strong().color(p().text_strong));
        ui.label(
            RichText::new(tr("Merges points that lie together and removes unused ones (the selection's points, or the whole object)."))
                .color(p().text_dim),
        );
        ui.add_space(6.0);
        let options = &mut editor.modeling_settings.optimize;
        Grid::new("grid4d.optimize.options").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label(tr("Points"));
            ui.checkbox(&mut options.merge_points, tr("Merge points closer than the tolerance"));
            ui.end_row();
            ui.label(tr("Tolerance"));
            ui.add_enabled(
                options.merge_points,
                crate::widgets::length(ui, &mut options.tolerance)
                    .speed(0.01)
                    .range(0.0..=1000.0)
                    .max_decimals(3),
            );
            ui.end_row();
            ui.label(tr("Unused Points"));
            ui.checkbox(&mut options.unused_points, tr("Remove points no polygon uses"));
            ui.end_row();
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            run = ui.button(tr("OK")).clicked();
            run || ui.button(tr("Cancel")).clicked()
        })
        .inner
    });
    if run {
        editor.commands.push(EditorCommand::Optimize);
    }
    if response.inner || response.should_close() {
        editor.show_optimize = false;
    }
}
