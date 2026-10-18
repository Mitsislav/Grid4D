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
    let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::unique("oc4d.drop_overlay")));
    painter.rect_filled(rect, CornerRadius::ZERO, p::SCRIM);
    painter.rect_stroke(
        rect.shrink(12.0),
        CornerRadius::same(6),
        Stroke::new(2.0, p::ACCENT),
        StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(20.0),
        p::TEXT_STRONG,
    );
}

/// What dropping the hovered files would do; `None` when nothing is hovered.
/// Some platforms do not report paths while hovering, hence the `Option`s.
fn hover_label<'a>(paths: impl Iterator<Item = Option<&'a std::path::Path>>) -> Option<String> {
    let mut any = false;
    let mut any_path = false;
    for path in paths {
        any = true;
        let Some(path) = path else { continue };
        any_path = true;
        if file::is_document(path) {
            let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
            return Some(format!("Drop to open {name}"));
        }
    }
    match (any, any_path) {
        (false, _) => None,
        (true, true) => Some("Only OpenC4D documents (.oc4d) can be opened".to_owned()),
        (true, false) => Some("Drop an OpenC4D document (.oc4d) to open it".to_owned()),
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
            Some("Only OpenC4D documents (.oc4d) can be opened")
        );
        assert_eq!(
            hover_label([None].into_iter()).as_deref(),
            Some("Drop an OpenC4D document (.oc4d) to open it")
        );
    }
}
