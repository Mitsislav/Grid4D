//! Main menu bar. Menus mirror Cinema 4D's; unimplemented entries are shown
//! disabled so the roadmap is visible in the product itself.

use egui::{Button, Layout, RichText, Ui};
use grid4d_scene::{Document, ObjectKindTag, SceneEdit};

use crate::i18n::{key, tr, trf};
use crate::{
    EditMode, EditorCommand, EditorState, Tool,
    theme::palette as p,
    workspace::{Layout as PaneLayout, Pane},
};

pub fn show(ui: &mut Ui, doc: &mut Document, editor: &mut EditorState) {
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button(tr("File"), |ui| {
            command(ui, editor, "New", "Ctrl+N", EditorCommand::NewDocument);
            command(ui, editor, "Open…", "Ctrl+O", EditorCommand::Open);
            ui.menu_button(tr("Open Recent"), |ui| recent_menu(ui, editor));
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

        ui.menu_button(tr("Edit"), |ui| {
            // The menu is only built while open, so formatting the labels is cheap.
            let undo = editor.history.undo_label();
            let redo = editor.history.redo_label();
            history_item(
                ui,
                editor,
                undo.map_or_else(|| tr("Undo").to_owned(), |l| trf("Undo {0}", &[&tr(l)])),
                "Ctrl+Z",
                undo.is_some(),
                EditorCommand::Undo,
            );
            history_item(
                ui,
                editor,
                redo.map_or_else(|| tr("Redo").to_owned(), |l| trf("Redo {0}", &[&tr(l)])),
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
            if ui.button(tr("Preferences…")).clicked() {
                editor.show_preferences = true;
                ui.close();
            }
        });

        ui.menu_button(tr("Create"), |ui| {
            ui.menu_button(tr("Object"), |ui| {
                create(ui, editor, ObjectKindTag::Null);
                ui.separator();
                for tag in ObjectKindTag::PRIMITIVES {
                    create(ui, editor, tag);
                }
                ui.separator();
                for name in [key("Platonic"), key("Landscape")] {
                    planned(ui, name, "");
                }
            });
            ui.menu_button(tr("Spline"), |ui| {
                if ui
                    .add(Button::new(tr(Tool::Pen.label())).selected(editor.tool == Tool::Pen))
                    .on_hover_text(tr("Click in the viewport to add points, drag to pull tangents"))
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
            ui.menu_button(tr("Generator"), |ui| {
                for tag in ObjectKindTag::GENERATORS {
                    if ui
                        .button(tr(tag.label()))
                        .on_hover_text(tr("Alt + click: around the selected object"))
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
            });
            ui.menu_button(tr("Effector"), |ui| {
                for tag in ObjectKindTag::EFFECTORS {
                    if ui
                        .button(tr(tag.label()))
                        .on_hover_text(tr("With a Cloner selected, it is added to the Cloner's Effectors"))
                        .clicked()
                    {
                        editor.commands.push(EditorCommand::CreateObject(tag));
                        ui.close();
                    }
                }
            });
            ui.menu_button(tr("Plugins"), |ui| {
                let registry = grid4d_plugin::registry();
                let mut any = false;
                for plugin in registry.generators() {
                    any = true;
                    if ui
                        .button(tr(plugin.name()))
                        .on_hover_text(tr(plugin.description()))
                        .clicked()
                    {
                        editor.commands.push(EditorCommand::CreatePlugin {
                            type_id: plugin.type_id().to_owned(),
                            child: false,
                        });
                        ui.close();
                    }
                }
                // Deformers after a line; Shift puts one under the selection.
                for (i, plugin) in registry.deformers().enumerate() {
                    if any && i == 0 {
                        ui.separator();
                    }
                    any = true;
                    let tip = format!(
                        "{}\n{}",
                        tr(plugin.description()),
                        tr("Shift + click: under the selected object, fitted to it")
                    );
                    if ui.button(tr(plugin.name())).on_hover_text(tip).clicked() {
                        let child = ui.input(|i| i.modifiers.shift);
                        editor.commands.push(EditorCommand::CreatePlugin {
                            type_id: plugin.type_id().to_owned(),
                            child,
                        });
                        ui.close();
                    }
                }
                if !any {
                    ui.add_enabled(false, Button::new(tr("No plugins installed")));
                }
            });
            ui.menu_button(tr("Deformer"), |ui| {
                for tag in ObjectKindTag::DEFORMERS {
                    if ui
                        .button(tr(tag.label()))
                        .on_hover_text(tr("Shift + click: under the selected object, fitted to it"))
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
                .button(tr("Material"))
                .on_hover_text(tr("A new material in the Material Manager"))
                .clicked()
            {
                editor.commands.push(EditorCommand::NewMaterial);
                editor.panes.show = Some(Pane::Materials);
                ui.close();
            }
        });

        ui.menu_button(tr("Modes"), |ui| {
            for mode in EditMode::ALL {
                if ui
                    .add(Button::new(tr(mode.label())).selected(editor.mode == mode))
                    .clicked()
                {
                    editor.mode = mode;
                    ui.close();
                }
            }
        });

        ui.menu_button(tr("Select"), |ui| {
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
                    .add_enabled(components, Button::new(tr(label)))
                    .on_disabled_hover_text(tr("Works on the points, edges or polygons of a polygon object"))
                    .clicked()
                {
                    editor.commands.push(cmd);
                    ui.close();
                }
            };
            item(ui, editor, key("Invert"), EditorCommand::InvertSelection);
            ui.separator();
            item(ui, editor, key("Select Connected"), EditorCommand::SelectConnected);
            item(ui, editor, key("Grow Selection"), EditorCommand::GrowSelection);
            item(ui, editor, key("Shrink Selection"), EditorCommand::ShrinkSelection);
            ui.checkbox(
                &mut editor.selection_settings.grow_edges_only,
                tr("Grow Using Edges Only"),
            )
            .on_hover_text(tr(
                "Polygons grow into neighbours sharing an edge; off: sharing a point is enough",
            ));
            ui.separator();
            item(ui, editor, key("Hide Selected"), EditorCommand::HideSelected);
            item(ui, editor, key("Hide Unselected"), EditorCommand::HideUnselected);
            item(ui, editor, key("Unhide All"), EditorCommand::UnhideAll);
        });

        ui.menu_button(tr("Tools"), |ui| {
            for tool in [Tool::Move, Tool::Scale, Tool::Rotate] {
                tool_item(ui, editor, tool);
            }
            ui.separator();
            ui.checkbox(&mut editor.axes[0], tr("X-Axis"));
            ui.checkbox(&mut editor.axes[1], tr("Y-Axis"));
            ui.checkbox(&mut editor.axes[2], tr("Z-Axis"));
            ui.separator();
            ui.checkbox(&mut editor.world_coordinates, tr("World Coordinates"));
            ui.menu_button(tr("Snap"), |ui| crate::widgets::snap_menu(ui, &mut editor.snap));
        });

        ui.menu_button(tr("Mesh"), |ui| {
            ui.menu_button(tr("Conversion"), |ui| {
                let convertible = crate::commands::can_make_editable(doc, &editor.selection);
                if ui
                    .add_enabled(convertible, Button::new(tr("Make Editable")).shortcut_text("C"))
                    .on_disabled_hover_text(tr("Select a primitive to turn it into a polygon object"))
                    .clicked()
                {
                    editor.commands.push(EditorCommand::MakeEditable);
                    ui.close();
                }
                let bakeable = crate::commands::can_bake_current_state(doc, &editor.selection);
                if ui
                    .add_enabled(bakeable, Button::new(tr("Current State to Object")))
                    .on_hover_text(tr(
                        "Add a copy of the selection as shown now: deformed, generated, at this frame",
                    ))
                    .on_disabled_hover_text(tr("Select the objects to copy as they are shown"))
                    .clicked()
                {
                    editor.commands.push(EditorCommand::CurrentStateToObject);
                    ui.close();
                }
            });
            ui.menu_button(tr("Spline"), |ui| {
                let spline = crate::commands::active_spline(doc, editor).is_some();
                for op in crate::commands::SplineOp::ALL {
                    if ui
                        .add_enabled(spline, Button::new(tr(op.label())))
                        .on_disabled_hover_text(tr(
                            "Select an editable spline (Make Editable turns a spline primitive into one)",
                        ))
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
                        key("Then hover an edge and click (Loop/Path Cut) or drag a line (Line Cut) in the viewport")
                    }
                    Tool::Bridge => key("Then drag from one polygon or border edge to another"),
                    Tool::Weld => key("Then click a selected point, or anywhere else for the centre"),
                    _ => key("Then drag left or right in the viewport, or Apply in the Attribute Manager"),
                };
                let item = ui
                    .add_enabled(
                        polygon_object,
                        Button::new(tr(tool.label())).shortcut_text(tool.shortcut()),
                    )
                    .on_hover_text(tr(how))
                    .on_disabled_hover_text(tr(tool.hint()));
                if item.clicked() {
                    editor.tool = tool;
                    ui.close();
                }
            }
            ui.separator();
            for (label, shortcut, command) in [
                (key("Connect Points/Edges"), "M~M", EditorCommand::ConnectSelected),
                (key("Melt"), "U~Z", EditorCommand::MeltSelected),
                (key("Dissolve"), "M~N", EditorCommand::DissolveSelected),
            ] {
                if ui
                    .add_enabled(polygon_object, Button::new(tr(label)).shortcut_text(shortcut))
                    .on_disabled_hover_text(tr("Works on the points, edges or polygons of a polygon object"))
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
                .add_enabled(active_polygon, Button::new(tr("Optimize…")).shortcut_text("U~O"))
                .on_disabled_hover_text(tr("Select a polygon object"))
                .clicked()
            {
                editor.show_optimize = true;
                ui.close();
            }
            ui.menu_button(tr("UV"), |ui| uv_menu(ui, editor, active_polygon));
            ui.separator();
            planned(ui, "Subdivide", "");
        });

        ui.menu_button(tr("Animate"), |ui| {
            let time = &mut doc.time;
            if ui.add(Button::new(tr("Play Forwards")).shortcut_text("F8")).clicked() {
                time.play_forward();
                ui.close();
            }
            if ui.add(Button::new(tr("Play Backwards")).shortcut_text("F6")).clicked() {
                time.play_backward();
                ui.close();
            }
            if ui.add(Button::new(tr("Stop")).shortcut_text("F7")).clicked() {
                time.stop();
                ui.close();
            }
            ui.separator();
            if ui.button(tr("Go to Start")).clicked() {
                time.go_to_start();
                ui.close();
            }
            if ui.button(tr("Go to End")).clicked() {
                time.go_to_end();
                ui.close();
            }
            // Same behaviour as the F / G shortcuts and the transport buttons.
            if ui.add(Button::new(tr("Previous Frame")).shortcut_text("F")).clicked() {
                time.stop();
                time.step(-1);
                ui.close();
            }
            if ui.add(Button::new(tr("Next Frame")).shortcut_text("G")).clicked() {
                time.stop();
                time.step(1);
                ui.close();
            }
            ui.separator();
            // A document setting: applied as an undo step, after layout.
            let mut settings = time.settings();
            if ui.checkbox(&mut settings.looping, tr("Loop Playback")).changed() {
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
                    Button::new(tr("Record Active Objects")).shortcut_text("F9"),
                )
                .clicked()
            {
                editor.commands.push(EditorCommand::RecordKeys { at: frame });
                ui.close();
            }
            let autokey = Button::new(tr("Autokeying"))
                .shortcut_text("Ctrl+F9")
                .selected(editor.animation.autokey);
            if ui.add(autokey).clicked() {
                editor.animation.autokey = !editor.animation.autokey;
                ui.close();
            }
            ui.menu_button(tr("Record"), |ui| {
                let a = &mut editor.animation;
                ui.checkbox(&mut a.position, tr("Position"));
                ui.checkbox(&mut a.scale, tr("Scale"));
                ui.checkbox(&mut a.rotation, tr("Rotation"));
            });
            for (label, shortcut, forward) in [
                (key("Go to Previous Key"), "Ctrl+F", false),
                (key("Go to Next Key"), "Ctrl+G", true),
            ] {
                let target = crate::keys::adjacent_key(&doc.scene, &editor.selection, frame, forward);
                if ui
                    .add_enabled(target.is_some(), Button::new(tr(label)).shortcut_text(shortcut))
                    .clicked()
                    && let Some(target) = target
                {
                    doc.time.stop();
                    doc.time.set_frame(target);
                    ui.close();
                }
            }
            if ui
                .add_enabled(!editor.keys.frames.is_empty(), Button::new(tr("Delete Keys")))
                .on_disabled_hover_text(tr("Select keys on the timeline first"))
                .clicked()
            {
                editor.commands.push(EditorCommand::DeleteKeys);
                ui.close();
            }
        });

        ui.menu_button(tr("Simulate"), |ui| {
            if ui
                .button(tr("Bake Simulation"))
                .on_hover_text(tr("Simulate the whole range now (Esc stops)"))
                .clicked()
            {
                editor.commands.push(EditorCommand::BakeSimulation);
                ui.close();
            }
            if ui
                .button(tr("Clear Simulation Cache"))
                .on_hover_text(tr(
                    "Forget the simulated frames; they are simulated again as you visit them",
                ))
                .clicked()
            {
                editor.commands.push(EditorCommand::ClearSimulation);
                ui.close();
            }
            ui.separator();
            for name in [key("Soft Body"), key("Cloth"), key("Particles")] {
                planned(ui, name, "");
            }
        });

        ui.menu_button(tr("Render"), |ui| {
            command(ui, editor, "Render View", "Ctrl+R", EditorCommand::RenderView);
            command(
                ui,
                editor,
                "Render to Picture Viewer",
                "Shift+R",
                EditorCommand::RenderToPictureViewer,
            );
            ui.separator();
            let settings = Button::new(tr("Edit Render Settings…"))
                .shortcut_text("Ctrl+B")
                .selected(editor.show_render_settings);
            if ui.add(settings).clicked() {
                editor.show_render_settings = !editor.show_render_settings;
                ui.close();
            }
        });

        ui.menu_button(tr("Plugins"), |ui| {
            let mut any = false;
            for plugin in grid4d_plugin::registry().commands() {
                any = true;
                // An ellipsis: its options come first.
                let label = if plugin.params().is_empty() {
                    tr(plugin.name()).to_owned()
                } else {
                    format!("{}…", tr(plugin.name()))
                };
                if ui.button(label).on_hover_text(tr(plugin.description())).clicked() {
                    if plugin.params().is_empty() {
                        editor.commands.push(EditorCommand::RunPlugin {
                            type_id: plugin.type_id().to_owned(),
                            options: grid4d_plugin::Registry::new_object(plugin),
                        });
                    } else {
                        editor.plugin_commands.open = Some(plugin.type_id().to_owned());
                    }
                    ui.close();
                }
            }
            if !any {
                ui.add_enabled(false, Button::new(tr("No commands installed")));
            }
        });
        ui.menu_button(tr("Script"), |ui| {
            // The library's scripts: a click runs one.
            let mut chosen = None;
            for name in editor.scripts.names() {
                if ui.button(name).clicked() {
                    chosen = Some(name.clone());
                }
            }
            if editor.scripts.names().is_empty() {
                ui.add_enabled(false, Button::new(tr("No scripts in the library")));
            }
            if let Some(name) = chosen {
                editor.commands.push(EditorCommand::RunLibraryScript(name));
                ui.close();
            }
            ui.separator();
            if ui
                .button(tr("Script Manager…"))
                .on_hover_text(tr("Edits, saves and runs scripts; shows what they print"))
                .clicked()
            {
                editor.scripts.show = true;
                ui.close();
            }
            if ui
                .button(tr("Run Script File…"))
                .on_hover_text(tr("Runs a Rhai script (.rhai) on the document: one undo step"))
                .clicked()
            {
                editor.commands.push(EditorCommand::RunScriptFile);
                ui.close();
            }
        });
        ui.menu_button(tr("Window"), |ui| {
            command(ui, editor, "Frame Scene", "H", EditorCommand::FrameScene);
            command(ui, editor, "Frame Selected Objects", "O", EditorCommand::FrameSelected);
            command(ui, editor, "Reset View", "", EditorCommand::ResetView);
            ui.separator();
            for pane in Pane::ALL {
                let shortcut = match pane {
                    Pane::Materials => "Shift+F2",
                    Pane::Attributes => "Shift+F5",
                    Pane::Coordinates => "Shift+F7",
                    _ => "",
                };
                let item = Button::new(tr(pane.window_label()))
                    .shortcut_text(shortcut)
                    .selected(editor.panes.is_shown(pane));
                if ui.add(item).clicked() {
                    editor.panes.show = Some(pane);
                    ui.close();
                }
            }
            let viewer = Button::new(tr("Picture Viewer")).selected(editor.renders.viewer_open);
            if ui.add(viewer).clicked() {
                editor.renders.viewer_open = !editor.renders.viewer_open;
                ui.close();
            }
            ui.separator();
            if ui
                .button(tr("Reset Layout"))
                .on_hover_text(tr("Put the current layout back as it was"))
                .clicked()
            {
                editor.panes.reset = true;
                ui.close();
            }
        });

        ui.menu_button(tr("Help"), |ui| {
            if ui.button(tr("Keyboard Shortcuts & About")).clicked() {
                editor.show_about = true;
                ui.close();
            }
        });

        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button(
                RichText::new(trf("Layout: {0}", &[&tr(editor.panes.current.label())]))
                    .color(p().text_dim)
                    .size(11.5),
                |ui| {
                    for layout in PaneLayout::ALL {
                        let item = Button::new(tr(layout.label())).selected(editor.panes.current == layout);
                        if ui.add(item).clicked() {
                            editor.panes.layout = Some(layout);
                            ui.close();
                        }
                    }
                },
            );
        });
    });
}

fn command(ui: &mut Ui, editor: &mut EditorState, label: &str, shortcut: &str, cmd: EditorCommand) {
    if ui.add(Button::new(tr(label)).shortcut_text(shortcut)).clicked() {
        editor.commands.push(cmd);
        ui.close();
    }
}

/// File ▸ Open Recent: newest first, full path on hover, and Clear.
fn recent_menu(ui: &mut Ui, editor: &mut EditorState) {
    let mut open = None;
    if editor.file.recent.is_empty() {
        ui.add_enabled(false, Button::new(tr("No recent documents")));
    }
    for path in editor.file.recent.paths() {
        let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
        if ui.button(name).on_hover_text(path.display().to_string()).clicked() {
            open = Some(path.clone());
        }
    }
    ui.separator();
    if ui
        .add_enabled(!editor.file.recent.is_empty(), Button::new(tr("Clear Recent")))
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
    if ui.button(tr(tag.label())).clicked() {
        editor.commands.push(EditorCommand::CreateObject(tag));
        ui.close();
    }
}

fn tool_item(ui: &mut Ui, editor: &mut EditorState, tool: Tool) {
    let button = Button::new(tr(tool.label()))
        .shortcut_text(tool.shortcut())
        .selected(editor.tool == tool);
    if ui.add(button).clicked() {
        editor.tool = tool;
        ui.close();
    }
}

/// A disabled menu entry for a feature on the roadmap.
fn planned(ui: &mut Ui, label: &str, shortcut: &str) {
    ui.add_enabled(false, Button::new(tr(label)).shortcut_text(shortcut))
        .on_disabled_hover_text(tr("Planned — not implemented yet"));
}

/// Mesh ▸ UV: the UV tools on the active polygon object.
fn uv_menu(ui: &mut Ui, editor: &mut EditorState, enabled: bool) {
    use crate::uv_tools::{PROJECTIONS, UvOp};
    use grid4d_geom::uv::FlipAxis;

    let mut item = |ui: &mut Ui, op: UvOp, label: &str| {
        if ui
            .add_enabled(enabled, Button::new(tr(label)))
            .on_disabled_hover_text(tr("Select a polygon object"))
            .clicked()
        {
            editor.commands.push(EditorCommand::UvCommand(op));
            ui.close();
        }
    };
    item(ui, UvOp::Unwrap, key("Unwrap"));
    item(ui, UvOp::Relax, key("Relax"));
    item(ui, UvOp::Pack, key("Pack Islands"));
    ui.menu_button(tr("Project"), |ui| {
        for (projection, label) in PROJECTIONS {
            item(ui, UvOp::Project(projection), label);
        }
    });
    ui.separator();
    item(ui, UvOp::Flip(FlipAxis::U), key("Flip U"));
    item(ui, UvOp::Flip(FlipAxis::V), key("Flip V"));
    ui.separator();
    item(ui, UvOp::MarkSeams, key("Mark Seams"));
    item(ui, UvOp::ClearSeams, key("Clear Seams"));
}
