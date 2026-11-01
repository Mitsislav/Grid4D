//! "About & shortcuts" modal: the version and build, then the shortcuts or
//! the third-party licences (see `docs/reference/app-release.md`).

use egui::{Grid, Id, Modal, RichText, ScrollArea};

use crate::i18n::{key, tr, trf};
use crate::{EditorState, theme::palette as p};

pub fn show(ctx: &egui::Context, editor: &mut EditorState) {
    if !editor.show_about {
        return;
    }
    let response = Modal::new(Id::unique("grid4d.about")).show(ctx, |ui| {
        ui.set_width(460.0);
        ui.label(RichText::new(tr("Grid4D")).size(20.0).strong().color(p().text_strong));
        let build = editor.build;
        let version = if build.commit.is_empty() {
            trf("Version {0} · open-source 3D content creation suite", &[&build.version])
        } else {
            trf(
                "Version {0} ({1}, {2}) · open-source 3D content creation suite",
                &[&build.version, &build.commit, &build.date],
            )
        };
        ui.label(RichText::new(version).color(p().text_dim));
        ui.label(
            RichText::new(tr(
                "Interface in English and Greek (Preferences › Interface › Language)",
            ))
            .color(p().text_dim),
        );
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            for (licences, label) in [(false, key("Keyboard & mouse")), (true, key("Licences"))] {
                if ui
                    .selectable_label(editor.show_licences == licences, RichText::new(tr(label)).strong())
                    .clicked()
                {
                    editor.show_licences = licences;
                }
            }
        });
        ui.add_space(4.0);
        // The page scrolls, so the dialog (and its Close button) fits small windows.
        let room = (ctx.content_rect().height() - 260.0).max(120.0);
        if editor.show_licences {
            licences(ui, build.licences, room);
        } else {
            ScrollArea::vertical().max_height(room).show(ui, |ui| {
                Grid::new("grid4d.shortcuts")
                    .num_columns(2)
                    .spacing([24.0, 4.0])
                    .striped(true)
                    .show(ui, |ui| {
                        for &(keys, action) in crate::shortcuts::TABLE {
                            ui.label(RichText::new(keys).monospace().color(p().text_strong));
                            ui.label(tr(action));
                            ui.end_row();
                        }
                    });
            });
        }
        ui.add_space(10.0);
        ui.label(
            RichText::new(tr(
                "Grid4D is an independent project inspired by the Cinema 4D workflow. \
                 It is not affiliated with or endorsed by Maxon. Cinema 4D is a trademark of Maxon Computer GmbH.",
            ))
            .size(11.0)
            .color(p().text_faint),
        );
        ui.add_space(8.0);
        ui.vertical_centered(|ui| ui.button(tr("Close")).clicked()).inner
    });
    if response.inner || response.should_close() {
        editor.show_about = false;
        editor.show_licences = false;
    }
}

/// The Licences page: the third-party list (its first lines say Grid4D's own
/// licence), a line per row, unwrapped (only the rows in view are laid out).
fn licences(ui: &mut egui::Ui, text: &str, room: f32) {
    if text.is_empty() {
        ui.label(RichText::new(tr("This build has no licence list.")).color(p().text_dim));
        return;
    }
    let row = ui.text_style_height(&egui::TextStyle::Monospace);
    let lines = text.lines().count();
    ScrollArea::both()
        .id_salt("grid4d.about.licences")
        .max_height(room)
        .max_width(ui.available_width())
        .auto_shrink([false, true])
        .show_rows(ui, row, lines, |ui, range| {
            for line in text.lines().skip(range.start).take(range.len()) {
                ui.add(egui::Label::new(RichText::new(line).monospace().color(p().text_dim)).extend());
            }
        });
}
