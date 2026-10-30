//! The Script Manager (Script ▸ Script Manager…): edits a script of the
//! library — New, Open, Save under a name — runs it (Run, or Ctrl+Enter in
//! the editor) and shows the console: what scripts printed and their errors.
//! The library's files are read and written by commands (`crate::scripts`).

use egui::{Id, Key, Modifiers, RichText, ScrollArea, TextEdit, TextStyle, Window};

use crate::i18n::tr;
use crate::{EditorCommand, EditorState, theme::palette as p};

/// The editor's id: Ctrl+Enter runs the script while it has the keyboard.
pub const EDITOR_ID: &str = "grid4d.script_manager.editor";

pub fn show(ctx: &egui::Context, editor: &mut EditorState) {
    let mut open = editor.scripts.show;
    if !open {
        return;
    }
    let scripts = &mut editor.scripts;
    let name = if scripts.name.trim().is_empty() {
        tr("Untitled").to_owned()
    } else {
        scripts.name.trim().to_owned()
    };
    let dirty = if scripts.is_dirty() { " *" } else { "" };
    let title = format!("{} — {name}{dirty}", tr("Script Manager"));
    let editor_id = Id::unique(EDITOR_ID);
    let mut commands = Vec::new();
    Window::new(title)
        .id(Id::unique("grid4d.script_manager"))
        .open(&mut open)
        .collapsible(false)
        .default_width(620.0)
        .default_pos(ctx.content_rect().center() - egui::vec2(310.0, 260.0))
        .show(ctx, |ui| {
            // The toolbar.
            ui.horizontal(|ui| {
                if ui.button(tr("New")).on_hover_text(tr("Starts a new script")).clicked() {
                    commands.push(EditorCommand::NewScript);
                }
                ui.menu_button(tr("Open"), |ui| {
                    if scripts.names().is_empty() {
                        ui.add_enabled(false, egui::Button::new(tr("No scripts in the library")));
                    }
                    for script in scripts.names() {
                        if ui.button(script).clicked() {
                            commands.push(EditorCommand::OpenScript(script.clone()));
                            ui.close();
                        }
                    }
                });
                if ui
                    .button(tr("Save"))
                    .on_hover_text(tr("Saves the script into the library under its name"))
                    .clicked()
                {
                    commands.push(EditorCommand::SaveScript);
                }
                ui.add(
                    TextEdit::singleline(&mut scripts.name)
                        .hint_text(tr("Name"))
                        .desired_width(180.0),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button(RichText::new(tr("Run")).strong())
                        .on_hover_text(tr("Runs the script on the document (Ctrl+Enter): one undo step"))
                        .clicked()
                    {
                        commands.push(EditorCommand::RunScript(scripts.text.clone()));
                    }
                });
            });
            ui.add_space(4.0);

            // Ctrl+Enter in the editor runs the script (before the editor
            // would take the Enter).
            let focused = ui.memory(|m| m.has_focus(editor_id));
            if focused && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter)) {
                commands.push(EditorCommand::RunScript(scripts.text.clone()));
            }

            // The editor, its line numbers beside it.
            ScrollArea::vertical()
                .id_salt("grid4d.script_manager.text")
                .max_height(320.0)
                .min_scrolled_height(320.0)
                .show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        let lines = scripts.text.split('\n').count().max(1);
                        let numbers = (1..=lines).map(|n| n.to_string()).collect::<Vec<_>>().join("\n");
                        ui.add_sized(
                            [28.0, 0.0],
                            egui::Label::new(
                                RichText::new(numbers)
                                    .text_style(TextStyle::Monospace)
                                    .color(p().text_dim),
                            )
                            .halign(egui::Align::RIGHT)
                            .selectable(false),
                        );
                        ui.add(
                            TextEdit::multiline(&mut scripts.text)
                                .id(editor_id)
                                .code_editor()
                                .desired_rows(18)
                                .desired_width(f32::INFINITY)
                                .lock_focus(true),
                        );
                    });
                });
            ui.add_space(4.0);

            // The console.
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr("Console")).strong());
                if ui.small_button(tr("Clear")).clicked() {
                    scripts.clear_console();
                }
            });
            egui::Frame::new().fill(p().bg_well).inner_margin(4.0).show(ui, |ui| {
                ScrollArea::vertical()
                    .id_salt("grid4d.script_manager.console")
                    .max_height(120.0)
                    .min_scrolled_height(120.0)
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for line in scripts.console() {
                            let colour = if line.error { p().warning } else { p().text };
                            ui.label(RichText::new(&line.text).text_style(TextStyle::Monospace).color(colour));
                        }
                    });
            });
        });
    editor.scripts.show = open;
    for command in commands {
        editor.commands.push(command);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One frame of the window with `events`.
    fn frame(ctx: &egui::Context, editor: &mut EditorState, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        ctx.run_ui(input, |ui| show(ui.ctx(), editor))
            .drop_without_applying_deltas();
    }

    #[test]
    fn ctrl_enter_in_the_editor_runs_the_script() {
        let ctx = egui::Context::default();
        let mut editor = EditorState::default();
        frame(&ctx, &mut editor, Vec::new());
        assert!(editor.commands.is_empty(), "closed: nothing");
        editor.scripts.show = true;
        editor.scripts.text = "print(1);".to_owned();
        frame(&ctx, &mut editor, Vec::new());
        assert!(editor.commands.is_empty(), "showing runs nothing");
        ctx.memory_mut(|m| m.request_focus(Id::unique(EDITOR_ID)));
        frame(&ctx, &mut editor, Vec::new());
        let enter = egui::Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        };
        frame(&ctx, &mut editor, vec![enter]);
        assert!(
            matches!(editor.commands.pending(), [EditorCommand::RunScript(text)] if text == "print(1);"),
            "{:?}",
            editor.commands
        );
        assert_eq!(editor.scripts.text, "print(1);", "no new line typed");
    }
}
