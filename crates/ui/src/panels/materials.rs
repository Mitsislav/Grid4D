//! The Material Manager (see `docs/reference/manager-material.md`): the
//! document's materials as round previews with their names. A tab beside the
//! Timeline and the F-Curve editor.
//!
//! A click selects a material and shows it in the Attribute Manager; a
//! double-click on empty space (or Ctrl+N over the manager) makes one; dragging a
//! material onto an object in the Object Manager or the viewport assigns it.

use egui::{
    Button, CornerRadius, FontId, Id, LayerId, Order, Rect, ScrollArea, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};
use grid4d_scene::{Document, MaterialId};

use crate::i18n::{tr, trf};
use crate::{EditorCommand, EditorState, MaterialDrag, icons, theme::palette as p};

/// Default height of the panel.
pub const DEFAULT_HEIGHT: f32 = 150.0;
const CELL: f32 = 72.0;
const BALL: f32 = 46.0;

#[derive(Debug, Default)]
pub struct MaterialsPanel;

impl MaterialsPanel {
    pub fn show(&mut self, ui: &mut Ui, doc: &mut Document, editor: &mut EditorState) {
        // A drag ends the frame after its release (others drop it on that frame).
        let (down, released) = ui.input(|i| (i.pointer.any_down(), i.pointer.any_released()));
        if editor.materials.drag.is_some() && !down && !released {
            editor.materials.drag = None;
        }
        if editor
            .materials
            .selected
            .is_some_and(|m| !doc.scene.materials().contains(m))
        {
            editor.materials.selected = None;
            editor.materials.shown = false;
        }

        ui.spacing_mut().item_spacing.y = 0.0;
        menu(ui, editor);
        let area = ui.available_rect_before_wrap();
        let background = ui.interact(area, Id::unique("grid4d.materials.area"), Sense::click());
        if ui.input(|i| i.pointer.any_pressed()) {
            editor.materials.focused = ui.rect_contains_pointer(area);
        }

        let mut clicked_material = false;
        ScrollArea::vertical()
            .id_salt("grid4d.materials.scroll")
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                    let ids: Vec<MaterialId> = doc.scene.materials().ids().to_vec();
                    for id in ids {
                        clicked_material |= cell(ui, doc, editor, id);
                    }
                });
            });
        if doc.scene.materials().is_empty() {
            ui.painter().text(
                area.center(),
                egui::Align2::CENTER_CENTER,
                tr("No materials: double-click here or press Ctrl+N to make one"),
                FontId::proportional(12.5),
                p().text_dim,
            );
        }
        // Empty space: a click clears the selection, a double-click makes a material.
        if !clicked_material {
            if background.double_clicked() {
                editor.commands.push(EditorCommand::NewMaterial);
            } else if background.clicked() {
                editor.materials.selected = None;
                editor.materials.shown = false;
            }
        }
        background.context_menu(|ui| {
            if ui
                .add(Button::new(tr("New Material")).shortcut_text("Ctrl+N"))
                .clicked()
            {
                editor.commands.push(EditorCommand::NewMaterial);
                ui.close();
            }
        });
        paint_drag(ui, doc, editor);
        if background.hovered() {
            editor.status_hint = Some(
                "Material Manager: click selects · double-click empty space makes a material · drag onto an object to assign · Delete removes",
            );
        }
    }
}

/// The manager's menus.
fn menu(ui: &mut Ui, editor: &mut EditorState) {
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(4, 1))
        .show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button(tr("Create"), |ui| {
                    if ui
                        .add(Button::new(tr("New Material")).shortcut_text("Ctrl+N"))
                        .clicked()
                    {
                        editor.commands.push(EditorCommand::NewMaterial);
                        ui.close();
                    }
                });
                let selected = editor.materials.selected;
                ui.menu_button(tr("Edit"), |ui| {
                    if ui
                        .add_enabled(selected.is_some(), Button::new(tr("Delete")).shortcut_text("Delete"))
                        .clicked()
                        && let Some(material) = selected
                    {
                        editor.commands.push(EditorCommand::DeleteMaterial(material));
                        ui.close();
                    }
                });
                ui.menu_button(tr("Material"), |ui| {
                    let can = selected.is_some() && !editor.selection.is_empty();
                    if ui
                        .add_enabled(can, Button::new(tr("Apply")))
                        .on_disabled_hover_text(tr("Select a material and objects first"))
                        .clicked()
                        && let Some(material) = selected
                    {
                        editor.commands.push(EditorCommand::AssignMaterial {
                            material,
                            nodes: editor.selection.as_slice().to_vec(),
                        });
                        ui.close();
                    }
                });
            });
        });
}

/// One material: its ball and name. Returns whether it was clicked.
fn cell(ui: &mut Ui, doc: &Document, editor: &mut EditorState, id: MaterialId) -> bool {
    let Some(material) = doc.scene.materials().get(id) else {
        return false;
    };
    let (rect, response) = ui.allocate_exact_size(vec2(CELL, CELL), Sense::click_and_drag());
    let selected = editor.materials.selected == Some(id);
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, CornerRadius::same(3), p().accent_bg);
        painter.rect_stroke(
            rect,
            CornerRadius::same(3),
            Stroke::new(1.0, p().accent),
            StrokeKind::Inside,
        );
    } else if response.hovered() {
        painter.rect_filled(rect, CornerRadius::same(3), p().bg_hover);
    }
    let ball = Rect::from_center_size(pos2(rect.center().x, rect.top() + 4.0 + BALL * 0.5), vec2(BALL, BALL));
    icons::paint_material_ball(painter, ball, material.color);
    let name = painter.layout(
        material.name.clone(),
        FontId::proportional(11.5),
        if selected { p().text_strong } else { p().text },
        CELL - 4.0,
    );
    let at = pos2(rect.center().x - name.size().x * 0.5, ball.bottom() + 3.0);
    painter.with_clip_rect(rect).galley(at, name, p().text);

    if response.drag_started() {
        editor.materials.drag = Some(MaterialDrag { material: id });
    }
    let clicked = response.clicked() || response.secondary_clicked();
    if clicked {
        editor.materials.selected = Some(id);
        editor.materials.shown = true;
        editor.panes.show = Some(crate::workspace::Pane::Attributes);
    }
    let response = response.on_hover_text(trf("{0} · drag onto an object to assign it", &[&material.name]));
    response.context_menu(|ui| {
        let can = !editor.selection.is_empty();
        if ui
            .add_enabled(can, Button::new(tr("Apply to Selected Objects")))
            .clicked()
        {
            editor.commands.push(EditorCommand::AssignMaterial {
                material: id,
                nodes: editor.selection.as_slice().to_vec(),
            });
            ui.close();
        }
        if ui.add(Button::new(tr("Delete")).shortcut_text("Delete")).clicked() {
            editor.commands.push(EditorCommand::DeleteMaterial(id));
            ui.close();
        }
    });
    clicked
}

/// The dragged material following the pointer, above everything.
fn paint_drag(ui: &Ui, doc: &Document, editor: &EditorState) {
    let (Some(drag), Some(pointer)) = (editor.materials.drag, ui.ctx().pointer_latest_pos()) else {
        return;
    };
    let Some(material) = doc.scene.materials().get(drag.material) else {
        return;
    };
    let painter = ui
        .ctx()
        .layer_painter(LayerId::new(Order::Tooltip, Id::unique("grid4d.materials.drag")));
    icons::paint_material_ball(
        &painter,
        Rect::from_center_size(pointer + vec2(16.0, 16.0), vec2(24.0, 24.0)),
        material.color,
    );
    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
}

#[cfg(test)]
mod tests {
    use egui::{Event, Modifiers, PointerButton, Pos2, RawInput};

    use super::*;

    struct Harness {
        ctx: egui::Context,
        time: f64,
        doc: Document,
        editor: EditorState,
        panel: MaterialsPanel,
    }

    impl Harness {
        fn new() -> Self {
            let mut h = Self {
                ctx: egui::Context::default(),
                time: 0.0,
                doc: Document::with_demo_scene(),
                editor: EditorState::default(),
                panel: MaterialsPanel,
            };
            h.frame(vec![]);
            h
        }

        fn frame(&mut self, events: Vec<Event>) {
            self.time += 1.0 / 60.0;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 200.0))),
                time: Some(self.time),
                events,
                ..RawInput::default()
            };
            let (doc, editor, panel) = (&mut self.doc, &mut self.editor, &mut self.panel);
            self.ctx
                .run_ui(input, |ui| panel.show(ui, doc, editor))
                .drop_without_applying_deltas();
            crate::commands::apply_pending(doc, editor, &mut crate::ViewportPanel::new(None));
        }

        fn button(pos: Pos2, pressed: bool) -> Event {
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            }
        }

        fn click(&mut self, pos: Pos2) {
            self.frame(vec![Event::PointerMoved(pos), Self::button(pos, true)]);
            self.frame(vec![Self::button(pos, false)]);
        }
    }

    /// Where the first material's ball is: the first cell of the grid, under the
    /// tabs and the menu.
    const FIRST: Pos2 = pos2(40.0, 90.0);

    #[test]
    fn double_click_makes_a_material_and_a_click_selects_it() {
        let mut h = Harness::new();
        let empty = pos2(500.0, 150.0);
        h.click(empty);
        h.click(empty);
        assert_eq!(h.doc.scene.materials().len(), 1, "a double-click on empty space");
        let id = h.doc.scene.materials().ids()[0];
        h.editor.materials = crate::MaterialSelection::default();
        for _ in 0..30 {
            h.frame(vec![]); // past the double-click delay
        }
        h.click(FIRST);
        assert_eq!(h.editor.materials.selected, Some(id));
        assert!(h.editor.materials.shown);
        assert!(h.editor.materials.focused, "Delete now removes it");
        for _ in 0..30 {
            h.frame(vec![]);
        }
        h.click(empty);
        assert_eq!(h.editor.materials.selected, None, "a click on empty space deselects");
    }

    #[test]
    fn dragging_a_material_starts_a_drag_that_ends_after_the_release() {
        let mut h = Harness::new();
        h.editor.commands.push(EditorCommand::NewMaterial);
        h.frame(vec![]);
        h.frame(vec![]); // the new cell is laid out once before it can be hit
        let id = h.doc.scene.materials().ids()[0];
        h.frame(vec![Event::PointerMoved(FIRST), Harness::button(FIRST, true)]);
        for k in 1..=6 {
            h.frame(vec![Event::PointerMoved(FIRST + vec2(10.0 * k as f32, 0.0))]);
        }
        assert_eq!(h.editor.materials.drag, Some(MaterialDrag { material: id }));
        h.frame(vec![Harness::button(FIRST + vec2(60.0, 0.0), false)]);
        assert!(h.editor.materials.drag.is_some(), "others can still drop it this frame");
        h.frame(vec![]);
        assert_eq!(h.editor.materials.drag, None);
    }
}
