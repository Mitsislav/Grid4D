//! "Recover Unsaved Work?" — shown at startup when a Grid4D that did not close
//! normally left a recovery copy. The answer is queued as a command. A crash
//! report written since the last start is pointed to there, or in its own
//! window, "Grid4D Stopped Unexpectedly", when there is nothing to recover.

use std::time::SystemTime;

use egui::{Id, Modal, RichText};

use crate::i18n::{tr, trf};
use crate::{EditorCommand, EditorState, autosave::RecoveryChoice, theme::palette as p};

pub fn show(ctx: &egui::Context, editor: &mut EditorState) {
    let Some(offer) = &editor.autosave.offer else {
        crash_report(ctx, editor);
        return;
    };
    let name = offer
        .original
        .as_deref()
        .and_then(std::path::Path::file_name)
        .map_or_else(
            || tr("an unsaved document").to_owned(),
            |n| format!("“{}”", n.to_string_lossy()),
        );
    let message = trf(
        "Grid4D did not close normally. A copy of {0} was saved automatically {1}.",
        &[&name, &ago(offer.written)],
    );
    let mut choice = None;
    let response = Modal::new(Id::unique("grid4d.recovery")).show(ctx, |ui| {
        ui.set_width(420.0);
        ui.label(
            RichText::new(tr("Recover Unsaved Work?"))
                .size(15.0)
                .strong()
                .color(p().text_strong),
        );
        ui.add_space(6.0);
        ui.label(RichText::new(message).color(p().text_dim));
        if let Some(report) = &editor.crash.new_report {
            ui.add_space(6.0);
            report_line(ui, report, &mut editor.commands);
        }
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            if ui.button(tr("Recover")).clicked() {
                choice = Some(RecoveryChoice::Recover);
            }
            if ui.button(tr("Discard")).clicked() {
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
    if editor.autosave.offer.is_none() || choice.is_some() {
        editor.crash.new_report = None;
    }
}

/// "Grid4D Stopped Unexpectedly": a crash report and nothing to recover.
fn crash_report(ctx: &egui::Context, editor: &mut EditorState) {
    let Some(report) = &editor.crash.new_report else { return };
    let mut ok = false;
    let response = Modal::new(Id::unique("grid4d.crash_report")).show(ctx, |ui| {
        ui.set_width(420.0);
        ui.label(
            RichText::new(tr("Grid4D Stopped Unexpectedly"))
                .size(15.0)
                .strong()
                .color(p().text_strong),
        );
        ui.add_space(6.0);
        report_line(ui, report, &mut editor.commands);
        ui.add_space(14.0);
        ui.vertical_centered(|ui| ok = ui.button(tr("OK")).clicked());
    });
    if ok || response.should_close() {
        editor.crash.new_report = None;
    }
}

/// Where the report is, that it stayed on this computer, and Show Report.
fn report_line(ui: &mut egui::Ui, report: &std::path::Path, commands: &mut crate::CommandQueue) {
    let name = report
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    ui.label(RichText::new(trf("A crash report was written: {0}", &[&name])).color(p().text_dim));
    ui.label(
        RichText::new(tr(
            "It stays on this computer; attach it to a bug report if you report the problem.",
        ))
        .color(p().text_dim),
    );
    ui.add_space(4.0);
    if ui.button(tr("Show Report")).clicked() {
        commands.push(EditorCommand::ShowCrashReport(report.to_path_buf()));
    }
}

/// "3 minutes ago", "an hour ago", … from the copy's time.
fn ago(written: SystemTime) -> String {
    let seconds = SystemTime::now().duration_since(written).map_or(0, |d| d.as_secs());
    match seconds {
        0..60 => tr("less than a minute ago").to_owned(),
        60..120 => tr("a minute ago").to_owned(),
        120..3600 => trf("{0} minutes ago", &[&(seconds / 60)]),
        3600..7200 => tr("an hour ago").to_owned(),
        7200..172_800 => trf("{0} hours ago", &[&(seconds / 3600)]),
        _ => trf("{0} days ago", &[&(seconds / 86_400)]),
    }
}
