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
            command(ui, editor, "Merge…", "Ctrl+Shift+O", EditorCommand::Merge);
            ui.separator();
            command(ui, editor, "Save", "Ctrl+S", EditorCommand::Save);
            command(ui, editor, "Save As…", "Ctrl+Shift+S", EditorCommand::SaveAs);
            ui.separator();
            command(ui, editor, "Export…", "", EditorCommand::Export);
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
                if ui
                    .add(Button::new(Tool::Pen.label()).selected(editor.tool == Tool::Pen))
                    .on_hover_text("Click in the viewport to add points, drag to pull tangents")
                    .clicked()
                {
                    editor.tool = Tool::Pen;
                    ui.close();
                }
                ui.separator();
                for tag in ObjectKindTag::SPLINES {
                    create(ui, editor, tag);
                }
            });
            ui.menu_button("Generator", |ui| {
                for tag in ObjectKindTag::GENERATORS {
                    if ui
                        .button(tag.label())
                        .on_hover_text("Alt + click: around the selected object")
                        .clicked()
                    {
                        let alt = ui.input(|i| i.modifiers.alt);
                        editor.commands.push(if alt {
                            EditorCommand::WrapInGenerator(tag)
                        } else {
                            EditorCommand::CreateObject(tag)
                        });
                        ui.close();
                    }
                }
                planned(ui, "Cloner", "");
            });
            ui.menu_button("Deformer", |ui| {
                for tag in ObjectKindTag::DEFORMERS {
                    if ui
                        .button(tag.label())
                        .on_hover_text("Shift + click: under the selected object, fitted to it")
                        .clicked()
                    {
                        let shift = ui.input(|i| i.modifiers.shift);
                        editor.commands.push(if shift {
                            EditorCommand::CreateChild(tag)
                        } else {
                            EditorCommand::CreateObject(tag)
                        });
                        ui.close();
                    }
                }
            });
            ui.separator();
            create(ui, editor, ObjectKindTag::Camera);
            create(ui, editor, ObjectKindTag::Light);
            ui.separator();
            if ui
                .button("Material")
                .on_hover_text("A new material in the Material Manager")
                .clicked()
            {
                editor.commands.push(EditorCommand::NewMaterial);
                editor.bottom = crate::BottomPanel::Materials;
                ui.close();
            }
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
            for tool in [
                Tool::LiveSelection,
                Tool::RectangleSelection,
                Tool::LoopSelection,
                Tool::RingSelection,
            ] {
                tool_item(ui, editor, tool);
            }
            ui.separator();
            command(ui, editor, "Select All", "Ctrl+A", EditorCommand::SelectAll);
            command(ui, editor, "Deselect All", "Ctrl+Shift+A", EditorCommand::DeselectAll);
            // The rest works on components: a component mode on a polygon object.
            let components = crate::commands::component_target(doc, editor).is_some();
            let item = |ui: &mut Ui, editor: &mut EditorState, label: &str, cmd: EditorCommand| {
                if ui
                    .add_enabled(components, Button::new(label))
                    .on_disabled_hover_text("Works on the points, edges or polygons of a polygon object")
                    .clicked()
                {
                    editor.commands.push(cmd);
                    ui.close();
                }
            };
            item(ui, editor, "Invert", EditorCommand::InvertSelection);
            ui.separator();
            item(ui, editor, "Select Connected", EditorCommand::SelectConnected);
            item(ui, editor, "Grow Selection", EditorCommand::GrowSelection);
            item(ui, editor, "Shrink Selection", EditorCommand::ShrinkSelection);
            ui.checkbox(&mut editor.selection_settings.grow_edges_only, "Grow Using Edges Only")
                .on_hover_text("Polygons grow into neighbours sharing an edge; off: sharing a point is enough");
            ui.separator();
            item(ui, editor, "Hide Selected", EditorCommand::HideSelected);
            item(ui, editor, "Hide Unselected", EditorCommand::HideUnselected);
            item(ui, editor, "Unhide All", EditorCommand::UnhideAll);
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
            ui.menu_button("Snap", |ui| crate::widgets::snap_menu(ui, &mut editor.snap));
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
                let bakeable = crate::commands::can_bake_current_state(doc, &editor.selection);
                if ui
                    .add_enabled(bakeable, Button::new("Current State to Object"))
                    .on_hover_text("Add a copy of the selection as shown now: deformed, generated, at this frame")
                    .on_disabled_hover_text("Select the objects to copy as they are shown")
                    .clicked()
                {
                    editor.commands.push(EditorCommand::CurrentStateToObject);
                    ui.close();
                }
            });
            ui.menu_button("Spline", |ui| {
                let spline = crate::commands::active_spline(doc, editor).is_some();
                for op in crate::commands::SplineOp::ALL {
                    if ui
                        .add_enabled(spline, Button::new(op.label()))
                        .on_disabled_hover_text(
                            "Select an editable spline (Make Editable turns a spline primitive into one)",
                        )
                        .clicked()
                    {
                        editor.commands.push(EditorCommand::SplineCommand(op));
                        ui.close();
                    }
                }
            });
            ui.separator();
            let polygon_object = crate::commands::component_target(doc, editor).is_some();
            for tool in [
                Tool::Extrude,
                Tool::Inset,
                Tool::Bevel,
                Tool::Bridge,
                Tool::Weld,
                Tool::LoopCut,
                Tool::LineCut,
            ] {
                let how = match tool {
                    Tool::LoopCut | Tool::LineCut => {
                        "Then hover an edge and click (Loop/Path Cut) or drag a line (Line Cut) in the viewport"
                    }
                    Tool::Bridge => "Then drag from one polygon or border edge to another",
                    Tool::Weld => "Then click a selected point, or anywhere else for the centre",
                    _ => "Then drag left or right in the viewport, or Apply in the Attribute Manager",
                };
                let item = ui
                    .add_enabled(polygon_object, Button::new(tool.label()).shortcut_text(tool.shortcut()))
                    .on_hover_text(how)
                    .on_disabled_hover_text(tool.hint());
                if item.clicked() {
                    editor.tool = tool;
                    ui.close();
                }
            }
            ui.separator();
            for (label, shortcut, command) in [
                ("Connect Points/Edges", "M~M", EditorCommand::ConnectSelected),
                ("Melt", "U~Z", EditorCommand::MeltSelected),
                ("Dissolve", "M~N", EditorCommand::DissolveSelected),
            ] {
                if ui
                    .add_enabled(polygon_object, Button::new(label).shortcut_text(shortcut))
                    .on_disabled_hover_text("Works on the points, edges or polygons of a polygon object")
                    .clicked()
                {
                    editor.commands.push(command);
                    ui.close();
                }
            }
            let active_polygon = editor
                .selection
                .active()
                .and_then(|id| doc.scene.get(id))
                .is_some_and(|n| matches!(n.data.kind, grid4d_scene::ObjectKind::Polygon { .. }));
            if ui
                .add_enabled(active_polygon, Button::new("Optimize…").shortcut_text("U~O"))
                .on_disabled_hover_text("Select a polygon object")
                .clicked()
            {
                editor.show_optimize = true;
                ui.close();
            }
            ui.separator();
            planned(ui, "Subdivide", "");
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
            ui.separator();
            let frame = time.frame();
            if ui
                .add_enabled(
                    !editor.selection.is_empty(),
                    Button::new("Record Active Objects").shortcut_text("F9"),
                )
                .clicked()
            {
                editor.commands.push(EditorCommand::RecordKeys { at: frame });
                ui.close();
            }
            let autokey = Button::new("Autokeying")
                .shortcut_text("Ctrl+F9")
                .selected(editor.animation.autokey);
            if ui.add(autokey).clicked() {
                editor.animation.autokey = !editor.animation.autokey;
                ui.close();
            }
            ui.menu_button("Record", |ui| {
                let a = &mut editor.animation;
                ui.checkbox(&mut a.position, "Position");
                ui.checkbox(&mut a.scale, "Scale");
                ui.checkbox(&mut a.rotation, "Rotation");
            });
            for (label, shortcut, forward) in [
                ("Go to Previous Key", "Ctrl+F", false),
                ("Go to Next Key", "Ctrl+G", true),
            ] {
                let target = crate::keys::adjacent_key(&doc.scene, &editor.selection, frame, forward);
                if ui
                    .add_enabled(target.is_some(), Button::new(label).shortcut_text(shortcut))
                    .clicked()
                    && let Some(target) = target
                {
                    doc.time.stop();
                    doc.time.set_frame(target);
                    ui.close();
                }
            }
            if ui
                .add_enabled(!editor.keys.frames.is_empty(), Button::new("Delete Keys"))
                .on_disabled_hover_text("Select keys on the timeline first")
                .clicked()
            {
                editor.commands.push(EditorCommand::DeleteKeys);
                ui.close();
            }
        });

        ui.menu_button("Simulate", |ui| {
            for name in ["Rigid Body", "Soft Body", "Cloth", "Particles"] {
                planned(ui, name, "");
            }
        });

        ui.menu_button("Render", |ui| {
            command(ui, editor, "Render View", "Ctrl+R", EditorCommand::RenderView);
            command(
                ui,
                editor,
                "Render to Picture Viewer",
                "Shift+R",
                EditorCommand::RenderToPictureViewer,
            );
            ui.separator();
            let settings = Button::new("Edit Render Settings…")
                .shortcut_text("Ctrl+B")
                .selected(editor.show_render_settings);
            if ui.add(settings).clicked() {
                editor.show_render_settings = !editor.show_render_settings;
                ui.close();
            }
        });

        ui.menu_button("Window", |ui| {
            command(ui, editor, "Frame Scene", "H", EditorCommand::FrameScene);
            command(ui, editor, "Frame Selected Objects", "O", EditorCommand::FrameSelected);
            command(ui, editor, "Reset View", "", EditorCommand::ResetView);
            ui.separator();
            let materials = Button::new("Material Manager")
                .shortcut_text("Shift+F2")
                .selected(editor.bottom == crate::BottomPanel::Materials);
            if ui.add(materials).clicked() {
                editor.bottom = crate::BottomPanel::Materials;
                ui.close();
            }
            let viewer = Button::new("Picture Viewer").selected(editor.renders.viewer_open);
            if ui.add(viewer).clicked() {
                editor.renders.viewer_open = !editor.renders.viewer_open;
                ui.close();
            }
            for (tab, label, shortcut) in [
                (crate::ManagerTab::Attributes, "Attribute Manager", "Shift+F5"),
                (crate::ManagerTab::Coordinates, "Coordinate Manager", "Shift+F7"),
            ] {
                let item = Button::new(label)
                    .shortcut_text(shortcut)
                    .selected(editor.managers == tab);
                if ui.add(item).clicked() {
                    editor.managers = tab;
                    ui.close();
                }
            }
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
