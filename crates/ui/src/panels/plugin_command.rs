//! Plugins ▸ a command… dialog: the command's options, laid out from its
//! plugin's description, then OK runs it (see `docs/reference/extend-plugins.md`).

use egui::{Grid, Id, Modal, RichText};

use crate::i18n::tr;
use crate::{EditorCommand, EditorState, theme::palette as p, widgets};

pub fn show(ctx: &egui::Context, editor: &mut EditorState) {
    let Some(type_id) = editor.plugin_commands.open.clone() else {
        return;
    };
    let Some(plugin) = grid4d_plugin::registry().command(&type_id) else {
        editor.plugin_commands.open = None;
        return;
    };
    let options = editor.plugin_commands.options_mut(plugin);
    let mut run = false;
    let response = Modal::new(Id::unique("grid4d.plugin_command")).show(ctx, |ui| {
        ui.set_width(360.0);
        ui.label(
            RichText::new(tr(plugin.name()))
                .size(16.0)
                .strong()
                .color(p().text_strong),
        );
        ui.label(RichText::new(tr(plugin.description())).color(p().text_dim));
        ui.add_space(6.0);
        Grid::new("grid4d.plugin_command.options")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                for spec in plugin.params() {
                    let current = grid4d_plugin::Values::new(plugin.params(), options)
                        .get(spec.name)
                        .unwrap_or(spec.default);
                    let mut value = current;
                    ui.label(tr(spec.name));
                    let response = widgets::plugin_value(ui, spec, &mut value, 120.0);
                    if !spec.hint.is_empty() {
                        response.on_hover_text(tr(spec.hint));
                    }
                    ui.end_row();
                    if value != current {
                        options.set(spec.name, value);
                    }
                }
            });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            run = ui.button(tr("OK")).clicked();
            run || ui.button(tr("Cancel")).clicked()
        })
        .inner
    });
    if run {
        editor.commands.push(EditorCommand::RunPlugin {
            type_id: type_id.clone(),
            options: options.clone(),
        });
    }
    if response.inner || response.should_close() {
        editor.plugin_commands.open = None;
    }
}
