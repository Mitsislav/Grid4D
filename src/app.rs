//! The application shell: owns the document and every panel, and lays out the
//! Cinema 4D "Standard" workspace each frame.

use egui::{CentralPanel, Margin, Panel};
use grid4d_scene::Document;
use grid4d_ui::{
    AttributesPanel, BottomPanel, CoordinatesPanel, EditorCommand, EditorState, FCurvePanel, ManagerTab,
    MaterialsPanel, OutlinerPanel, PictureViewerPanel, RecentFiles, RenderSettingsPanel, TimelinePanel, ViewportPanel,
    commands, file,
    panels::{about, alert, drop_overlay, menu_bar, mode_palette, optimize, status_bar, toolbar, unsaved_changes},
    recent, shortcuts,
    theme::{self, metrics},
    window::WindowFit,
};

const MIN_MANAGERS_WIDTH: f32 = 280.0;
const MIN_FCURVE_HEIGHT: f32 = 160.0;
const MIN_MATERIALS_HEIGHT: f32 = 110.0;

pub struct Grid4DApp {
    doc: Document,
    editor: EditorState,
    viewport: ViewportPanel,
    outliner: OutlinerPanel,
    attributes: AttributesPanel,
    coordinates: CoordinatesPanel,
    timeline: TimelinePanel,
    fcurve: FCurvePanel,
    materials: MaterialsPanel,
    render_settings: RenderSettingsPanel,
    picture_viewer: PictureViewerPanel,
    /// Time of the previous playback tick (egui clock, seconds).
    last_tick: Option<f64>,
    window_fit: WindowFit,
}

impl Grid4DApp {
    /// `open`: a document to open instead of the demo scene (from the command line).
    pub fn new(cc: &eframe::CreationContext<'_>, open: Option<std::path::PathBuf>) -> Self {
        theme::apply(&cc.egui_ctx);

        let render_state = cc.wgpu_render_state.as_ref();
        let viewport = ViewportPanel::new(render_state);

        let mut editor = EditorState {
            renderer_info: render_state.map_or_else(
                || "No GPU renderer".to_owned(),
                |rs| {
                    let info = rs.adapter.get_info();
                    format!("{} · {:?}", info.name, info.backend)
                },
            ),
            ..EditorState::default()
        };
        log::info!("Renderer: {}", editor.renderer_info);
        if let Some(stored) = cc.storage.and_then(|s| s.get_string(recent::STORAGE_KEY)) {
            editor.file.recent = RecentFiles::from_storage(&stored);
        }

        let doc = Document::with_demo_scene();
        // Start with the first object active so the gizmo and Attribute Manager have content.
        if let Some(&first) = doc.scene.roots().first() {
            editor.selection.set_single(first);
        }
        // Opened on the first frame, through the same command as File ▸ Open Recent
        // (errors show the usual alert).
        if let Some(path) = open {
            editor.commands.push(EditorCommand::OpenFile(path));
        }

        Self {
            doc,
            editor,
            viewport,
            outliner: OutlinerPanel::default(),
            attributes: AttributesPanel::default(),
            coordinates: CoordinatesPanel::default(),
            timeline: TimelinePanel::default(),
            fcurve: FCurvePanel::default(),
            materials: MaterialsPanel,
            render_settings: RenderSettingsPanel::default(),
            picture_viewer: PictureViewerPanel::default(),
            last_tick: None,
            window_fit: WindowFit::default(),
        }
    }
}

impl eframe::App for Grid4DApp {
    /// Called by eframe on exit and periodically. Window geometry and panel sizes
    /// are persisted by eframe itself (the `persistence` feature).
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(recent::STORAGE_KEY, self.editor.file.recent.to_storage());
    }

    /// Tab over the bottom panel switches its tab, before egui moves focus with it.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        shortcuts::take_tab(ctx, &mut self.editor, raw_input);
    }

    /// Advances playback before the UI runs, using real elapsed time.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.window_fit.check(&ctx);
        shortcuts::handle(&ctx, &mut self.doc, &mut self.editor);
        file::accept_dropped_files(&ctx, &mut self.editor);

        // Order matters: top/bottom bars span the full width, then the right-hand
        // managers take full height, then the left palette and the timeline
        // (under the viewport only), and finally the viewport fills the rest.
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

        // Limits are relative to the window so the viewport and timeline always keep room.
        let max_managers_width = (ui.available_width() * 0.45).max(MIN_MANAGERS_WIDTH);
        Panel::right("grid4d.managers")
            .frame(theme::panel_frame())
            .resizable(true)
            .default_size(370.0)
            .size_range(MIN_MANAGERS_WIDTH..=max_managers_width)
            .show(ui, |ui| {
                let max_outliner_height = (ui.available_height() - 160.0).max(140.0);
                Panel::top("grid4d.object_manager")
                    .frame(theme::panel_frame())
                    .resizable(true)
                    .default_size(330.0)
                    .size_range(140.0..=max_outliner_height)
                    .show(ui, |ui| self.outliner.show(ui, &mut self.doc, &mut self.editor));
                CentralPanel::no_frame().show(ui, |ui| match self.editor.managers {
                    ManagerTab::Attributes => self.attributes.show(ui, &mut self.doc, &mut self.editor),
                    ManagerTab::Coordinates => self.coordinates.show(ui, &mut self.doc, &mut self.editor),
                });
            });

        Panel::left("grid4d.mode_palette")
            .frame(theme::panel_frame())
            .resizable(false)
            .exact_size(40.0)
            .show(ui, |ui| mode_palette::show(ui, &self.doc, &mut self.editor));

        // The bottom panel's tabs: the Timeline, or the taller F-Curve editor.
        match self.editor.bottom {
            BottomPanel::Timeline => {
                Panel::bottom("grid4d.timeline")
                    .frame(theme::panel_frame())
                    .resizable(false)
                    .exact_size(grid4d_ui::panels::timeline::HEIGHT)
                    .show(ui, |ui| self.timeline.show(ui, &mut self.doc, &mut self.editor));
            }
            BottomPanel::FCurve => {
                let max_height = (ui.available_height() * 0.65).max(MIN_FCURVE_HEIGHT);
                Panel::bottom("grid4d.fcurve")
                    .frame(theme::panel_frame())
                    .resizable(true)
                    .default_size(grid4d_ui::panels::fcurve::DEFAULT_HEIGHT)
                    .size_range(MIN_FCURVE_HEIGHT..=max_height)
                    .show(ui, |ui| self.fcurve.show(ui, &mut self.doc, &mut self.editor));
            }
            BottomPanel::Materials => {
                let max_height = (ui.available_height() * 0.5).max(MIN_MATERIALS_HEIGHT);
                Panel::bottom("grid4d.materials")
                    .frame(theme::panel_frame())
                    .resizable(true)
                    .default_size(grid4d_ui::panels::materials::DEFAULT_HEIGHT)
                    .size_range(MIN_MATERIALS_HEIGHT..=max_height)
                    .show(ui, |ui| self.materials.show(ui, &mut self.doc, &mut self.editor));
            }
        }

        CentralPanel::no_frame().show(ui, |ui| self.viewport.show(ui, &mut self.doc, &mut self.editor));

        self.render_settings.show(&ctx, &mut self.doc, &mut self.editor);
        self.picture_viewer.show(&ctx, &mut self.editor);
        about::show(&ctx, &mut self.editor);
        optimize::show(&ctx, &mut self.editor);
        unsaved_changes::show(&ctx, &self.doc, &mut self.editor);
        alert::show(&ctx, &mut self.editor);
        drop_overlay::show(&ctx);

        if commands::apply_pending(&mut self.doc, &mut self.editor, &mut self.viewport) {
            ctx.request_repaint();
        }
        // Title, close button and confirmed quit follow the document's file state.
        file::sync_window(&ctx, &self.doc, &mut self.editor);
    }
}
