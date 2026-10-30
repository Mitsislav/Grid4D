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
pub mod coordinates;
pub mod drop_overlay;
pub mod fcurve;
pub mod layers;
pub mod materials;
pub mod menu_bar;
pub mod mode_palette;
pub mod optimize;
pub mod outliner;
pub mod picture_viewer;
pub mod plugin_command;
pub mod preferences;
pub mod recovery;
pub mod render_settings;
pub mod status_bar;
pub mod takes;
pub mod timeline;
pub mod toolbar;
pub mod unsaved_changes;
pub mod viewport;

pub use attributes::AttributesPanel;
pub use coordinates::CoordinatesPanel;
pub use fcurve::FCurvePanel;
pub use layers::LayersPanel;
pub use materials::MaterialsPanel;
pub use outliner::OutlinerPanel;
pub use picture_viewer::PictureViewerPanel;
pub use render_settings::RenderSettingsPanel;
pub use takes::TakesPanel;
pub use timeline::TimelinePanel;
pub use viewport::ViewportPanel;

#[cfg(test)]
mod tests {
    use egui::RawInput;
    use grid4d_scene::Document;

    use super::*;
    use crate::{EditorState, commands};

    /// Panels must only *read* the scene unless the user changes something:
    /// `Scene::get_mut` bumps the revision, and an unchanged revision is what lets
    /// the evaluation skip idle frames.
    #[test]
    fn idle_frames_do_not_touch_the_document() {
        let doc = Document::with_demo_scene();
        let mut editor = EditorState::default();
        editor.selection.set_single(doc.scene.roots()[0]);
        assert_idle_frames_leave_the_document_alone(doc, editor);
    }

    #[test]
    fn idle_frames_in_a_component_mode_do_not_touch_the_document() {
        let mut doc = Document::with_demo_scene();
        let mut editor = EditorState::default();
        editor.selection.set_single(doc.scene.roots()[0]);
        editor.commands.push(crate::EditorCommand::MakeEditable);
        commands::apply_pending(&mut doc, &mut editor, &mut ViewportPanel::new(None));
        editor.mode = crate::EditMode::Points;
        assert_idle_frames_leave_the_document_alone(doc, editor);
    }

    /// Four views draw without writing anything either.
    #[test]
    fn idle_frames_with_four_views_do_not_touch_the_document() {
        let doc = Document::with_demo_scene();
        let mut editor = EditorState::default();
        editor.selection.set_single(doc.scene.roots()[0]);
        editor.commands.push(crate::EditorCommand::ShowView(None));
        assert_idle_frames_leave_the_document_alone(doc, editor);
    }

    /// Splines draw without writing anything.
    #[test]
    fn idle_frames_with_splines_do_not_touch_the_document() {
        let mut doc = Document::with_demo_scene();
        for tag in grid4d_scene::ObjectKindTag::SPLINES {
            doc.scene
                .insert(grid4d_scene::Node::new(tag.label(), tag.default_kind()));
        }
        let mut editor = EditorState::default();
        editor.selection.set_single(*doc.scene.roots().last().unwrap());
        assert_idle_frames_leave_the_document_alone(doc, editor);
    }

    /// Looking through a scene camera writes nothing while nothing moves.
    #[test]
    fn idle_frames_through_a_scene_camera_do_not_touch_the_document() {
        let doc = Document::with_demo_scene();
        let mut editor = EditorState::default();
        let camera = doc.scene.iter().find(|(_, n)| n.data.name == "Camera").unwrap().0;
        editor.view_camera = Some(camera);
        editor.selection.set_single(camera);
        assert_idle_frames_leave_the_document_alone(doc, editor);
    }

    /// The Render Settings window draws without writing anything.
    #[test]
    fn idle_frames_with_the_render_settings_do_not_touch_the_document() {
        let doc = Document::with_demo_scene();
        let editor = EditorState {
            show_render_settings: true,
            ..EditorState::default()
        };
        assert_idle_frames_leave_the_document_alone(doc, editor);
    }

    /// The Picture Viewer shows a finished render without writing anything.
    #[test]
    fn idle_frames_with_the_picture_viewer_do_not_touch_the_document() {
        let mut doc = Document::with_demo_scene();
        crate::rendering::tests::small(&mut doc, 2);
        let mut editor = EditorState::default();
        editor
            .renders
            .start(&doc, &grid4d_render::OrbitCamera::default(), None, None, None);
        crate::rendering::tests::wait(&mut editor.renders);
        editor.renders.viewer_open = true;
        assert_idle_frames_leave_the_document_alone(doc, editor);
    }

    /// The Attribute Manager and the gizmo show animated values between keys.
    #[test]
    fn idle_frames_between_keys_do_not_touch_the_document() {
        let mut doc = Document::with_demo_scene();
        let mut editor = EditorState::default();
        let cube = doc.scene.roots()[0];
        editor.selection.set_single(cube);
        let data = &mut doc.scene.get_mut(cube).unwrap().data;
        data.record(grid4d_scene::Param::TRANSFORM, 0.0, 0.0);
        let mut shown = data.transform_at(10.0);
        shown.position.x = 123.4;
        shown.rotation.y = 0.7;
        data.set_transform_at(&shown, 10.0, true);
        doc.time.set_frame(7);
        assert_idle_frames_leave_the_document_alone(doc, editor);
    }

    fn assert_idle_frames_leave_the_document_alone(mut doc: Document, mut editor: EditorState) {
        let ctx = egui::Context::default();
        let mut outliner = OutlinerPanel::default();
        let mut attributes = AttributesPanel::default();
        let mut timeline = TimelinePanel::default();
        let mut fcurve = FCurvePanel::default();
        let mut coordinates = CoordinatesPanel::default();
        let mut materials = MaterialsPanel;
        let mut render_settings = RenderSettingsPanel::default();
        let mut picture_viewer = PictureViewerPanel::default();
        let mut viewport = ViewportPanel::new(None);

        let mut frame = |doc: &mut Document, editor: &mut EditorState| {
            ctx.run_ui(RawInput::default(), |ui| {
                menu_bar::show(ui, doc, editor);
                toolbar::show(ui, editor);
                mode_palette::show(ui, doc, editor);
                outliner.show(ui, doc, editor);
                attributes.show(ui, doc, editor);
                timeline.show(ui, doc, editor);
                fcurve.show(ui, doc, editor);
                coordinates.show(ui, doc, editor);
                materials.show(ui, doc, editor);
                viewport.show(ui, doc, editor);
                render_settings.show(ui.ctx(), doc, editor);
                picture_viewer.show(ui.ctx(), editor);
            })
            .drop_without_applying_deltas();
            commands::apply_pending(doc, editor, &mut viewport);
        };
        frame(&mut doc, &mut editor);
        frame(&mut doc, &mut editor);
        let revision = doc.scene.revision();
        let (render, steps) = (doc.render.clone(), editor.history.undo_len());
        for _ in 0..5 {
            frame(&mut doc, &mut editor);
        }
        assert_eq!(
            doc.scene.revision(),
            revision,
            "a panel wrote the scene on an idle frame"
        );
        assert_eq!(doc.render, render, "a panel wrote the render settings on an idle frame");
        assert_eq!(editor.history.undo_len(), steps, "an idle frame recorded an undo step");
    }
}
