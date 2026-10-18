//! "Save changes?" — shown when New, Open or Quit would replace a document
//! with unsaved changes. The answer is queued as a command.

use egui::{Id, Modal, RichText};
use oc4d_scene::Document;

use crate::{
    EditorCommand, EditorState,
    file::{FileAction, UnsavedChoice},
    theme::palette as p,
};

pub fn show(ctx: &egui::Context, doc: &Document, editor: &mut EditorState) {
    let Some(action) = editor.file.pending() else { return };
    let mut choice = None;
    let response = Modal::new(Id::unique("oc4d.unsaved_changes")).show(ctx, |ui| {
        ui.set_width(400.0);
        ui.label(
            RichText::new(format!("Save changes to “{}”?", editor.file.display_name(doc)))
                .size(15.0)
                .strong()
                .color(p::TEXT_STRONG),
        );
        ui.add_space(6.0);
        ui.label(RichText::new(consequence(action)).color(p::TEXT_DIM));
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            if ui.button("Save").clicked() {
                choice = Some(UnsavedChoice::Save);
            }
            if ui.button("Don't Save").clicked() {
                choice = Some(UnsavedChoice::Discard);
            }
            if ui.button("Cancel").clicked() {
                choice = Some(UnsavedChoice::Cancel);
            }
        });
    });
    // Escape or a click outside the dialog cancels.
    if choice.is_none() && response.should_close() {
        choice = Some(UnsavedChoice::Cancel);
    }
    if let Some(choice) = choice {
        editor.commands.push(EditorCommand::AnswerUnsaved(choice));
    }
}

const fn consequence(action: &FileAction) -> &'static str {
    match action {
        FileAction::New => "Your changes will be lost if you start a new document without saving.",
        FileAction::Open | FileAction::OpenPath(_) => {
            "Your changes will be lost if you open another document without saving."
        }
        FileAction::Quit => "Your changes will be lost if you quit without saving.",
    }
}
