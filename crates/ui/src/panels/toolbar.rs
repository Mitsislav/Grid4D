//! The command palette under the menu bar: undo/redo, tools, axis locks,
//! coordinate system, render buttons and object creation.

use egui::{Align2, CornerRadius, FontId, Sense, Ui, Vec2, vec2};
use grid4d_scene::ObjectKindTag;

use crate::i18n::{key, tr, trf};
use crate::{
    EditorCommand, EditorState, Tool,
    commands::tag_color,
    icons::Icon,
    theme::{metrics, palette as p},
    widgets::{IconStyle, icon_button, icon_button_bare},
};

pub fn show(ui: &mut Ui, editor: &mut EditorState) {
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;

        history_button(ui, editor, HistoryStep::Undo);
        history_button(ui, editor, HistoryStep::Redo);
        group_gap(ui);

        for tool in Tool::TOOLBAR {
            let style = IconStyle {
                selected: editor.tool == tool,
                ..IconStyle::default()
            };
            let response = icon_button(ui, tool_icon(tool), style, tool_tooltip(tool));
            if response.hovered() {
                editor.status_hint = Some(tool.hint());
            }
            if response.clicked() {
                editor.tool = tool;
            }
        }
        group_gap(ui);

        for (axis, label) in ["X", "Y", "Z"].into_iter().enumerate() {
            axis_toggle(ui, &mut editor.axes[axis], label, p().axis[axis]);
        }
        let (icon, tip) = if editor.world_coordinates {
            (Icon::CoordWorld, key("Coordinate System: World (click for Object)"))
        } else {
            (Icon::CoordObject, key("Coordinate System: Object (click for World)"))
        };
        if icon_button(ui, icon, IconStyle::default(), tip).clicked() {
            editor.world_coordinates = !editor.world_coordinates;
        }
        group_gap(ui);

        let view_style = IconStyle {
            selected: editor.renders.view_render().is_some(),
            ..IconStyle::default()
        };
        if icon_button(ui, Icon::RenderView, view_style, "Render View (Ctrl+R)").clicked() {
            editor.commands.push(EditorCommand::RenderView);
        }
        if icon_button(
            ui,
            Icon::RenderPictureViewer,
            IconStyle::default(),
            "Render to Picture Viewer (Shift+R)",
        )
        .clicked()
        {
            editor.commands.push(EditorCommand::RenderToPictureViewer);
        }
        let settings = IconStyle {
            selected: editor.show_render_settings,
            ..IconStyle::default()
        };
        if icon_button(ui, Icon::RenderSettings, settings, "Edit Render Settings (Ctrl+B)").clicked() {
            editor.show_render_settings = !editor.show_render_settings;
        }
        group_gap(ui);

        let others = [ObjectKindTag::Null, ObjectKindTag::Camera, ObjectKindTag::Light];
        for tag in ObjectKindTag::PRIMITIVES.into_iter().chain(others) {
            let style = IconStyle {
                tint: Some(tag_color(tag)),
                ..IconStyle::default()
            };
            if icon_button(ui, Icon::Object(tag), style, create_tooltip(tag)).clicked() {
                editor.commands.push(EditorCommand::CreateObject(tag));
            }
        }
        // One button for the splines, opening their list.
        let style = IconStyle {
            tint: Some(tag_color(ObjectKindTag::Circle)),
            ..IconStyle::default()
        };
        let splines = icon_button(
            ui,
            Icon::Object(ObjectKindTag::Circle),
            style,
            "Add Spline (Circle, Rectangle, …)",
        );
        egui::Popup::menu(&splines).show(|ui| {
            for tag in ObjectKindTag::SPLINES {
                let label = trf("Add {0}", &[&tr(tag.label())]);
                if ui.button(label).clicked() {
                    editor.commands.push(EditorCommand::CreateObject(tag));
                    ui.close();
                }
            }
        });
        group_gap(ui);
        for tag in ObjectKindTag::GENERATORS {
            let style = IconStyle {
                tint: Some(tag_color(tag)),
                ..IconStyle::default()
            };
            if icon_button(ui, Icon::Object(tag), style, create_tooltip(tag)).clicked() {
                // Alt + click wraps the selected object, as in C4D.
                let alt = ui.input(|i| i.modifiers.alt);
                editor.commands.push(if alt {
                    EditorCommand::WrapInGenerator(tag)
                } else {
                    EditorCommand::CreateObject(tag)
                });
            }
        }
        for tag in ObjectKindTag::EFFECTORS {
            let style = IconStyle {
                tint: Some(tag_color(tag)),
                ..IconStyle::default()
            };
            if icon_button(ui, Icon::Object(tag), style, create_tooltip(tag)).clicked() {
                editor.commands.push(EditorCommand::CreateObject(tag));
            }
        }
        group_gap(ui);
        for tag in ObjectKindTag::DEFORMERS {
            let style = IconStyle {
                tint: Some(tag_color(tag)),
                ..IconStyle::default()
            };
            if icon_button(ui, Icon::Object(tag), style, create_tooltip(tag)).clicked() {
                // Shift + click: under the selected object, fitted to it.
                let shift = ui.input(|i| i.modifiers.shift);
                editor.commands.push(if shift {
                    EditorCommand::CreateChild(tag)
                } else {
                    EditorCommand::CreateObject(tag)
                });
            }
        }
    });
}

#[derive(Clone, Copy)]
enum HistoryStep {
    Undo,
    Redo,
}

/// Undo or Redo, enabled when there is a step, with the step's name as tooltip
/// ("Undo Move (Ctrl+Z)").
fn history_button(ui: &mut Ui, editor: &mut EditorState, step: HistoryStep) {
    let (icon, verb, shortcut, empty, label, command) = match step {
        HistoryStep::Undo => (
            Icon::Undo,
            key("Undo"),
            "Ctrl+Z",
            key("Undo (Ctrl+Z) — nothing to undo"),
            editor.history.undo_label(),
            EditorCommand::Undo,
        ),
        HistoryStep::Redo => (
            Icon::Redo,
            key("Redo"),
            "Ctrl+Y",
            key("Redo (Ctrl+Y) — nothing to redo"),
            editor.history.redo_label(),
            EditorCommand::Redo,
        ),
    };
    let style = IconStyle {
        enabled: label.is_some(),
        ..IconStyle::default()
    };
    // The tooltip text is only built while hovered.
    let response = icon_button_bare(ui, icon, style).on_hover_ui(|ui| match label {
        Some(label) => {
            ui.label(format!("{} {} ({shortcut})", tr(verb), tr(label)));
        }
        None => {
            ui.label(tr(empty));
        }
    });
    if response.clicked() {
        editor.commands.push(command);
    }
}

fn group_gap(ui: &mut Ui) {
    ui.add_space(4.0);
    ui.separator();
    ui.add_space(4.0);
}

/// C4D's X / Y / Z lock buttons: coloured letter when enabled, grey when locked.
fn axis_toggle(ui: &mut Ui, enabled: &mut bool, label: &'static str, color: egui::Color32) {
    let (rect, response) = ui.allocate_exact_size(vec2(22.0, metrics::ICON_BUTTON), Sense::click());
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect.shrink(1.0), CornerRadius::same(3), p().bg_hover);
    }
    let text_color = if *enabled { color } else { p().text_faint };
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(15.0),
        text_color,
    );
    if *enabled {
        let underline = egui::Rect::from_center_size(rect.center_bottom() - vec2(0.0, 5.0), Vec2::new(10.0, 2.0));
        painter.rect_filled(underline, CornerRadius::same(1), color.gamma_multiply(0.7));
    }
    if response
        .on_hover_text(tr("Axis lock: enable/disable this axis for the active tool"))
        .clicked()
    {
        *enabled = !*enabled;
    }
}

const fn tool_icon(tool: Tool) -> Icon {
    match tool {
        Tool::LiveSelection => Icon::LiveSelection,
        Tool::RectangleSelection => Icon::RectangleSelection,
        Tool::LoopSelection => Icon::LoopSelection,
        Tool::RingSelection => Icon::RingSelection,
        Tool::Move => Icon::Move,
        Tool::Scale => Icon::Scale,
        Tool::Rotate => Icon::Rotate,
        // Not on the toolbar (Mesh menu and shortcuts); a plain arrow if ever shown.
        Tool::Extrude
        | Tool::Inset
        | Tool::LoopCut
        | Tool::LineCut
        | Tool::Bevel
        | Tool::Bridge
        | Tool::Weld
        | Tool::Pen => Icon::Move,
    }
}

const fn tool_tooltip(tool: Tool) -> &'static str {
    match tool {
        Tool::LiveSelection => key("Live Selection (9)"),
        Tool::RectangleSelection => key("Rectangle Selection (0)"),
        Tool::LoopSelection => key("Loop Selection (U~L)"),
        Tool::RingSelection => key("Ring Selection (U~B)"),
        Tool::Move => key("Move (E)"),
        Tool::Scale => key("Scale (T)"),
        Tool::Rotate => key("Rotate (R)"),
        Tool::Extrude => key("Extrude (D)"),
        Tool::Inset => key("Inset (I)"),
        Tool::LoopCut => key("Loop/Path Cut (K~L)"),
        Tool::LineCut => key("Line Cut (K~K)"),
        Tool::Bevel => key("Bevel (M~S)"),
        Tool::Bridge => key("Bridge (B)"),
        Tool::Weld => "Weld",
        Tool::Pen => "Pen",
    }
}

const fn create_tooltip(tag: ObjectKindTag) -> &'static str {
    match tag {
        ObjectKindTag::Null => key("Add Null"),
        ObjectKindTag::Cube => key("Add Cube"),
        ObjectKindTag::Sphere => key("Add Sphere"),
        ObjectKindTag::Cylinder => key("Add Cylinder"),
        ObjectKindTag::Plane => key("Add Plane"),
        ObjectKindTag::Cone => key("Add Cone"),
        ObjectKindTag::Torus => key("Add Torus"),
        ObjectKindTag::Disc => key("Add Disc"),
        ObjectKindTag::Capsule => key("Add Capsule"),
        ObjectKindTag::Camera => key("Add Camera (at the current view)"),
        ObjectKindTag::Light => key("Add Light"),
        ObjectKindTag::Sky => key("Add Sky"),
        ObjectKindTag::Environment => key("Add Environment"),
        ObjectKindTag::Polygon => key("Add Polygon Object"),
        ObjectKindTag::SubdivisionSurface => key("Add Subdivision Surface (Alt: around the selected object)"),
        ObjectKindTag::Extrude => key("Add Extrude (Alt: around the selected spline)"),
        ObjectKindTag::Lathe => key("Add Lathe (Alt: around the selected spline)"),
        ObjectKindTag::Sweep => key("Add Sweep: profile first, path second (Alt: around the selected spline)"),
        ObjectKindTag::Loft => key("Add Loft: a skin over its child splines (Alt: around the selected splines)"),
        ObjectKindTag::Cloner => key("Add Cloner: repeats its children (Alt: around the selected object)"),
        ObjectKindTag::PlainEffector => key("Add Plain effector: moves, turns, scales the selected Cloner's clones"),
        ObjectKindTag::RandomEffector => key("Add Random effector: varies the selected Cloner's clones at random"),
        ObjectKindTag::Bend => key("Add Bend (Shift: under the selected object, fitted to it)"),
        ObjectKindTag::Twist => key("Add Twist (Shift: under the selected object, fitted to it)"),
        ObjectKindTag::Taper => key("Add Taper (Shift: under the selected object, fitted to it)"),
        ObjectKindTag::Bulge => key("Add Bulge (Shift: under the selected object, fitted to it)"),
        ObjectKindTag::Ffd => key("Add FFD (Shift: under the selected object, fitted to it)"),
        ObjectKindTag::Circle => key("Add Circle"),
        ObjectKindTag::Rectangle => key("Add Rectangle"),
        ObjectKindTag::Arc => key("Add Arc"),
        ObjectKindTag::NSide => key("Add n-Side"),
        ObjectKindTag::Star => key("Add Star"),
        ObjectKindTag::Helix => key("Add Helix"),
        ObjectKindTag::Flower => key("Add Flower"),
        ObjectKindTag::Text => key("Add Text"),
        ObjectKindTag::Spline => key("Add Spline"),
        ObjectKindTag::Plugin => key("Add a plugin's object from Create › Plugins"),
    }
}
