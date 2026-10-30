//! The Take Manager: Main and the document's takes, the active one, its
//! camera and its overrides. Every change is an undoable command; the document
//! holds the active take's values (Auto Take records edits made in it). Spec:
//! `docs/reference/manager-take.md`.

use egui::{Button, ComboBox, Margin, RichText, ScrollArea, Sense, TextEdit, Ui, vec2};
use grid4d_scene::{Document, NodeId, ObjectKind, SceneEdit, TakeId};

use crate::i18n::{tr, trf};
use crate::{
    EditorCommand, EditorState,
    theme::{metrics, palette as p},
    widgets::section_heading,
};

#[derive(Debug, Default)]
pub struct TakesPanel {
    /// The take being renamed and the text typed so far.
    renaming: Option<(TakeId, String)>,
    rename_needs_focus: bool,
}

impl TakesPanel {
    pub fn show(&mut self, ui: &mut Ui, doc: &Document, editor: &mut EditorState) {
        egui::Frame::new().inner_margin(Margin::symmetric(4, 2)).show(ui, |ui| {
            if ui
                .button(tr("New Take"))
                .on_hover_text(tr("Add a take and make it active: changes are then recorded in it"))
                .clicked()
            {
                editor.commands.push(EditorCommand::NewTake);
            }
        });
        ScrollArea::vertical()
            .id_salt("grid4d.takes.scroll")
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                let active = doc.takes.active();
                if take_row(ui, "Main", active.is_none(), 0).clicked() {
                    editor.commands.push(EditorCommand::SwitchTake(None));
                }
                for (id, take) in doc.takes.iter() {
                    self.row(ui, doc, editor, id, &take.name, active == Some(id));
                }
                ui.add_space(6.0);
                match active.and_then(|id| doc.takes.get(id).map(|t| (id, t))) {
                    Some((id, take)) => active_take(ui, doc, editor, id, take),
                    None => {
                        ui.label(
                            RichText::new(tr(
                                "Main is the scene as it is. Make a take active to record changes in it.",
                            ))
                            .color(p().text_dim),
                        );
                    }
                }
            });
    }

    fn row(&mut self, ui: &mut Ui, doc: &Document, editor: &mut EditorState, id: TakeId, name: &str, active: bool) {
        if let Some((renaming, text)) = self.renaming.as_mut().filter(|(r, _)| *r == id) {
            let edit = ui.add(TextEdit::singleline(text).desired_width(f32::INFINITY));
            if std::mem::take(&mut self.rename_needs_focus) {
                edit.request_focus();
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.renaming = None;
            } else if edit.lost_focus() {
                let name = text.trim().to_owned();
                let id = *renaming;
                self.renaming = None;
                if !name.is_empty() && doc.takes.get(id).is_some_and(|t| t.name != name) {
                    let mut takes = doc.takes.clone();
                    if let Some(take) = takes.get_mut(id) {
                        take.name = name;
                    }
                    editor.commands.push(EditorCommand::Edit {
                        edit: SceneEdit::SetTakes(takes),
                        label: "Rename Take",
                    });
                }
            }
            return;
        }
        let count = doc.takes.get(id).map_or(0, grid4d_scene::Take::override_count);
        let response = take_row(ui, name, active, count);
        if response.double_clicked() {
            self.renaming = Some((id, name.to_owned()));
            self.rename_needs_focus = true;
        } else if response.clicked() {
            editor.commands.push(EditorCommand::SwitchTake(Some(id)));
        }
        response.context_menu(|ui| {
            if ui.button(tr("Rename")).clicked() {
                self.renaming = Some((id, name.to_owned()));
                self.rename_needs_focus = true;
                ui.close();
            }
            if ui.button(tr("Delete Take")).clicked() {
                editor.commands.push(EditorCommand::DeleteTake(id));
                ui.close();
            }
        });
    }
}

/// A take's row: a mark for the active one, its name, how many overrides.
fn take_row(ui: &mut Ui, name: &str, active: bool, overrides: usize) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), metrics::ROW_HEIGHT), Sense::click());
    let painter = ui.painter();
    if active {
        painter.rect_filled(rect, 0.0, p().accent_bg);
    } else if response.hovered() {
        painter.rect_filled(rect, 0.0, p().bg_hover);
    }
    let mark = rect.left_center() + vec2(10.0, 0.0);
    if active {
        painter.circle_filled(mark, 4.0, p().accent);
    } else {
        painter.circle_stroke(mark, 4.0, egui::Stroke::new(1.0, p().text_faint));
    }
    let font = egui::TextStyle::Body.resolve(ui.style());
    let colour = if active { p().text_strong } else { p().text };
    painter.text(
        rect.left_center() + vec2(22.0, 0.0),
        egui::Align2::LEFT_CENTER,
        name,
        font,
        colour,
    );
    if overrides > 0 {
        painter.text(
            rect.right_center() - vec2(6.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            if overrides == 1 {
                tr("1 override").to_owned()
            } else {
                trf("{0} overrides", &[&overrides])
            },
            egui::FontId::proportional(11.0),
            p().text_dim,
        );
    }
    response.on_hover_text(if name == "Main" {
        tr("The scene as it is")
    } else {
        tr("Click: make it active · double-click: rename · right-click: more")
    })
}

/// The active take's camera and overrides.
fn active_take(ui: &mut Ui, doc: &Document, editor: &mut EditorState, id: TakeId, take: &grid4d_scene::Take) {
    section_heading(ui, &trf("Take “{0}”", &[&take.name]));
    ui.horizontal(|ui| {
        ui.label(tr("Camera"));
        let name_of = |camera: Option<NodeId>| {
            camera
                .and_then(|c| doc.scene.get(c))
                .map_or_else(|| "The view's".to_owned(), |n| n.data.name.clone())
        };
        let mut chosen = take.camera;
        ComboBox::from_id_salt("grid4d.takes.camera")
            .selected_text(name_of(take.camera))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut chosen, None, tr("The view's"));
                for (camera, node) in doc.scene.iter() {
                    if matches!(node.data.kind, ObjectKind::Camera { .. }) {
                        ui.selectable_value(&mut chosen, Some(camera), &node.data.name);
                    }
                }
            })
            .response
            .on_hover_text(tr("The camera renders of this take look through"));
        if chosen != take.camera {
            let mut takes = doc.takes.clone();
            if let Some(t) = takes.get_mut(id) {
                t.camera = chosen;
            }
            editor.commands.push(EditorCommand::Edit {
                edit: SceneEdit::SetTakes(takes),
                label: "Take Camera",
            });
        }
    });
    ui.add_space(4.0);
    if take.objects.is_empty() && take.render.is_none() {
        ui.label(
            RichText::new(tr("No overrides yet: what you change now is recorded in this take.")).color(p().text_dim),
        );
        return;
    }
    ui.label(RichText::new(tr("Overrides")).color(p().text_dim).size(11.5));
    for (&node, values) in &take.objects {
        let name = doc.scene.get(node).map_or("(deleted)", |n| n.data.name.as_str());
        for value in values {
            let group = value.group();
            let label = format!("{name} · {}", tr(group.label()));
            let clear = override_row(ui, &label);
            if clear {
                editor.commands.push(EditorCommand::ClearOverride {
                    take: id,
                    node: Some(node),
                    group: Some(group),
                });
            }
        }
    }
    if take.render.is_some() && override_row(ui, "Render Settings") {
        editor.commands.push(EditorCommand::ClearOverride {
            take: id,
            node: None,
            group: None,
        });
    }
}

/// One override with its Clear button; returns whether it was clicked.
fn override_row(ui: &mut Ui, label: &str) -> bool {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(Button::new(tr("Clear")).small())
                .on_hover_text(tr("Drop this override: the object shows Main's value"))
                .clicked()
        })
        .inner
    })
    .inner
}
