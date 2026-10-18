//! Bottom status line: contextual help on the left, scene/GPU info on the right.

use egui::{Align, Layout, RichText, Ui};
use oc4d_scene::Document;

use crate::{EditorState, theme::palette as p};

pub fn show(ui: &mut Ui, doc: &Document, editor: &EditorState) {
    ui.horizontal_centered(|ui| {
        let hint = editor.status_hint.unwrap_or_else(|| editor.tool.hint());
        ui.label(RichText::new(hint).color(p::TEXT_DIM).size(11.5));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                RichText::new(editor.renderer_info.as_str())
                    .color(p::TEXT_FAINT)
                    .size(11.0),
            );
            ui.separator();
            let objects = doc.scene.len();
            let selected = editor.selection.len();
            ui.label(
                RichText::new(format!("{objects} objects · {selected} selected"))
                    .color(p::TEXT_DIM)
                    .size(11.0),
            );
        });
    });
}
