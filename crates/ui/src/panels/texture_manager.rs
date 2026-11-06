//! The Texture Manager (see `docs/reference/material-textures.md`): every
//! picture the document uses, with a thumbnail, its size, where it comes from
//! and whether it was found. A tab beside the Material Manager.
//!
//! The panel only reads the document and the texture cache; every change is a
//! command (one undo step each): Add…, Relink…, Relink Folder…, Embed,
//! Unembed…, Reveal, Delete, Remove Unused, and the selected texture's name
//! and colour space.

use std::collections::HashMap;

use egui::{
    Align2, ColorImage, ComboBox, CornerRadius, FontId, Rect, RichText, ScrollArea, Sense, Stroke, StrokeKind,
    TextEdit, TextureHandle, TextureOptions, Ui, WidgetInfo, accesskit::Role, pos2, vec2,
};
use grid4d_scene::{ColorSpace, Document, SceneEdit, Texture, TextureId, TextureSource};

use crate::i18n::{tr, trf};
use crate::{EditorCommand, EditorState, textures::Status, theme::palette as p};

/// A row's height and its thumbnail's side.
const ROW: f32 = 44.0;
const THUMB: f32 = 38.0;

#[derive(Default)]
pub struct TextureManagerPanel {
    /// Each texture's thumbnail and the picture version it shows.
    thumbnails: HashMap<TextureId, (u64, TextureHandle)>,
    /// The name being typed for the selected texture.
    name: Option<(TextureId, String)>,
}

impl std::fmt::Debug for TextureManagerPanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextureManagerPanel")
            .field("thumbnails", &self.thumbnails.len())
            .finish_non_exhaustive()
    }
}

/// The colour spaces offered, with their labels.
const SPACES: [(ColorSpace, &str); 2] = [(ColorSpace::Srgb, "sRGB"), (ColorSpace::Linear, "Linear")];

impl TextureManagerPanel {
    pub fn show(&mut self, ui: &mut Ui, doc: &mut Document, editor: &mut EditorState) {
        let textures = doc.scene.textures();
        if editor.texture_selection.is_some_and(|id| !textures.contains(id)) {
            editor.texture_selection = None;
        }
        self.thumbnails.retain(|id, _| textures.contains(*id));
        let selected = editor
            .texture_selection
            .and_then(|id| textures.get(id).map(|t| (id, t)));
        toolbar(ui, editor, selected);
        ui.separator();

        let details = if selected.is_some() { 40.0 } else { 0.0 };
        let list_height = (ui.available_height() - details).max(ROW);
        ScrollArea::vertical()
            .id_salt("grid4d.textures.scroll")
            .auto_shrink([false; 2])
            .max_height(list_height)
            .show(ui, |ui| {
                if textures.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(tr(
                            "No textures yet. Add… chooses pictures; you can also drop picture files on the window.",
                        ))
                        .color(p().text_dim),
                    );
                    return;
                }
                for (id, texture) in textures.iter() {
                    self.row(ui, editor, id, texture);
                }
            });
        if let Some((id, texture)) = selected {
            self.details(ui, editor, doc, id, texture);
        }
    }

    fn row(&mut self, ui: &mut Ui, editor: &mut EditorState, id: TextureId, texture: &Texture) {
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
        let is_selected = editor.texture_selection == Some(id);
        response.widget_info(|| WidgetInfo::selected(Role::ListItem, true, is_selected, &texture.name));
        if response.clicked() || response.secondary_clicked() {
            editor.texture_selection = Some(id);
        }
        let painter = ui.painter_at(rect);
        if is_selected {
            painter.rect_filled(rect, CornerRadius::same(2), p().accent_bg);
        } else if response.hovered() {
            painter.rect_filled(rect, CornerRadius::same(2), p().bg_hover);
        }

        // The thumbnail: the picture, a checker when it is missing, nothing
        // while it loads.
        let thumb = Rect::from_min_size(
            pos2(rect.left() + 4.0, rect.center().y - THUMB / 2.0),
            vec2(THUMB, THUMB),
        );
        let status = editor.textures.status(id, texture);
        match status {
            Status::Ready { image, version } => {
                let handle = match self.thumbnails.get(&id) {
                    Some((shown, handle)) if *shown == version => handle.clone(),
                    _ => {
                        let handle = thumbnail(ui.ctx(), id, image, texture.color_space);
                        self.thumbnails.insert(id, (version, handle.clone()));
                        handle
                    }
                };
                let size = handle.size_vec2();
                let fit = (THUMB / size.x.max(size.y).max(1.0)) * size;
                let shown = Rect::from_center_size(thumb.center(), fit);
                painter.rect_filled(thumb, CornerRadius::ZERO, p().bg_well);
                painter.image(
                    handle.id(),
                    shown,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
            Status::Missing | Status::Failed(_) => checker(&painter, thumb),
            Status::Loading => {
                painter.rect_filled(thumb, CornerRadius::ZERO, p().bg_well);
            }
        }
        painter.rect_stroke(
            thumb,
            CornerRadius::ZERO,
            Stroke::new(1.0, p().border),
            StrokeKind::Inside,
        );

        // Name, then what it is and where it comes from.
        let left = thumb.right() + 8.0;
        let space = SPACES
            .iter()
            .find(|(s, _)| *s == texture.color_space)
            .map_or("", |(_, label)| *label);
        painter.text(
            pos2(rect.right() - 6.0, rect.top() + 12.0),
            Align2::RIGHT_CENTER,
            tr(space),
            FontId::proportional(11.0),
            p().text_dim,
        );
        painter.text(
            pos2(left, rect.top() + 12.0),
            Align2::LEFT_CENTER,
            &texture.name,
            FontId::proportional(13.0),
            p().text_strong,
        );
        let folder = editor.file.path().and_then(std::path::Path::parent);
        let source = match &texture.source {
            TextureSource::Linked(path) => shown_path(path, folder),
            TextureSource::Embedded { file_name, .. } => trf("Embedded ({0})", &[file_name]),
        };
        let (line, colour) = match status {
            Status::Ready { image, .. } => {
                let (w, h) = image.size();
                (format!("{w} × {h}  ·  {source}"), p().text_dim)
            }
            Status::Loading => (trf("Loading…  ·  {0}", &[&source]), p().text_dim),
            Status::Missing => (trf("Missing: {0}", &[&source]), p().warning),
            Status::Failed(message) => (trf("Cannot be read: {0}", &[&message]), p().warning),
        };
        painter.text(
            pos2(left, rect.top() + 30.0),
            Align2::LEFT_CENTER,
            line,
            FontId::proportional(11.0),
            colour,
        );

        response.context_menu(|ui| {
            if let Some(command) = texture_actions(ui, id, texture) {
                editor.commands.push(command);
                ui.close();
            }
        });
    }

    /// The selected texture's name and colour space.
    fn details(&mut self, ui: &mut Ui, editor: &mut EditorState, doc: &Document, id: TextureId, texture: &Texture) {
        ui.separator();
        ui.horizontal(|ui| {
            let label = ui.label(tr("Name"));
            let name = match &mut self.name {
                Some((editing, name)) if *editing == id => name,
                slot => &mut slot.insert((id, texture.name.clone())).1,
            };
            let edit = ui
                .add(TextEdit::singleline(name).desired_width(160.0))
                .labelled_by(label.id);
            if edit.lost_focus() && *name != texture.name && !name.trim().is_empty() {
                let renamed = name.trim().to_owned();
                push_change(doc, editor, id, "Rename Texture", |t| t.name = renamed);
            }
            if !edit.has_focus() && *name != texture.name && !edit.lost_focus() {
                // Changed elsewhere (undo): show the document's name.
                self.name = None;
            }
            ui.add_space(12.0);
            ui.label(tr("Colour Space"));
            let current = texture.color_space;
            let mut chosen = current;
            let label = SPACES.iter().find(|(s, _)| *s == current).map_or("", |(_, l)| *l);
            let combo = ComboBox::from_id_salt("grid4d.textures.color_space")
                .selected_text(tr(label))
                .show_ui(ui, |ui| {
                    for (space, label) in SPACES {
                        ui.selectable_value(&mut chosen, space, tr(label));
                    }
                })
                .response;
            ui.ctx()
                .accesskit_node_builder(combo.id, |node| node.set_label(tr("Colour Space")));
            if chosen != current {
                push_change(doc, editor, id, "Texture Colour Space", |t| t.color_space = chosen);
            }
        });
    }
}

/// The buttons over the list; those for one texture act on the selected one.
fn toolbar(ui: &mut Ui, editor: &mut EditorState, selected: Option<(TextureId, &Texture)>) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if ui
            .button(tr("Add…"))
            .on_hover_text(tr("Choose pictures to use as textures"))
            .clicked()
        {
            editor.commands.push(EditorCommand::AddTextures);
        }
        if ui
            .button(tr("Relink Folder…"))
            .on_hover_text(tr("Look for every missing picture by its name in a folder"))
            .clicked()
        {
            editor.commands.push(EditorCommand::RelinkTextureFolder);
        }
        if ui
            .button(tr("Remove Unused"))
            .on_hover_text(tr("Remove the textures no material uses"))
            .clicked()
        {
            editor.commands.push(EditorCommand::RemoveUnusedTextures);
        }
        ui.add_space(8.0);
        ui.add_enabled_ui(selected.is_some(), |ui| match selected {
            Some((id, texture)) => {
                if let Some(command) = texture_actions(ui, id, texture) {
                    editor.commands.push(command);
                }
            }
            None => {
                let _ = ui.button(tr("Relink…"));
                let _ = ui.button(tr("Embed"));
                let _ = ui.button(tr("Reveal"));
            }
        });
    });
}

/// Relink…, Embed or Unembed…, Reveal and Delete for one texture (the
/// toolbar's and a row's context menu): the command clicked, if any.
fn texture_actions(ui: &mut Ui, id: TextureId, texture: &Texture) -> Option<EditorCommand> {
    let mut chosen = None;
    let mut push = |command| chosen = Some(command);
    if ui
        .button(tr("Relink…"))
        .on_hover_text(tr("Choose another picture for this texture"))
        .clicked()
    {
        push(EditorCommand::RelinkTexture(id));
    }
    match &texture.source {
        TextureSource::Linked(_) => {
            if ui
                .button(tr("Embed"))
                .on_hover_text(tr("Keep the picture inside the document"))
                .clicked()
            {
                push(EditorCommand::EmbedTexture(id));
            }
            if ui
                .button(tr("Reveal"))
                .on_hover_text(tr("Show the picture's file in the file browser"))
                .clicked()
            {
                push(EditorCommand::RevealTexture(id));
            }
        }
        TextureSource::Embedded { .. } => {
            if ui
                .button(tr("Unembed…"))
                .on_hover_text(tr("Write the picture to a file and link it"))
                .clicked()
            {
                push(EditorCommand::UnembedTexture(id));
            }
        }
    }
    if ui
        .button(tr("Delete"))
        .on_hover_text(tr("Remove this texture"))
        .clicked()
    {
        push(EditorCommand::DeleteTexture(id));
    }
    chosen
}

/// Pushes the change of one texture's settings as one undo step.
fn push_change(
    doc: &Document,
    editor: &mut EditorState,
    id: TextureId,
    label: &'static str,
    change: impl FnOnce(&mut Texture),
) {
    let mut textures = doc.scene.textures().clone();
    if let Some(texture) = textures.get_mut(id) {
        change(texture);
        editor.commands.push(EditorCommand::Edit {
            edit: SceneEdit::SetTextures(textures),
            label,
        });
    }
}

/// The picture's level of about the thumbnail's size, as an egui texture.
fn thumbnail(ctx: &egui::Context, id: TextureId, image: &grid4d_io::TextureImage, space: ColorSpace) -> TextureHandle {
    // Twice the thumbnail's side, for sharp thumbnails on high-density screens.
    let side = 96;
    let picture = match image.level_within(side) {
        Some(level) => ColorImage::from_rgba_unmultiplied(
            [
                usize::try_from(level.width).unwrap_or(1),
                usize::try_from(level.height).unwrap_or(1),
            ],
            &level.to_srgb8(space),
        ),
        None => ColorImage::filled([1, 1], egui::Color32::BLACK),
    };
    ctx.load_texture(format!("grid4d.texture.{id:?}"), picture, TextureOptions::LINEAR)
}

/// `path` as the manager shows it: relative to the document's folder (with
/// `/`) when it lies inside it, else in full.
fn shown_path(path: &std::path::Path, folder: Option<&std::path::Path>) -> String {
    match folder.and_then(|folder| path.strip_prefix(folder).ok()) {
        Some(inside) => inside
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
        None => path.display().to_string(),
    }
}

/// The magenta and black checker a missing picture shows.
fn checker(painter: &egui::Painter, rect: Rect) {
    let cells = 4;
    let size = rect.width() / f32::from(u8::try_from(cells).unwrap_or(4));
    for y in 0..cells {
        for x in 0..cells {
            let colour = if (x + y) % 2 == 0 {
                egui::Color32::from_rgb(255, 0, 255)
            } else {
                egui::Color32::BLACK
            };
            let min = rect.min
                + vec2(
                    f32::from(u8::try_from(x).unwrap_or(0)),
                    f32::from(u8::try_from(y).unwrap_or(0)),
                ) * size;
            painter.rect_filled(Rect::from_min_size(min, vec2(size, size)), CornerRadius::ZERO, colour);
        }
    }
}
