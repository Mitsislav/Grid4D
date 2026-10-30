//! Feedback while files are dragged over the window: darkens the workspace and
//! says what dropping will do. The drop itself is handled by
//! `file::accept_dropped_files`.

use egui::{Align2, CornerRadius, FontId, Id, LayerId, Order, Stroke, StrokeKind};

use crate::{file, theme::palette as p};

pub fn show(ctx: &egui::Context) {
    let Some(label) = ctx.input(|i| hover_label(i.raw.hovered_files.iter().map(|f| f.path.as_deref()))) else {
        return;
    };
    let rect = ctx.content_rect();
    let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::unique("grid4d.drop_overlay")));
    painter.rect_filled(rect, CornerRadius::ZERO, p().scrim);
    painter.rect_stroke(
        rect.shrink(12.0),
        CornerRadius::same(6),
        Stroke::new(2.0, p().accent),
        StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(20.0),
        p().text_strong,
    );
}

/// What dropping the hovered files would do; `None` when nothing is hovered.
/// Some platforms do not report paths while hovering, hence the `Option`s.
fn hover_label<'a>(paths: impl Iterator<Item = Option<&'a std::path::Path>>) -> Option<String> {
    let mut any = false;
    let mut any_path = false;
    // A document wins over a model anywhere in the same drop.
    let mut merge: Option<String> = None;
    for path in paths {
        any = true;
        let Some(path) = path else { continue };
        any_path = true;
        let name = || path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
        if file::is_document(path) {
            return Some(format!("Drop to open {}", name()));
        }
        if merge.is_none() && grid4d_io::is_importable(path) {
            merge = Some(format!("Drop to merge {}", name()));
        }
    }
    match (any, any_path, merge) {
        (false, _, _) => None,
        (true, true, Some(merge)) => Some(merge),
        (true, true, None) => {
            Some("Only Grid4D documents (.g4d) and models (.gltf, .glb, .obj) can be dropped".to_owned())
        }
        (true, false, _) => Some("Drop a .g4d document to open it, or a model to merge it".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::hover_label;

    #[test]
    fn the_label_says_what_a_drop_will_do() {
        assert_eq!(hover_label(std::iter::empty()), None);
        assert_eq!(
            hover_label([Some(Path::new("notes.txt")), Some(Path::new("shots/a.OC4D"))].into_iter()).as_deref(),
            Some("Drop to open a.OC4D")
        );
        assert_eq!(
            hover_label([Some(Path::new("notes.txt"))].into_iter()).as_deref(),
            Some("Only Grid4D documents (.g4d) and models (.gltf, .glb, .obj) can be dropped")
        );
        assert_eq!(
            hover_label([None].into_iter()).as_deref(),
            Some("Drop a .g4d document to open it, or a model to merge it")
        );
        assert_eq!(
            hover_label([Some(Path::new("duck.glb")), Some(Path::new("shot.g4d"))].into_iter()).as_deref(),
            Some("Drop to open shot.g4d"),
            "a document wins"
        );
        assert_eq!(
            hover_label([Some(Path::new("notes.txt")), Some(Path::new("duck.GLB"))].into_iter()).as_deref(),
            Some("Drop to merge duck.GLB")
        );
    }
}
