//! Main menu bar. Menus mirror Cinema 4D's; unimplemented entries are shown
//! disabled so the roadmap is visible in the product itself.

use egui::{Button, Layout, RichText, Ui, ViewportCommand};
use oc4d_scene::{Document, ObjectKindTag};

use crate::{EditMode, EditorCommand, EditorState, Tool, theme::palette as p};

pub fn show(ui: &mut Ui, doc: &mut Document, editor: &mut EditorState) {
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| {
            if ui.button("New").clicked() {
                editor.commands.push(EditorCommand::NewDocument);
                ui.close();
            }
            planned(ui, "Open…", "Ctrl+O");
            planned(ui, "Save", "Ctrl+S");
            planned(ui, "Save As…", "Ctrl+Shift+S");
            ui.separator();
            planned(ui, "Import…", "");
            planned(ui, "Export…", "");
            ui.separator();
            if ui.button("Quit").clicked() {
                ui.ctx().send_viewport_cmd(ViewportCommand::Close);
            }
        });

        ui.menu_button("Edit", |ui| {
            planned(ui, "Undo", "Ctrl+Z");
            planned(ui, "Redo", "Ctrl+Y");
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
                for name in ["Cone", "Torus", "Disc", "Capsule", "Platonic"] {
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
            ui.checkbox(&mut time.looping, "Loop Playback");
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
