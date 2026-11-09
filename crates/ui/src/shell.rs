//! The whole interface: the document, the editor state and every panel, laid
//! out each frame as the fixed bars around the docked workspace. The binary
//! wraps it for eframe (settings in storage, the window); tests drive it
//! headless with `egui_kittest` (`crates/ui/tests/interface.rs`).

use egui::{CentralPanel, Margin, Panel};
use grid4d_scene::Document;

use crate::panels::{
    about, alert, drop_overlay, menu_bar, mode_palette, optimize, plugin_command, preferences as preferences_window,
    recovery, script_manager, status_bar, toolbar, unsaved_changes,
};
use crate::theme::{self, metrics};
use crate::workspace::{Panels, Workspace};
use crate::{
    EditorState, PictureViewerPanel, RenderSettingsPanel, ViewportPanel, commands, file, preferences, shortcuts,
    window::WindowFit,
};

/// The document, the editor and the panels: everything a frame draws.
#[derive(Debug)]
pub struct Shell {
    pub doc: Document,
    pub editor: EditorState,
    /// The docked panels and their arrangement.
    pub panels: Panels,
    pub workspace: Workspace,
    render_settings: RenderSettingsPanel,
    picture_viewer: PictureViewerPanel,
    /// Time of the previous playback tick (egui clock, seconds).
    last_tick: Option<f64>,
    window_fit: WindowFit,
}

impl Shell {
    #[must_use]
    pub fn new(doc: Document, editor: EditorState, viewport: ViewportPanel, workspace: Workspace) -> Self {
        Self {
            doc,
            editor,
            panels: Panels::new(viewport),
            workspace,
            render_settings: RenderSettingsPanel::default(),
            picture_viewer: PictureViewerPanel::default(),
            last_tick: None,
            window_fit: WindowFit::default(),
        }
    }

    /// Tab over the bottom panel switches its tab, before egui moves focus
    /// with it (call with the frame's raw input).
    pub fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        shortcuts::take_tab(ctx, &mut self.editor, raw_input);
    }

    /// Advances playback before the interface runs, by real elapsed time.
    pub fn logic(&mut self, ctx: &egui::Context) {
        if self.doc.time.is_playing() {
            let now = ctx.input(|i| i.time);
            let dt = self.last_tick.map_or(0.0, |prev| (now - prev).clamp(0.0, 0.25));
            self.doc.time.advance(dt);
            self.last_tick = Some(now);
            ctx.request_repaint();
        } else {
            self.last_tick = None;
        }
    }

    /// One frame of the interface: shortcuts, the bars, the workspace, the
    /// windows, then the queued commands.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.window_fit.check(&ctx);
        preferences::set_units(&ctx, self.editor.preferences.units);
        theme::follow(&ctx, self.editor.preferences.theme);
        crate::i18n::set_language(self.editor.preferences.language);
        shortcuts::handle(&ctx, &mut self.doc, &mut self.editor);
        file::accept_dropped_files(&ctx, &mut self.editor);

        // Order matters: the top and bottom bars span the full width, then the
        // mode palette takes the left edge and the docked panes fill the rest.
        Panel::top("grid4d.menu_bar")
            .frame(theme::header_frame())
            .exact_size(metrics::MENU_BAR_HEIGHT)
            .show(ui, |ui| menu_bar::show(ui, &mut self.doc, &mut self.editor));

        Panel::top("grid4d.toolbar")
            .frame(theme::panel_frame().inner_margin(Margin::symmetric(6, 0)))
            .exact_size(metrics::TOOLBAR_HEIGHT)
            .show(ui, |ui| toolbar::show(ui, &mut self.editor));

        Panel::bottom("grid4d.status_bar")
            .frame(theme::header_frame())
            .exact_size(metrics::STATUS_BAR_HEIGHT)
            .show(ui, |ui| status_bar::show(ui, &self.doc, &self.editor));
        // The status bar shows last frame's hint; panels below set this frame's.
        self.editor.status_hint = None;

        Panel::left("grid4d.mode_palette")
            .frame(theme::panel_frame())
            .resizable(false)
            .exact_size(40.0)
            .show(ui, |ui| mode_palette::show(ui, &self.doc, &mut self.editor));

        // The viewport and the managers, docked as the user arranged them.
        CentralPanel::no_frame().show(ui, |ui| {
            self.workspace
                .show(ui, &mut self.panels, &mut self.doc, &mut self.editor);
        });

        self.render_settings.show(&ctx, &mut self.doc, &mut self.editor);
        self.picture_viewer.show(&ctx, &mut self.editor);
        crate::panels::render_queue::show(&ctx, &self.doc, &mut self.editor);
        about::show(&ctx, &mut self.editor);
        preferences_window::show(&ctx, &mut self.editor);
        recovery::show(&ctx, &mut self.editor);
        optimize::show(&ctx, &mut self.editor);
        plugin_command::show(&ctx, &mut self.editor);
        script_manager::show(&ctx, &mut self.editor);
        unsaved_changes::show(&ctx, &self.doc, &mut self.editor);
        alert::show(&ctx, &mut self.editor);
        drop_overlay::show(&ctx);

        if commands::apply_pending(&mut self.doc, &mut self.editor, &mut self.panels.viewport) {
            ctx.request_repaint();
        }
        // Title, close button and confirmed quit follow the document's file state.
        file::sync_window(&ctx, &self.doc, &mut self.editor);
        file::autosave(&ctx, &self.doc, &mut self.editor);
    }
}
