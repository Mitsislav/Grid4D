//! A modal message for errors the user must know about (a file that could not
//! be opened or saved). Dismissing it clears `EditorState::file.alert`.

use egui::{Id, Modal, RichText};

use crate::{EditorState, theme::palette as p};

pub fn show(ctx: &egui::Context, editor: &mut EditorState) {
    let Some(alert) = &editor.file.alert else { return };
    let mut dismissed = false;
    let response = Modal::new(Id::unique("grid4d.alert")).show(ctx, |ui| {
        ui.set_width(440.0);
        ui.label(RichText::new(alert.title).size(15.0).strong().color(p().text_strong));
        ui.add_space(6.0);
        ui.label(alert.message.as_str());
        ui.add_space(12.0);
        ui.vertical_centered(|ui| dismissed = ui.button("OK").clicked());
    });
    if dismissed || response.should_close() {
        editor.file.alert = None;
    }
}
