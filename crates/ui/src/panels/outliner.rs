//! Object Manager (the "Outliner"): the scene hierarchy as a tree.
//!
//! Rows show fold arrow, type icon, name, the two visibility "traffic light"
//! dots (editor / renderer) and the enabled check mark — like Cinema 4D.
//! Double-click renames, dragging a row onto another re-parents it.

use std::collections::HashSet;

use egui::{
    Align2, Button, CornerRadius, FontId, Id, Key, LayerId, Margin, Order, Rect, RichText, ScrollArea, Sense, Stroke,
    StrokeKind, TextEdit, TextStyle, Ui, Vec2, pos2, vec2,
};
use oc4d_scene::{Document, Node, NodeData, NodeId, ObjectKindTag, Scene, SceneEdit};

use crate::{
    EditorCommand, EditorState, SelectMode,
    commands::tag_color,
    icons::{self, Icon},
    theme::{metrics, palette as p},
    widgets::manager_tabs,
};

const INDENT: f32 = 16.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowAction {
    Select(NodeId, SelectMode),
    ToggleFold(NodeId),
    ToggleEditorVisibility(NodeId),
    ToggleRenderVisibility(NodeId),
    ToggleEnabled(NodeId),
    StartRename(NodeId),
    CommitRename(NodeId),
    CancelRename,
    Delete(NodeId),
    MakeRoot(NodeId),
    Frame(NodeId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DropTarget {
    Onto(NodeId),
    Root,
}

#[derive(Debug, Default)]
pub struct OutlinerPanel {
    tab: usize,
    collapsed: HashSet<NodeId>,
    filter: String,
    renaming: Option<NodeId>,
    rename_buffer: String,
    rename_needs_focus: bool,
    dragging: Option<NodeId>,
    drop_target: Option<DropTarget>,
    /// Reused every frame; row interactions are applied after the tree is drawn.
    actions: Vec<RowAction>,
}

impl OutlinerPanel {
    pub fn show(&mut self, ui: &mut Ui, doc: &mut Document, editor: &mut EditorState) {
        if let Some(tab) = manager_tabs(ui, &["Objects", "Takes"], self.tab) {
            self.tab = tab;
        }
        if self.tab != 0 {
            ui.add_space(12.0);
            ui.vertical_centered(|ui| ui.label(RichText::new("Takes — planned").color(p::TEXT_DIM)));
            return;
        }

        self.menu(ui, &doc.scene, editor);
        self.search_field(ui);

        self.actions.clear();
        self.drop_target = None;

        egui::Frame::new().fill(p::BG_PANEL).show(ui, |ui| {
            ScrollArea::vertical()
                .id_salt("oc4d.outliner.scroll")
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for &root in doc.scene.roots() {
                        self.tree(ui, &doc.scene, root, 0, editor);
                    }
                    self.empty_area(ui, editor);
                });
        });

        self.finish_drag(ui, &doc.scene, editor);
        self.apply_actions(doc, editor);
    }

    fn menu(&mut self, ui: &mut Ui, scene: &Scene, editor: &mut EditorState) {
        egui::Frame::new().inner_margin(Margin::symmetric(4, 1)).show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    ui.add_enabled(false, Button::new("Save Selection as Object…"));
                });
                ui.menu_button("Edit", |ui| {
                    for (label, cmd) in [
                        ("Select All", EditorCommand::SelectAll),
                        ("Deselect All", EditorCommand::DeselectAll),
                        ("Delete", EditorCommand::DeleteSelected),
                    ] {
                        if ui.button(label).clicked() {
                            editor.commands.push(cmd);
                            ui.close();
                        }
                    }
                });
                ui.menu_button("View", |ui| {
                    if ui.button("Expand All").clicked() {
                        self.collapsed.clear();
                        ui.close();
                    }
                    if ui.button("Collapse All").clicked() {
                        self.collapsed
                            .extend(scene.iter().filter(|(_, n)| !n.children().is_empty()).map(|(id, _)| id));
                        ui.close();
                    }
                });
                ui.menu_button("Object", |ui| {
                    for tag in ObjectKindTag::ALL {
                        if ui.button(tag.label()).clicked() {
                            editor.commands.push(EditorCommand::CreateObject(tag));
                            ui.close();
                        }
                    }
                });
                ui.menu_button("Tags", |ui| {
                    ui.add_enabled(false, Button::new("Tags — planned"));
                });
                ui.menu_button("Bookmarks", |ui| {
                    ui.add_enabled(false, Button::new("Bookmarks — planned"));
                });
            });
        });
    }

    fn search_field(&mut self, ui: &mut Ui) {
        egui::Frame::new().inner_margin(Margin::symmetric(6, 3)).show(ui, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                icons::paint(ui.painter(), rect, Icon::NavZoom, p::TEXT_DIM);
                ui.add(
                    TextEdit::singleline(&mut self.filter)
                        .hint_text("Search objects")
                        .desired_width(f32::INFINITY),
                );
            });
        });
    }

    fn tree(&mut self, ui: &mut Ui, scene: &Scene, id: NodeId, depth: u32, editor: &EditorState) {
        let Some(node) = scene.get(id) else { return };
        let filtering = !self.filter.is_empty();
        if !filtering || contains_ignore_ascii_case(&node.data.name, &self.filter) {
            self.row(ui, scene, id, node, if filtering { 0 } else { depth }, editor);
        }
        if filtering || !self.collapsed.contains(&id) {
            for &child in node.children() {
                self.tree(ui, scene, child, depth + 1, editor);
            }
        }
    }

    fn row(&mut self, ui: &mut Ui, scene: &Scene, id: NodeId, node: &Node, depth: u32, editor: &EditorState) {
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), metrics::ROW_HEIGHT), Sense::click_and_drag());
        if !ui.is_rect_visible(rect) {
            return;
        }
        let painter = ui.painter().clone();
        let selected = editor.selection.contains(id);
        let active = editor.selection.active() == Some(id);
        let cy = rect.center().y;

        // Background.
        if selected {
            let fill = if active {
                p::ACCENT_BG
            } else {
                p::ACCENT_BG.gamma_multiply(0.65)
            };
            painter.rect_filled(rect, CornerRadius::ZERO, fill);
        } else if response.hovered() {
            painter.rect_filled(rect, CornerRadius::ZERO, p::BG_HOVER.gamma_multiply(0.55));
        }

        // Drop target highlight while dragging another row.
        if let Some(dragged) = self.dragging
            && dragged != id
            && ui.rect_contains_pointer(rect)
            && !scene.is_ancestor_of(dragged, id)
        {
            self.drop_target = Some(DropTarget::Onto(id));
            painter.rect_stroke(
                rect.shrink(1.0),
                CornerRadius::same(2),
                Stroke::new(1.5, p::ACCENT),
                StrokeKind::Inside,
            );
        }

        let mut x = rect.left() + 4.0 + depth as f32 * INDENT;

        // Fold arrow.
        if !node.children().is_empty() {
            let fold_rect = Rect::from_center_size(pos2(x + 6.0, cy), Vec2::splat(12.0));
            let fold = ui.interact(fold_rect.expand(2.0), ui.scope_id().with(("fold", id)), Sense::click());
            let icon = if self.collapsed.contains(&id) {
                Icon::FoldRight
            } else {
                Icon::FoldDown
            };
            icons::paint(
                &painter,
                fold_rect,
                icon,
                if fold.hovered() { p::TEXT_STRONG } else { p::TEXT_DIM },
            );
            if fold.clicked() {
                self.actions.push(RowAction::ToggleFold(id));
            }
        }
        x += 15.0;

        // Type icon.
        let tag = node.data.kind.tag();
        icons::paint(
            &painter,
            Rect::from_min_size(pos2(x, cy - 8.0), Vec2::splat(16.0)),
            Icon::Object(tag),
            tag_color(tag),
        );
        x += 22.0;

        // Name (or the inline rename editor).
        let name_rect = Rect::from_min_max(
            pos2(x, rect.top() + 1.0),
            pos2(rect.right() - 46.0, rect.bottom() - 1.0),
        );
        if self.renaming == Some(id) {
            let edit = ui.put(
                name_rect,
                TextEdit::singleline(&mut self.rename_buffer).margin(vec2(2.0, 1.0)),
            );
            if self.rename_needs_focus {
                edit.request_focus();
                self.rename_needs_focus = false;
            }
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                self.actions.push(RowAction::CancelRename);
            } else if edit.lost_focus() {
                self.actions.push(RowAction::CommitRename(id));
            }
        } else {
            let color = match (selected, node.data.enabled) {
                (_, false) => p::TEXT_FAINT,
                (true, true) => p::TEXT_STRONG,
                (false, true) => p::TEXT,
            };
            ui.painter_at(name_rect).text(
                pos2(name_rect.left(), cy),
                Align2::LEFT_CENTER,
                &node.data.name,
                TextStyle::Body.resolve(ui.style()),
                color,
            );
        }

        // Traffic lights: upper = editor visibility, lower = render visibility.
        let dots_x = rect.right() - 32.0;
        for (offset, visible, action) in [
            (-3.5, node.data.visible_in_editor, RowAction::ToggleEditorVisibility(id)),
            (3.5, node.data.visible_in_render, RowAction::ToggleRenderVisibility(id)),
        ] {
            let center = pos2(dots_x, cy + offset);
            let hit = ui.interact(
                Rect::from_center_size(center, vec2(12.0, 7.0)),
                ui.scope_id().with(("vis", id, offset.to_bits())),
                Sense::click(),
            );
            let color = if visible { p::VIS_DEFAULT } else { p::VIS_HIDDEN };
            painter.circle_filled(center, if hit.hovered() { 3.4 } else { 2.8 }, color);
            if hit
                .on_hover_text("Visibility (upper: editor, lower: renderer)")
                .clicked()
            {
                self.actions.push(action);
            }
        }

        // Enabled check mark.
        let check_rect = Rect::from_center_size(pos2(rect.right() - 14.0, cy), Vec2::splat(13.0));
        let check = ui.interact(check_rect, ui.scope_id().with(("enabled", id)), Sense::click());
        let (icon, color) = if node.data.enabled {
            (Icon::Check, p::ENABLED)
        } else {
            (Icon::Cross, p::VIS_HIDDEN)
        };
        icons::paint(&painter, check_rect, icon, color);
        if check.on_hover_text("Enable / disable object").clicked() {
            self.actions.push(RowAction::ToggleEnabled(id));
        }

        // Row interactions.
        if response.double_clicked() {
            self.actions.push(RowAction::StartRename(id));
        } else if response.clicked() {
            let mode = SelectMode::from_modifiers(ui.input(|i| i.modifiers));
            self.actions.push(RowAction::Select(id, mode));
        }
        if response.drag_started() && self.renaming.is_none() {
            self.dragging = Some(id);
        }

        let actions = &mut self.actions;
        let has_parent = node.parent().is_some();
        response.context_menu(|ui| {
            if ui.button("Rename").clicked() {
                actions.push(RowAction::StartRename(id));
                ui.close();
            }
            if ui.button("Frame Object").clicked() {
                actions.push(RowAction::Frame(id));
                ui.close();
            }
            if has_parent && ui.button("Move to Top Level").clicked() {
                actions.push(RowAction::MakeRoot(id));
                ui.close();
            }
            ui.separator();
            if ui.button("Delete").clicked() {
                actions.push(RowAction::Delete(id));
                ui.close();
            }
        });
    }

    /// The empty space below the rows: click to deselect, drop to un-parent.
    fn empty_area(&mut self, ui: &mut Ui, editor: &mut EditorState) {
        let height = ui.available_height().max(metrics::ROW_HEIGHT * 2.0);
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
        if response.clicked() {
            editor.commands.push(EditorCommand::DeselectAll);
        }
        if self.dragging.is_some() && ui.rect_contains_pointer(rect) {
            self.drop_target = Some(DropTarget::Root);
            ui.painter().line_segment(
                [rect.left_top() + vec2(4.0, 1.0), rect.right_top() + vec2(-4.0, 1.0)],
                Stroke::new(2.0, p::ACCENT),
            );
        }
    }

    fn finish_drag(&mut self, ui: &Ui, scene: &Scene, editor: &mut EditorState) {
        let Some(dragged) = self.dragging else { return };

        // Drag label following the cursor, on top of everything.
        if let (Some(pointer), Some(node)) = (ui.ctx().pointer_latest_pos(), scene.get(dragged)) {
            let painter = ui
                .ctx()
                .layer_painter(LayerId::new(Order::Tooltip, Id::unique("oc4d.outliner.drag")));
            let text = painter.layout_no_wrap(node.data.name.clone(), FontId::proportional(12.0), p::TEXT_STRONG);
            let bg = Rect::from_min_size(pointer + vec2(14.0, 6.0), text.size() + vec2(10.0, 4.0));
            painter.rect_filled(bg, CornerRadius::same(3), p::ACCENT_BG);
            painter.galley(bg.min + vec2(5.0, 2.0), text, p::TEXT_STRONG);
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }

        if ui.input(|i| i.pointer.any_released()) {
            let new_parent = match self.drop_target {
                Some(DropTarget::Onto(target)) => Some(Some(target)),
                Some(DropTarget::Root) => Some(None),
                None => None,
            };
            if let Some(new_parent) = new_parent {
                editor.commands.push(EditorCommand::Reparent {
                    node: dragged,
                    new_parent,
                });
            }
            self.dragging = None;
        }
    }

    /// Applies row interactions; document changes are queued as undoable commands.
    fn apply_actions(&mut self, doc: &Document, editor: &mut EditorState) {
        let mut actions = std::mem::take(&mut self.actions);
        for action in actions.drain(..) {
            match action {
                RowAction::Select(id, mode) => editor.commands.push(EditorCommand::Select { id, mode }),
                RowAction::ToggleFold(id) => {
                    if !self.collapsed.remove(&id) {
                        self.collapsed.insert(id);
                    }
                }
                RowAction::ToggleEditorVisibility(id) => {
                    edit_data(doc, editor, id, "Visible in Editor", |d| {
                        d.visible_in_editor = !d.visible_in_editor;
                    });
                }
                RowAction::ToggleRenderVisibility(id) => {
                    edit_data(doc, editor, id, "Visible in Renderer", |d| {
                        d.visible_in_render = !d.visible_in_render;
                    });
                }
                RowAction::ToggleEnabled(id) => edit_data(doc, editor, id, "Enable", |d| d.enabled = !d.enabled),
                RowAction::StartRename(id) => {
                    if let Some(n) = doc.scene.get(id) {
                        self.renaming = Some(id);
                        self.rename_buffer.clear();
                        self.rename_buffer.push_str(&n.data.name);
                        self.rename_needs_focus = true;
                    }
                }
                RowAction::CommitRename(id) => {
                    let name = self.rename_buffer.trim();
                    if !name.is_empty() {
                        edit_data(doc, editor, id, "Rename", |d| name.clone_into(&mut d.name));
                    }
                    self.renaming = None;
                }
                RowAction::CancelRename => self.renaming = None,
                RowAction::Delete(id) => {
                    if !editor.selection.contains(id) {
                        editor.commands.push(EditorCommand::Select {
                            id,
                            mode: SelectMode::Replace,
                        });
                    }
                    editor.commands.push(EditorCommand::DeleteSelected);
                }
                RowAction::MakeRoot(id) => editor.commands.push(EditorCommand::Reparent {
                    node: id,
                    new_parent: None,
                }),
                RowAction::Frame(id) => {
                    editor.commands.push(EditorCommand::Select {
                        id,
                        mode: SelectMode::Replace,
                    });
                    editor.commands.push(EditorCommand::FrameSelected);
                }
            }
        }
        self.actions = actions;
        self.collapsed.retain(|&id| doc.scene.contains(id));
    }
}

/// Queues `change`, applied to a copy of the node's data, as one undo step.
/// Nothing is queued if the node is gone or the change is a no-op.
fn edit_data(
    doc: &Document,
    editor: &mut EditorState,
    id: NodeId,
    label: &'static str,
    change: impl FnOnce(&mut NodeData),
) {
    let Some(node) = doc.scene.get(id) else { return };
    let mut data = node.data.clone();
    change(&mut data);
    if data != node.data {
        editor.commands.push(EditorCommand::Edit {
            edit: SceneEdit::SetData { id, data },
            label,
        });
    }
}

/// Allocation-free, ASCII case-insensitive substring search.
fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    let (h, n) = (haystack.as_bytes(), needle.as_bytes());
    n.is_empty() || h.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n))
}

#[cfg(test)]
mod tests {
    use oc4d_scene::Document;

    use super::{contains_ignore_ascii_case, edit_data};
    use crate::{EditorCommand, EditorState, ViewportPanel, commands};

    #[test]
    fn row_edits_are_undoable_commands() {
        let mut doc = Document::with_demo_scene();
        let mut editor = EditorState::default();
        let mut viewport = ViewportPanel::new(None, None, 1);
        let sphere = doc.scene.roots()[1];

        edit_data(&doc, &mut editor, sphere, "Rename", |d| "Ball".clone_into(&mut d.name));
        edit_data(&doc, &mut editor, sphere, "Enable", |d| d.enabled = true); // already enabled: a no-op
        assert_eq!(
            doc.scene.get(sphere).unwrap().data.name,
            "Sphere",
            "queued, not applied"
        );
        commands::apply_pending(&mut doc, &mut editor, &mut viewport);
        assert_eq!(doc.scene.get(sphere).unwrap().data.name, "Ball");
        assert_eq!(editor.history.undo_len(), 1, "the no-op toggle queued nothing");

        editor.commands.push(EditorCommand::Undo);
        commands::apply_pending(&mut doc, &mut editor, &mut viewport);
        assert_eq!(doc.scene.get(sphere).unwrap().data.name, "Sphere");
    }

    #[test]
    fn search_is_case_insensitive() {
        assert!(contains_ignore_ascii_case("Cylinder.1", "cyl"));
        assert!(contains_ignore_ascii_case("Cube", ""));
        assert!(!contains_ignore_ascii_case("Cube", "sphere"));
    }
}
