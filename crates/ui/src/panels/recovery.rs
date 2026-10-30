//! "Recover Unsaved Work?" — shown at startup when a Grid4D that did not close
//! normally left a recovery copy. The answer is queued as a command.

use std::time::SystemTime;

use egui::{Id, Modal, RichText};

use crate::{EditorCommand, EditorState, autosave::RecoveryChoice, theme::palette as p};

pub fn show(ctx: &egui::Context, editor: &mut EditorState) {
    let Some(offer) = &editor.autosave.offer else { return };
    let name = offer
        .original
        .as_deref()
        .and_then(std::path::Path::file_name)
        .map_or_else(
            || "an unsaved document".to_owned(),
            |n| format!("“{}”", n.to_string_lossy()),
        );
    let message = format!(
        "Grid4D did not close normally. A copy of {name} was saved automatically {}.",
        ago(offer.written)
    );
    let mut choice = None;
    let response = Modal::new(Id::unique("grid4d.recovery")).show(ctx, |ui| {
        ui.set_width(420.0);
        ui.label(
            RichText::new("Recover Unsaved Work?")
                .size(15.0)
                .strong()
                .color(p().text_strong),
        );
        ui.add_space(6.0);
        ui.label(RichText::new(message).color(p().text_dim));
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            if ui.button("Recover").clicked() {
                choice = Some(RecoveryChoice::Recover);
            }
            if ui.button("Discard").clicked() {
                choice = Some(RecoveryChoice::Discard);
            }
        });
    });
    // Escape or a click outside keeps the copy for the next start.
    if response.should_close() && choice.is_none() {
        editor.autosave.offer = None;
    }
    if let Some(choice) = choice {
        editor.commands.push(EditorCommand::Recover(choice));
    }
}

/// "3 minutes ago", "an hour ago", … from the copy's time.
fn ago(written: SystemTime) -> String {
    let seconds = SystemTime::now().duration_since(written).map_or(0, |d| d.as_secs());
    match seconds {
        0..60 => "less than a minute ago".to_owned(),
        60..120 => "a minute ago".to_owned(),
        120..3600 => format!("{} minutes ago", seconds / 60),
        3600..7200 => "an hour ago".to_owned(),
        7200..172_800 => format!("{} hours ago", seconds / 3600),
        _ => format!("{} days ago", seconds / 86_400),
    }
}
