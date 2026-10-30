//! Bottom status line: contextual help on the left, scene/GPU info on the right.

use egui::{Align, Layout, RichText, Ui};
use grid4d_scene::Document;

use crate::{EditorState, theme::palette as p};

pub fn show(ui: &mut Ui, doc: &Document, editor: &EditorState) {
    ui.horizontal_centered(|ui| {
        let hint = editor
            .chord
            .map(crate::shortcuts::Chord::hint)
            .or(editor.status_hint)
            .unwrap_or_else(|| editor.tool.hint());
        match editor.simulation.baking() {
            Some((done, target)) => ui.label(
                RichText::new(format!("Simulating: frame {done} of {target} — Esc stops"))
                    .color(p().text_strong)
                    .size(11.5),
            ),
            None if editor.simulation.is_full() => ui.label(
                RichText::new(format!(
                    "The simulation cache is full ({} MB): raise Cache Memory in the Project settings",
                    doc.simulation.memory_limit_mb
                ))
                .color(p().warning)
                .size(11.5),
            ),
            None => ui.label(RichText::new(hint).color(p().text_dim).size(11.5)),
        };
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                RichText::new(editor.renderer_info.as_str())
                    .color(p().text_faint)
                    .size(11.0),
            );
            ui.separator();
            let objects = doc.scene.len();
            let selected = editor.selection.len();
            ui.label(
                RichText::new(format!("{objects} objects · {selected} selected"))
                    .color(p().text_dim)
                    .size(11.0),
            );
        });
    });
}
