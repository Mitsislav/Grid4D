//! The command palette under the menu bar: undo/redo, tools, axis locks,
//! coordinate system, render buttons and object creation.

use egui::{Align2, CornerRadius, FontId, Sense, Ui, Vec2, vec2};
use grid4d_scene::ObjectKindTag;

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

        let disabled = IconStyle {
            enabled: false,
            ..IconStyle::default()
        };
        history_button(ui, editor, HistoryStep::Undo);
        history_button(ui, editor, HistoryStep::Redo);
        group_gap(ui);

        for tool in Tool::ALL {
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
            axis_toggle(ui, &mut editor.axes[axis], label, p::AXIS[axis]);
        }
        let (icon, tip) = if editor.world_coordinates {
            (Icon::CoordWorld, "Coordinate System: World (click for Object)")
        } else {
            (Icon::CoordObject, "Coordinate System: Object (click for World)")
        };
        if icon_button(ui, icon, IconStyle::default(), tip).clicked() {
            editor.world_coordinates = !editor.world_coordinates;
        }
        group_gap(ui);

        icon_button(ui, Icon::RenderView, disabled, "Render View (Ctrl+R) — planned");
        icon_button(
            ui,
            Icon::RenderPictureViewer,
            disabled,
            "Render to Picture Viewer (Shift+R) — planned",
        );
        icon_button(
            ui,
            Icon::RenderSettings,
            disabled,
            "Edit Render Settings (Ctrl+B) — planned",
        );
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
            "Undo",
            "Ctrl+Z",
            "Undo (Ctrl+Z) — nothing to undo",
            editor.history.undo_label(),
            EditorCommand::Undo,
        ),
        HistoryStep::Redo => (
            Icon::Redo,
            "Redo",
            "Ctrl+Y",
            "Redo (Ctrl+Y) — nothing to redo",
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
            ui.label(format!("{verb} {label} ({shortcut})"));
        }
        None => {
            ui.label(empty);
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
        painter.rect_filled(rect.shrink(1.0), CornerRadius::same(3), p::BG_HOVER);
    }
    let text_color = if *enabled { color } else { p::TEXT_FAINT };
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
        .on_hover_text("Axis lock: enable/disable this axis for the active tool")
        .clicked()
    {
        *enabled = !*enabled;
    }
}

const fn tool_icon(tool: Tool) -> Icon {
    match tool {
        Tool::LiveSelection => Icon::LiveSelection,
        Tool::Move => Icon::Move,
        Tool::Scale => Icon::Scale,
        Tool::Rotate => Icon::Rotate,
    }
}

const fn tool_tooltip(tool: Tool) -> &'static str {
    match tool {
        Tool::LiveSelection => "Live Selection (9)",
        Tool::Move => "Move (E)",
        Tool::Scale => "Scale (T)",
        Tool::Rotate => "Rotate (R)",
    }
}

const fn create_tooltip(tag: ObjectKindTag) -> &'static str {
    match tag {
        ObjectKindTag::Null => "Add Null",
        ObjectKindTag::Cube => "Add Cube",
        ObjectKindTag::Sphere => "Add Sphere",
        ObjectKindTag::Cylinder => "Add Cylinder",
        ObjectKindTag::Plane => "Add Plane",
        ObjectKindTag::Cone => "Add Cone",
        ObjectKindTag::Torus => "Add Torus",
        ObjectKindTag::Disc => "Add Disc",
        ObjectKindTag::Capsule => "Add Capsule",
        ObjectKindTag::Camera => "Add Camera (at the current view)",
        ObjectKindTag::Light => "Add Light",
    }
}
