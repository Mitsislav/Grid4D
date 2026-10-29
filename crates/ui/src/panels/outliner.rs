//! Object Manager (the "Outliner"): the scene hierarchy as a tree.
//!
//! Rows show fold arrow, type icon, name, the object's tags, the two visibility
//! "traffic light" dots (editor / renderer) and the enabled check mark — like
//! Cinema 4D. Clicking a tag selects it (the Attribute Manager shows it).
//! Double-click renames, dragging a row onto another re-parents it.

use std::collections::HashSet;

use egui::{
    Align2, Button, CornerRadius, FontId, Id, Key, LayerId, Margin, Order, Rect, RichText, ScrollArea, Sense, Stroke,
    StrokeKind, TextEdit, TextStyle, Ui, Vec2, pos2, vec2,
};
use grid4d_scene::{Body, Document, Node, NodeData, NodeId, ObjectKindTag, Scene, SceneEdit, Tag, TagKind};

use crate::{
    EditorCommand, EditorState, SelectMode, TagRef,
    commands::tag_color,
    icons::{self, Icon},
    theme::{metrics, palette as p},
    widgets::manager_tabs,
};

const INDENT: f32 = 16.0;
/// Horizontal distance between tag icons.
const TAG_STEP: f32 = 19.0;

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
    /// A click on a tag icon: select the tag (and its object).
    SelectTag(NodeId, TagKind),
    DeleteTag(NodeId, TagKind),
    /// Context menu: add a tag to the (just selected) object.
    /// An entry of [`tag_menu`], by index.
    AddTag(usize),
    /// A material dropped on the object.
    AssignMaterial(NodeId, grid4d_scene::MaterialId),
    /// A double-click on a Material tag: show its material.
    ShowMaterial(grid4d_scene::MaterialId),
    /// A double-click on a camera's icon: look through it (again: stop).
    LookThrough(NodeId),
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
                .id_salt("grid4d.outliner.scroll")
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
                    let any = !editor.selection.is_empty();
                    for (label, tag) in tag_menu() {
                        if ui
                            .add_enabled(any, Button::new(label))
                            .on_disabled_hover_text("Select the objects to tag first")
                            .clicked()
                        {
                            editor.commands.push(EditorCommand::AddTag(tag));
                            ui.close();
                        }
                    }
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

        // A material from the Material Manager: dropped on this row assigns it.
        if let Some(drag) = editor.materials.drag
            && ui.rect_contains_pointer(rect)
        {
            painter.rect_stroke(
                rect.shrink(1.0),
                CornerRadius::same(2),
                Stroke::new(1.5, p::HIGHLIGHT),
                StrokeKind::Inside,
            );
            if ui.input(|i| i.pointer.any_released()) {
                self.actions.push(RowAction::AssignMaterial(id, drag.material));
            }
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

        // Type icon; a camera's links the view on a double-click.
        let tag = node.data.kind.tag();
        let icon_rect = Rect::from_min_size(pos2(x, cy - 8.0), Vec2::splat(16.0));
        if tag == ObjectKindTag::Camera {
            let linked = editor.view_camera == Some(id);
            if linked {
                painter.rect_filled(icon_rect.expand(2.0), CornerRadius::same(3), p::ACCENT);
            }
            let hit = ui
                .interact(icon_rect, ui.scope_id().with(("camera", id)), Sense::click())
                .on_hover_text(if linked {
                    "The view looks through this camera · double-click: back to the default camera"
                } else {
                    "Double-click: look through this camera"
                });
            if hit.double_clicked() {
                self.actions.push(RowAction::LookThrough(id));
            } else if hit.clicked() {
                let mode = SelectMode::from_modifiers(ui.input(|i| i.modifiers));
                self.actions.push(RowAction::Select(id, mode));
            }
        }
        icons::paint(&painter, icon_rect, Icon::Object(tag), tag_color(tag));
        x += 22.0;

        // Tags: icons right of the name, before the visibility dots.
        let tags_right = rect.right() - 44.0;
        let tags_left = tags_right - node.data.tags.len() as f32 * TAG_STEP;
        let selected_tag = editor.tag_selection.filter(|t| t.node == id).map(|t| t.kind);
        for (k, tag) in node.data.tags.iter().enumerate() {
            let kind = tag.kind();
            let icon_rect = Rect::from_min_size(pos2(tags_left + k as f32 * TAG_STEP, cy - 8.0), Vec2::splat(16.0));
            let hit = ui.interact(icon_rect, ui.scope_id().with(("tag", id, k)), Sense::click());
            if selected_tag == Some(kind) {
                painter.rect_stroke(
                    icon_rect.expand(1.5),
                    CornerRadius::same(2),
                    Stroke::new(1.5, p::HIGHLIGHT),
                    StrokeKind::Inside,
                );
            }
            let color = if hit.hovered() { p::TEXT_STRONG } else { p::TAG };
            // A Material tag shows its material's colour.
            let material = match tag {
                Tag::Material { material } => material.and_then(|m| scene.materials().get(m).map(|mat| (m, mat))),
                Tag::Phong { .. } | Tag::Protection { .. } | Tag::Target { .. } | Tag::Dynamics(_) => None,
            };
            match material {
                Some((_, mat)) => icons::paint_material_ball(&painter, icon_rect.shrink(2.0), mat.color),
                None => icons::paint(&painter, icon_rect.shrink(1.0), Icon::Tag(kind), color),
            }
            let tip = match material {
                Some((_, mat)) => format!("Material Tag: {} · double-click shows the material", mat.name),
                None => format!("{} Tag", kind.label()),
            };
            let hit = hit.on_hover_text(tip);
            if hit.double_clicked()
                && let Some((m, _)) = material
            {
                self.actions.push(RowAction::ShowMaterial(m));
            } else if hit.clicked() {
                self.actions.push(RowAction::SelectTag(id, kind));
            }
            let actions = &mut self.actions;
            hit.context_menu(|ui| {
                if ui.button("Delete Tag").clicked() {
                    actions.push(RowAction::DeleteTag(id, kind));
                    ui.close();
                }
            });
        }

        // Name (or the inline rename editor).
        let name_rect = Rect::from_min_max(
            pos2(x, rect.top() + 1.0),
            pos2((tags_left - 4.0).max(x + 20.0), rect.bottom() - 1.0),
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
            ui.menu_button("Add Tag", |ui| {
                for (index, (label, _)) in tag_menu().enumerate() {
                    if ui.button(label).clicked() {
                        actions.push(RowAction::Select(id, SelectMode::Replace));
                        actions.push(RowAction::AddTag(index));
                        ui.close();
                    }
                }
            });
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
                .layer_painter(LayerId::new(Order::Tooltip, Id::unique("grid4d.outliner.drag")));
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
                RowAction::Select(id, mode) => {
                    editor.tag_selection = None;
                    editor.commands.push(EditorCommand::Select { id, mode });
                }
                RowAction::SelectTag(id, kind) => {
                    if editor.selection.active() != Some(id) {
                        editor.commands.push(EditorCommand::Select {
                            id,
                            mode: SelectMode::Replace,
                        });
                    }
                    editor.tag_selection = Some(TagRef { node: id, kind });
                }
                RowAction::DeleteTag(node, kind) => editor.commands.push(EditorCommand::DeleteTag { node, kind }),
                RowAction::AddTag(index) => {
                    if let Some((_, tag)) = tag_menu().nth(index) {
                        editor.commands.push(EditorCommand::AddTag(tag));
                    }
                }
                RowAction::AssignMaterial(node, material) => editor.commands.push(EditorCommand::AssignMaterial {
                    material,
                    nodes: vec![node],
                }),
                RowAction::LookThrough(id) => {
                    editor.view_camera = if editor.view_camera == Some(id) { None } else { Some(id) };
                }
                RowAction::ShowMaterial(material) => {
                    editor.materials.selected = Some(material);
                    editor.materials.shown = true;
                    editor.managers = crate::ManagerTab::Attributes;
                }
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

/// The Tags menus' entries: every kind with its defaults, and the Dynamics
/// tag twice — as a Rigid Body and as a Collider Body.
fn tag_menu() -> impl Iterator<Item = (String, Tag)> {
    TagKind::ALL
        .into_iter()
        .map(|kind| (format!("{} Tag", kind.label()), kind.default_tag()))
        .chain(std::iter::once((
            "Collider Body Tag".to_owned(),
            Tag::Dynamics(Body::collider()),
        )))
}

#[cfg(test)]
mod tests {
    use grid4d_scene::Document;

    use super::{contains_ignore_ascii_case, edit_data};
    use crate::{EditorCommand, EditorState, ViewportPanel, commands};

    #[test]
    fn row_edits_are_undoable_commands() {
        let mut doc = Document::with_demo_scene();
        let mut editor = EditorState::default();
        let mut viewport = ViewportPanel::new(None);
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
    fn double_clicking_a_row_starts_renaming_it() {
        use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2, vec2};

        let mut doc = Document::with_demo_scene();
        let mut editor = EditorState::default();
        let mut panel = super::OutlinerPanel::default();
        let mut viewport = ViewportPanel::new(None);
        let sphere = doc.scene.roots()[1];
        let ctx = egui::Context::default();
        let mut time = 0.0;
        let mut frame =
            |events: Vec<Event>, doc: &mut Document, editor: &mut EditorState, panel: &mut super::OutlinerPanel| {
                time += 1.0 / 60.0;
                let input = RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 500.0))),
                    time: Some(time),
                    events,
                    ..RawInput::default()
                };
                ctx.run_ui(input, |ui| {
                    egui::CentralPanel::no_frame().show(ui, |ui| panel.show(ui, doc, editor));
                })
                .drop_without_applying_deltas();
                commands::apply_pending(doc, editor, &mut viewport);
            };
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame(vec![], &mut doc, &mut editor, &mut panel);
        // Find the Sphere's row by clicking down the list (below the tabs and the menu).
        let row = (0..100)
            .map(|i| pos2(120.0, 60.0 + 4.0 * f32::from(u8::try_from(i).unwrap())))
            .find(|&at| {
                frame(vec![Event::PointerMoved(at)], &mut doc, &mut editor, &mut panel);
                frame(vec![button(at, true)], &mut doc, &mut editor, &mut panel);
                frame(vec![button(at, false)], &mut doc, &mut editor, &mut panel);
                for _ in 0..30 {
                    frame(vec![], &mut doc, &mut editor, &mut panel); // past the double-click delay
                }
                editor.selection.active() == Some(sphere)
            })
            .expect("a row selects the Sphere");
        assert_eq!(panel.renaming, None);
        for _ in 0..2 {
            frame(
                vec![Event::PointerMoved(row), button(row, true)],
                &mut doc,
                &mut editor,
                &mut panel,
            );
            frame(vec![button(row, false)], &mut doc, &mut editor, &mut panel);
        }
        frame(vec![], &mut doc, &mut editor, &mut panel);
        assert_eq!(panel.renaming, Some(sphere));
        assert_eq!(panel.rename_buffer, "Sphere");
    }

    #[test]
    fn a_material_dropped_on_a_row_is_assigned_to_its_object() {
        use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2, vec2};

        let mut doc = Document::with_demo_scene();
        let material = doc.scene.materials_mut().insert(grid4d_scene::Material::new("Mat"));
        let mut editor = EditorState::default();
        let mut panel = super::OutlinerPanel::default();
        let mut viewport = ViewportPanel::new(None);
        let ctx = egui::Context::default();
        let mut time = 0.0;
        let mut frame = |events: Vec<Event>, doc: &mut Document, editor: &mut EditorState| {
            time += 1.0 / 60.0;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 500.0))),
                time: Some(time),
                events,
                ..RawInput::default()
            };
            ctx.run_ui(input, |ui| {
                egui::CentralPanel::no_frame().show(ui, |ui| panel.show(ui, doc, editor));
            })
            .drop_without_applying_deltas();
            commands::apply_pending(doc, editor, &mut viewport);
        };
        frame(vec![], &mut doc, &mut editor);
        let release = |pos| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        };
        // Drop down the list until an object gets the material.
        let mut got = None;
        for i in 0..60u8 {
            let at = pos2(120.0, 60.0 + 4.0 * f32::from(i));
            editor.materials.drag = Some(crate::MaterialDrag { material });
            frame(vec![Event::PointerMoved(at)], &mut doc, &mut editor);
            frame(vec![release(at)], &mut doc, &mut editor);
            got = doc
                .scene
                .iter()
                .find(|(_, n)| grid4d_scene::Tags::material(n.data.tags.as_slice()) == Some(material))
                .map(|(id, _)| id);
            if got.is_some() {
                break;
            }
        }
        let id = got.expect("a row takes the material");
        assert_eq!(doc.scene.get(id).unwrap().data.name, "Cube", "the first row");
        assert_eq!(editor.history.undo_len(), 1);
    }

    #[test]
    fn clicking_a_tag_icon_selects_the_tag() {
        use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2, vec2};

        let mut doc = Document::with_demo_scene();
        let sphere = doc.scene.roots()[1];
        doc.scene.get_mut(sphere).unwrap().data.tags = vec![grid4d_scene::TagKind::Phong.default_tag()];
        let mut editor = EditorState::default();
        let mut panel = super::OutlinerPanel::default();
        let mut viewport = ViewportPanel::new(None);
        let ctx = egui::Context::default();
        let mut time = 0.0;
        let mut click = |at: Pos2, doc: &mut Document, editor: &mut EditorState| {
            let button = |pressed| Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            for events in [
                vec![Event::PointerMoved(at)],
                vec![button(true)],
                vec![button(false)],
                vec![],
            ] {
                time += 0.5; // never a double click
                let input = RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 500.0))),
                    time: Some(time),
                    events,
                    ..RawInput::default()
                };
                ctx.run_ui(input, |ui| {
                    egui::CentralPanel::no_frame().show(ui, |ui| panel.show(ui, doc, editor));
                })
                .drop_without_applying_deltas();
                commands::apply_pending(doc, editor, &mut viewport);
            }
        };
        let row_y = (0..100)
            .map(|i| 60.0 + 4.0 * f32::from(u8::try_from(i).unwrap()))
            .find(|&y| {
                click(pos2(120.0, y), &mut doc, &mut editor);
                editor.selection.active() == Some(sphere)
            })
            .expect("the Sphere's row");
        assert_eq!(editor.tag_selection, None);
        // The icon sits in the middle of the row, right of the name.
        let hit = (0..6)
            .flat_map(|dy: u8| (0..40u8).map(move |i| pos2(400.0 - 3.0 * f32::from(i), row_y + 4.0 * f32::from(dy))))
            .find(|&at| {
                click(at, &mut doc, &mut editor);
                editor.tag_selection.is_some()
            });
        assert!(hit.is_some(), "a click on the tag icon selects it");
        let (tag, index) = editor.selected_tag(&doc).unwrap();
        assert_eq!((tag.node, tag.kind, index), (sphere, grid4d_scene::TagKind::Phong, 0));
        // Clicking the object's name shows the object again.
        click(pos2(120.0, row_y), &mut doc, &mut editor);
        assert_eq!(editor.tag_selection, None);
    }

    #[test]
    fn search_is_case_insensitive() {
        assert!(contains_ignore_ascii_case("Cylinder.1", "cyl"));
        assert!(contains_ignore_ascii_case("Cube", ""));
        assert!(!contains_ignore_ascii_case("Cube", "sphere"));
    }
}
