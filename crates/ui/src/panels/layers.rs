//! The Layer Manager: the document's layers with their colour, name and
//! switches. Changes are undoable commands (colours one step per picker
//! interaction). Spec: `docs/reference/manager-layer.md`.

use egui::{
    Align2, Button, CornerRadius, Margin, Rect, RichText, ScrollArea, Sense, Stroke, StrokeKind, TextEdit, TextStyle,
    Ui, UiBuilder, pos2, vec2,
};
use grid4d_scene::{Document, LayerId, LayerSwitches, Layers, NodeId, SceneEdit};

use crate::{
    EditorCommand, EditorState,
    live_edit::{LiveEdit, interaction_ongoing},
    theme::{metrics, palette as p},
};

/// Reaches one switch of a layer's.
type Switch = fn(&mut LayerSwitches) -> &mut bool;

/// The switch columns: letter, name, and the switch.
const SWITCHES: [(&str, &str, Switch); 8] = [
    ("S", "Solo: only soloed layers' objects show", |s| &mut s.solo),
    ("V", "View: shown in the views", |s| &mut s.view),
    ("R", "Render: shown in renders", |s| &mut s.render),
    ("M", "Manager: listed in the Object Manager", |s| &mut s.manager),
    ("L", "Locked: not pickable or movable in the views", |s| &mut s.locked),
    ("A", "Animation: animated (off: shown unanimated)", |s| &mut s.animation),
    (
        "G",
        "Generators: generators build (off: they act as switched off)",
        |s| &mut s.generators,
    ),
    ("D", "Deformers: deformers deform", |s| &mut s.deformers),
];
const SWITCH_WIDTH: f32 = 18.0;
/// Where a row's name starts (after the colour chip).
const NAME_LEFT: f32 = 48.0;

/// Column `index` of the switches, counted from the right edge of `row`.
fn switch_rect(row: Rect, index: usize) -> Rect {
    let step = SWITCH_WIDTH + 2.0;
    let left = row.right() - 4.0 - SWITCHES.len() as f32 * step + index as f32 * step;
    Rect::from_min_size(pos2(left, row.center().y - 9.0), vec2(SWITCH_WIDTH, 18.0))
}

#[derive(Debug, Default)]
pub struct LayersPanel {
    /// The layer being renamed and the text typed so far.
    renaming: Option<(LayerId, String)>,
    rename_needs_focus: bool,
    /// Colour changes made live in a picker, recorded as one step.
    colours: LiveEdit<(), Layers>,
}

/// What a row asks for; applied after the rows are drawn.
enum RowAction {
    Toggle(LayerId, usize),
    StartRename(LayerId),
    Rename(LayerId, String),
    Delete(LayerId),
    Assign(Option<LayerId>, Vec<NodeId>),
    /// Takes these objects off this layer (those on it).
    RemoveFrom(LayerId, Vec<NodeId>),
    Select(LayerId),
}

impl LayersPanel {
    pub fn show(&mut self, ui: &mut Ui, doc: &mut Document, editor: &mut EditorState) {
        self.colours.begin((), doc.scene.layers());
        let mut actions = Vec::new();
        egui::Frame::new().inner_margin(Margin::symmetric(4, 2)).show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("New Layer").on_hover_text("Add a layer").clicked() {
                    let mut layers = doc.scene.layers().clone();
                    let layer = layers.next_layer("Layer");
                    layers.insert(layer);
                    editor.commands.push(EditorCommand::Edit {
                        edit: SceneEdit::SetLayers(layers),
                        label: "New Layer",
                    });
                }
            });
        });
        let ongoing = interaction_ongoing(ui);
        ScrollArea::vertical()
            .id_salt("grid4d.layers.scroll")
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                if doc.scene.layers().is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(
                            "No layers yet. New Layer adds one; then drag objects from the Object Manager onto it.",
                        )
                        .color(p().text_dim),
                    );
                    return;
                }
                self.header(ui);
                let ids = doc.scene.layers().ids().to_vec();
                for id in ids {
                    self.row(ui, doc, editor, id, &mut actions);
                }
            });
        self.apply(actions, doc, editor);
        if let Some(((), before)) = self.colours.end((), doc.scene.layers(), ongoing) {
            editor.commands.push(EditorCommand::CommitLive {
                inverse: SceneEdit::SetLayers(before),
                label: "Layer Colour",
            });
        }
    }

    /// The switch letters over their columns.
    fn header(&self, ui: &mut Ui) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
        let font = egui::FontId::proportional(11.0);
        ui.painter().text(
            pos2(rect.left() + NAME_LEFT, rect.center().y),
            Align2::LEFT_CENTER,
            "Layer",
            font.clone(),
            p().text_dim,
        );
        for (index, (letter, tip, _)) in SWITCHES.iter().enumerate() {
            let column = switch_rect(rect, index);
            ui.painter().text(
                column.center(),
                Align2::CENTER_CENTER,
                *letter,
                font.clone(),
                p().text_dim,
            );
            ui.interact(column, ui.scope_id().with(("layers.header", index)), Sense::hover())
                .on_hover_text(*tip);
        }
    }

    fn row(
        &mut self,
        ui: &mut Ui,
        doc: &mut Document,
        editor: &EditorState,
        id: LayerId,
        actions: &mut Vec<RowAction>,
    ) {
        let Some(layer) = doc.scene.layers().get(id).cloned() else {
            return;
        };
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), metrics::ROW_HEIGHT), Sense::hover());
        // The colour chip: its picker writes live (one undo step).
        let chip = Rect::from_min_size(pos2(rect.left() + 4.0, rect.center().y - 8.0), vec2(26.0, 16.0));
        let mut colour = layer.color;
        let picked = ui
            .scope_builder(UiBuilder::new().max_rect(chip), |ui| {
                ui.color_edit_button_rgb(&mut colour)
            })
            .inner;
        if picked.changed()
            && let Some(l) = doc.scene.layers_mut().get_mut(id)
        {
            l.color = colour;
        }
        let name_rect = Rect::from_min_max(
            pos2(rect.left() + NAME_LEFT, rect.top() + 1.0),
            pos2(
                (switch_rect(rect, 0).left() - 4.0).max(rect.left() + NAME_LEFT + 20.0),
                rect.bottom() - 1.0,
            ),
        );
        match &mut self.renaming {
            Some((renaming, text)) if *renaming == id => {
                let edit = ui.put(name_rect, TextEdit::singleline(text).margin(vec2(2.0, 1.0)));
                if std::mem::take(&mut self.rename_needs_focus) {
                    edit.request_focus();
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.renaming = None;
                } else if edit.lost_focus() {
                    actions.push(RowAction::Rename(id, std::mem::take(text)));
                }
            }
            _ => {
                let name = ui
                    .interact(name_rect, ui.scope_id().with(("layers.name", id)), Sense::click())
                    .on_hover_text("Double-click to rename; right-click for more");
                ui.painter_at(name_rect).text(
                    pos2(name_rect.left(), name_rect.center().y),
                    Align2::LEFT_CENTER,
                    &layer.name,
                    TextStyle::Body.resolve(ui.style()),
                    p().text,
                );
                if name.double_clicked() {
                    actions.push(RowAction::StartRename(id));
                }
                layer_menu(&name, id, editor, actions);
            }
        }
        for (index, (letter, tip, switch)) in SWITCHES.iter().enumerate() {
            let mut switches = layer.switches;
            let on = *switch(&mut switches);
            let button = Button::selectable(on, RichText::new(*letter).size(11.0));
            if ui.put(switch_rect(rect, index), button).on_hover_text(*tip).clicked() {
                actions.push(RowAction::Toggle(id, index));
            }
        }
        // Objects dragged from the Object Manager go on the layer.
        if let Some(dragged) = editor.object_drag
            && ui.rect_contains_pointer(rect)
        {
            ui.painter().rect_stroke(
                rect.expand(1.0),
                CornerRadius::same(2),
                Stroke::new(1.5, p().accent),
                StrokeKind::Inside,
            );
            if ui.input(|i| i.pointer.any_released()) {
                let objects = if editor.selection.contains(dragged) {
                    editor.selection.as_slice().to_vec()
                } else {
                    vec![dragged]
                };
                actions.push(RowAction::Assign(Some(id), objects));
            }
        }
        ui.add_space(1.0);
    }

    fn apply(&mut self, actions: Vec<RowAction>, doc: &Document, editor: &mut EditorState) {
        let layers = doc.scene.layers();
        for action in actions {
            match action {
                RowAction::Toggle(id, index) => {
                    let mut layers = layers.clone();
                    if let Some(layer) = layers.get_mut(id) {
                        let switch = SWITCHES[index].2;
                        let value = switch(&mut layer.switches);
                        *value = !*value;
                    }
                    editor.commands.push(EditorCommand::Edit {
                        edit: SceneEdit::SetLayers(layers),
                        label: "Layer Switch",
                    });
                }
                RowAction::StartRename(id) => {
                    let name = layers.get(id).map(|l| l.name.clone()).unwrap_or_default();
                    self.renaming = Some((id, name));
                    self.rename_needs_focus = true;
                }
                RowAction::Rename(id, name) => {
                    self.renaming = None;
                    let name = name.trim();
                    if !name.is_empty() && layers.get(id).is_some_and(|l| l.name != name) {
                        let mut layers = layers.clone();
                        if let Some(layer) = layers.get_mut(id) {
                            layer.name = name.to_owned();
                        }
                        editor.commands.push(EditorCommand::Edit {
                            edit: SceneEdit::SetLayers(layers),
                            label: "Rename Layer",
                        });
                    }
                }
                RowAction::Delete(id) => {
                    let mut layers = layers.clone();
                    layers.remove(id);
                    editor.commands.push(EditorCommand::Edit {
                        edit: SceneEdit::SetLayers(layers),
                        label: "Delete Layer",
                    });
                }
                RowAction::Assign(layer, objects) => {
                    editor.commands.push(EditorCommand::AssignLayer { layer, objects });
                }
                RowAction::RemoveFrom(id, objects) => {
                    let objects = objects
                        .into_iter()
                        .filter(|&o| doc.scene.get(o).is_some_and(|n| n.data.layer == Some(id)))
                        .collect();
                    editor
                        .commands
                        .push(EditorCommand::AssignLayer { layer: None, objects });
                }
                RowAction::Select(id) => editor.commands.push(EditorCommand::SelectLayerObjects(id)),
            }
        }
    }
}

/// A layer's right-click menu.
fn layer_menu(response: &egui::Response, id: LayerId, editor: &EditorState, actions: &mut Vec<RowAction>) {
    let selected = editor.selection.as_slice().to_vec();
    response.context_menu(|ui| {
        let any = !selected.is_empty();
        if ui
            .add_enabled(any, Button::new("Add Selected Objects"))
            .on_disabled_hover_text("Select objects first")
            .clicked()
        {
            actions.push(RowAction::Assign(Some(id), selected.clone()));
            ui.close();
        }
        if ui.add_enabled(any, Button::new("Remove Selected Objects")).clicked() {
            actions.push(RowAction::RemoveFrom(id, selected.clone()));
            ui.close();
        }
        if ui.button("Select Objects").clicked() {
            actions.push(RowAction::Select(id));
            ui.close();
        }
        ui.separator();
        if ui.button("Rename").clicked() {
            actions.push(RowAction::StartRename(id));
            ui.close();
        }
        if ui.button("Delete Layer").clicked() {
            actions.push(RowAction::Delete(id));
            ui.close();
        }
    });
}

#[cfg(test)]
mod tests {
    use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Shape};
    use grid4d_scene::Layer;

    use super::*;

    /// One frame of the panel; returns where each text was drawn.
    fn frame(
        ctx: &egui::Context,
        panel: &mut LayersPanel,
        doc: &mut Document,
        editor: &mut EditorState,
        events: Vec<Event>,
    ) -> Vec<(String, Pos2)> {
        let input = RawInput {
            events,
            ..RawInput::default()
        };
        let mut output = ctx.run_ui(input, |ui| panel.show(ui, doc, editor));
        output.textures_delta.clear();
        output
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                Shape::Text(t) => Some((t.galley.text().to_owned(), t.visual_bounding_rect().center())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn an_object_dropped_on_a_layer_goes_on_it_with_the_selection() {
        let ctx = egui::Context::default();
        let mut panel = LayersPanel::default();
        let mut doc = Document::with_demo_scene();
        let mut editor = EditorState::default();
        let set = doc.scene.layers_mut().insert(Layer::new("Set", [1.0; 3]));
        doc.scene.layers_mut().insert(Layer::new("Props", [1.0; 3]));
        let (cube, sphere) = (doc.scene.roots()[0], doc.scene.roots()[1]);
        editor.selection.set_all([cube, sphere]);

        let texts = frame(&ctx, &mut panel, &mut doc, &mut editor, Vec::new());
        let at = texts.iter().find(|(t, _)| t == "Set").unwrap().1;
        assert!(texts.iter().any(|(t, _)| t == "Props"));
        assert!(editor.commands.is_empty(), "showing changes nothing");

        // The Object Manager reports its drag; the pointer lets go over "Set".
        editor.object_drag = Some(sphere);
        frame(&ctx, &mut panel, &mut doc, &mut editor, vec![Event::PointerMoved(at)]);
        assert!(editor.commands.is_empty());
        let release = Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        };
        frame(&ctx, &mut panel, &mut doc, &mut editor, vec![release]);
        let pending = editor.commands.pending();
        assert!(
            matches!(
                pending,
                [EditorCommand::AssignLayer { layer: Some(l), objects }] if *l == set && objects == &[cube, sphere]
            ),
            "{pending:?}"
        );
    }
}
