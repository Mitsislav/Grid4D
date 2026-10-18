//! One module per panel of the Cinema 4D "Standard" layout:
//!
//! ```text
//! ┌──────────────────────────────── menu_bar ─────────────────────────────────┐
//! ├──────────────────────────────── toolbar ──────────────────────────────────┤
//! │mode│                                         │ outliner (Object Manager)  │
//! │pal-│               viewport                  ├────────────────────────────┤
//! │ette│                                         │ attributes (Attribute Mgr) │
//! │    ├──────────────── timeline ───────────────┤                            │
//! ├────┴────────────────────────── status_bar ─────────────────────────────────┤
//! ```

pub mod about;
pub mod alert;
pub mod attributes;
pub mod drop_overlay;
pub mod menu_bar;
pub mod mode_palette;
pub mod outliner;
pub mod status_bar;
pub mod timeline;
pub mod toolbar;
pub mod unsaved_changes;
pub mod viewport;

pub use attributes::AttributesPanel;
pub use outliner::OutlinerPanel;
pub use timeline::TimelinePanel;
pub use viewport::ViewportPanel;

#[cfg(test)]
mod tests {
    use egui::RawInput;
    use oc4d_scene::Document;

    use super::*;
    use crate::{EditorState, commands};

    /// Panels must only *read* the scene unless the user changes something:
    /// `Scene::get_mut` bumps the revision, and an unchanged revision is what lets
    /// the evaluation skip idle frames.
    #[test]
    fn idle_frames_do_not_touch_the_document() {
        let ctx = egui::Context::default();
        let mut doc = Document::with_demo_scene();
        let mut editor = EditorState::default();
        editor.selection.set_single(doc.scene.roots()[0]);
        let mut outliner = OutlinerPanel::default();
        let mut attributes = AttributesPanel::default();
        let mut timeline = TimelinePanel::default();
        let mut viewport = ViewportPanel::new(None, None, 1);

        let mut frame = |doc: &mut Document, editor: &mut EditorState| {
            ctx.run_ui(RawInput::default(), |ui| {
                menu_bar::show(ui, doc, editor);
                toolbar::show(ui, editor);
                outliner.show(ui, doc, editor);
                attributes.show(ui, doc, editor);
                timeline.show(ui, &mut doc.time, editor);
                viewport.show(ui, doc, editor);
            })
            .drop_without_applying_deltas();
            commands::apply_pending(doc, editor, &mut viewport);
        };
        frame(&mut doc, &mut editor);
        frame(&mut doc, &mut editor);
        let revision = doc.scene.revision();
        for _ in 0..5 {
            frame(&mut doc, &mut editor);
        }
        assert_eq!(
            doc.scene.revision(),
            revision,
            "a panel wrote the scene on an idle frame"
        );
    }
}
