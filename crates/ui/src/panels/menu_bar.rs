//! Main menu bar. Menus mirror Cinema 4D's; unimplemented entries are shown
//! disabled so the roadmap is visible in the product itself.

use egui::{Button, Layout, RichText, Ui};
use grid4d_scene::{Document, ObjectKindTag, SceneEdit};

use crate::{EditMode, EditorCommand, EditorState, Tool, theme::palette as p};

pub fn show(ui: &mut Ui, doc: &mut Document, editor: &mut EditorState) {
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| {
            command(ui, editor, "New", "Ctrl+N", EditorCommand::NewDocument);
            command(ui, editor, "Open…", "Ctrl+O", EditorCommand::Open);
            ui.menu_button("Open Recent", |ui| recent_menu(ui, editor));
            ui.separator();
            command(ui, editor, "Save", "Ctrl+S", EditorCommand::Save);
            command(ui, editor, "Save As…", "Ctrl+Shift+S", EditorCommand::SaveAs);
            ui.separator();
            planned(ui, "Import…", "");
            planned(ui, "Export…", "");
            ui.separator();
            // Asks to save unsaved changes before the window closes.
            command(ui, editor, "Quit", "", EditorCommand::Quit);
        });

        ui.menu_button("Edit", |ui| {
            // The menu is only built while open, so formatting the labels is cheap.
            let undo = editor.history.undo_label();
            let redo = editor.history.redo_label();
            history_item(
                ui,
                editor,
                undo.map_or_else(|| "Undo".to_owned(), |l| format!("Undo {l}")),
                "Ctrl+Z",
                undo.is_some(),
                EditorCommand::Undo,
            );
            history_item(
                ui,
                editor,
                redo.map_or_else(|| "Redo".to_owned(), |l| format!("Redo {l}")),
                "Ctrl+Y",
                redo.is_some(),
                EditorCommand::Redo,
            );
            ui.separator();
            command(ui, editor, "Delete", "Del", EditorCommand::DeleteSelected);
            command(ui, editor, "Select All", "Ctrl+A", EditorCommand::SelectAll);
            command(ui, editor, "Deselect All", "Ctrl+Shift+A", EditorCommand::DeselectAll);
            ui.separator();
            // With nothing selected the Attribute Manager shows the project settings.
            command(ui, editor, "Project Settings…", "", EditorCommand::DeselectAll);
        });

        ui.menu_button("Create", |ui| {
            ui.menu_button("Object", |ui| {
                create(ui, editor, ObjectKindTag::Null);
                ui.separator();
                for tag in ObjectKindTag::PRIMITIVES {
                    create(ui, editor, tag);
                }
                ui.separator();
                for name in ["Platonic", "Landscape"] {
                    planned(ui, name, "");
                }
            });
            ui.menu_button("Spline", |ui| {
                for name in ["Pen", "Circle", "Rectangle", "Helix", "Text"] {
                    planned(ui, name, "");
                }
            });
            ui.menu_button("Generator", |ui| {
                for name in ["Subdivision Surface", "Extrude", "Lathe", "Loft", "Sweep", "Cloner"] {
                    planned(ui, name, "");
                }
            });
            ui.separator();
            create(ui, editor, ObjectKindTag::Camera);
            create(ui, editor, ObjectKindTag::Light);
            ui.separator();
            planned(ui, "Material", "");
        });

        ui.menu_button("Modes", |ui| {
            for mode in EditMode::ALL {
                if ui
                    .add(Button::new(mode.label()).selected(editor.mode == mode))
                    .clicked()
                {
                    editor.mode = mode;
                    ui.close();
                }
            }
        });

        ui.menu_button("Select", |ui| {
            tool_item(ui, editor, Tool::LiveSelection);
            ui.separator();
            command(ui, editor, "Select All", "Ctrl+A", EditorCommand::SelectAll);
            command(ui, editor, "Deselect All", "Ctrl+Shift+A", EditorCommand::DeselectAll);
        });

        ui.menu_button("Tools", |ui| {
            for tool in [Tool::Move, Tool::Scale, Tool::Rotate] {
                tool_item(ui, editor, tool);
            }
            ui.separator();
            ui.checkbox(&mut editor.axes[0], "X-Axis");
            ui.checkbox(&mut editor.axes[1], "Y-Axis");
            ui.checkbox(&mut editor.axes[2], "Z-Axis");
            ui.separator();
            ui.checkbox(&mut editor.world_coordinates, "World Coordinates");
            ui.checkbox(&mut editor.snapping, "Snapping");
        });

        ui.menu_button("Mesh", |ui| {
            ui.menu_button("Conversion", |ui| {
                let convertible = crate::commands::can_make_editable(doc, &editor.selection);
                if ui
                    .add_enabled(convertible, Button::new("Make Editable").shortcut_text("C"))
                    .on_disabled_hover_text("Select a primitive to turn it into a polygon object")
                    .clicked()
                {
                    editor.commands.push(EditorCommand::MakeEditable);
                    ui.close();
                }
            });
            ui.separator();
            for name in [
                "Extrude",
                "Inner Extrude",
                "Bevel",
                "Knife",
                "Loop/Path Cut",
                "Subdivide",
            ] {
                planned(ui, name, "");
            }
        });

        ui.menu_button("Animate", |ui| {
            let time = &mut doc.time;
            if ui.add(Button::new("Play Forwards").shortcut_text("F8")).clicked() {
                time.play_forward();
                ui.close();
            }
            if ui.add(Button::new("Play Backwards").shortcut_text("F6")).clicked() {
                time.play_backward();
                ui.close();
            }
            if ui.add(Button::new("Stop").shortcut_text("F7")).clicked() {
                time.stop();
                ui.close();
            }
            ui.separator();
            if ui.button("Go to Start").clicked() {
                time.go_to_start();
                ui.close();
            }
            if ui.button("Go to End").clicked() {
                time.go_to_end();
                ui.close();
            }
            // Same behaviour as the F / G shortcuts and the transport buttons.
            if ui.add(Button::new("Previous Frame").shortcut_text("F")).clicked() {
                time.stop();
                time.step(-1);
                ui.close();
            }
            if ui.add(Button::new("Next Frame").shortcut_text("G")).clicked() {
                time.stop();
                time.step(1);
                ui.close();
            }
            ui.separator();
            // A document setting: applied as an undo step, after layout.
            let mut settings = time.settings();
            if ui.checkbox(&mut settings.looping, "Loop Playback").changed() {
                editor.commands.push(EditorCommand::Edit {
                    edit: SceneEdit::SetTime(settings),
                    label: "Loop Playback",
                });
            }
            planned(ui, "Record Active Objects", "F9");
        });

        ui.menu_button("Simulate", |ui| {
            for name in ["Rigid Body", "Soft Body", "Cloth", "Particles"] {
                planned(ui, name, "");
            }
        });

        ui.menu_button("Render", |ui| {
            planned(ui, "Render View", "Ctrl+R");
            planned(ui, "Render to Picture Viewer", "Shift+R");
            ui.separator();
            planned(ui, "Edit Render Settings…", "Ctrl+B");
        });

        ui.menu_button("Window", |ui| {
            command(ui, editor, "Frame Scene", "H", EditorCommand::FrameScene);
            command(ui, editor, "Frame Selected Objects", "O", EditorCommand::FrameSelected);
            command(ui, editor, "Reset View", "", EditorCommand::ResetView);
            ui.separator();
            planned(ui, "Customize Layout…", "");
        });

        ui.menu_button("Help", |ui| {
            if ui.button("Keyboard Shortcuts & About").clicked() {
                editor.show_about = true;
                ui.close();
            }
        });

        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new("Layout: Standard").color(p::TEXT_DIM).size(11.5));
        });
    });
}

fn command(ui: &mut Ui, editor: &mut EditorState, label: &str, shortcut: &str, cmd: EditorCommand) {
    if ui.add(Button::new(label).shortcut_text(shortcut)).clicked() {
        editor.commands.push(cmd);
        ui.close();
    }
}

/// File ▸ Open Recent: newest first, full path on hover, and Clear.
fn recent_menu(ui: &mut Ui, editor: &mut EditorState) {
    let mut open = None;
    if editor.file.recent.is_empty() {
        ui.add_enabled(false, Button::new("No recent documents"));
    }
    for path in editor.file.recent.paths() {
        let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
        if ui.button(name).on_hover_text(path.display().to_string()).clicked() {
            open = Some(path.clone());
        }
    }
    ui.separator();
    if ui
        .add_enabled(!editor.file.recent.is_empty(), Button::new("Clear Recent"))
        .clicked()
    {
        editor.file.recent.clear();
        ui.close();
    }
    if let Some(path) = open {
        editor.commands.push(EditorCommand::OpenFile(path));
        ui.close();
    }
}

fn history_item(
    ui: &mut Ui,
    editor: &mut EditorState,
    label: String,
    shortcut: &str,
    enabled: bool,
    cmd: EditorCommand,
) {
    if ui
        .add_enabled(enabled, Button::new(label).shortcut_text(shortcut))
        .clicked()
    {
        editor.commands.push(cmd);
        ui.close();
    }
}

fn create(ui: &mut Ui, editor: &mut EditorState, tag: ObjectKindTag) {
    if ui.button(tag.label()).clicked() {
        editor.commands.push(EditorCommand::CreateObject(tag));
        ui.close();
    }
}

fn tool_item(ui: &mut Ui, editor: &mut EditorState, tool: Tool) {
    let button = Button::new(tool.label())
        .shortcut_text(tool.shortcut())
        .selected(editor.tool == tool);
    if ui.add(button).clicked() {
        editor.tool = tool;
        ui.close();
    }
}

/// A disabled menu entry for a feature on the roadmap.
fn planned(ui: &mut Ui, label: &str, shortcut: &str) {
    ui.add_enabled(false, Button::new(label).shortcut_text(shortcut))
        .on_disabled_hover_text("Planned — not implemented yet");
}
