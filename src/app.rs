//! The application: the interface (`grid4d_ui::shell::Shell`) set up from
//! the settings eframe keeps between sessions, and those settings saved.

use grid4d_scene::Document;
use grid4d_ui::{
    EditorCommand, EditorState, RecentFiles, ViewportPanel,
    autosave::Autosave,
    preferences::{self, Preferences},
    recent,
    shell::Shell,
    theme,
    workspace::{self, Workspace},
};

/// The interface ([`Shell`]) with what only the application has: settings
/// kept in eframe storage between sessions.
pub struct Grid4DApp {
    shell: Shell,
}

impl Grid4DApp {
    /// `open`: a document to open instead of the demo scene (from the command
    /// line). `data`: the data folder, if it can be used; `data_problem`: why
    /// not, told on the first frame.
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        open: Option<std::path::PathBuf>,
        data: Option<std::path::PathBuf>,
        data_problem: Option<grid4d_ui::startup::DataFolderProblem>,
    ) -> Self {
        let render_state = cc.wgpu_render_state.as_ref();
        let mut viewport = ViewportPanel::new(render_state);

        let mut editor = EditorState {
            renderer_info: render_state.map_or_else(
                || "No GPU renderer".to_owned(),
                |rs| {
                    let info = rs.adapter.get_info();
                    format!("{} · {:?}", info.name, info.backend)
                },
            ),
            build: build_info(),
            ..EditorState::default()
        };
        log::info!("Renderer: {}", editor.renderer_info);
        grid4d_ui::crash::set_adapter(&editor.renderer_info);
        if let Some(stored) = cc.storage.and_then(|s| s.get_string(recent::STORAGE_KEY)) {
            editor.file.recent = RecentFiles::from_storage(&stored);
        }
        if let Some(stored) = cc
            .storage
            .and_then(|s| eframe::get_value::<Preferences>(s, preferences::STORAGE_KEY))
        {
            editor.preferences = stored.checked();
        }
        theme::apply(&cc.egui_ctx, editor.preferences.theme);
        grid4d_ui::i18n::set_language(editor.preferences.language);
        viewport.apply_defaults(&editor.preferences.views);
        // Recovery copies live beside the stored settings; one a crashed run
        // left is offered on the first frame, with a crash report written
        // since the last start.
        if let Some(dir) = data {
            editor.autosave = Autosave::new(dir.join("recovery"));
            editor.crash = grid4d_ui::crash::CrashReports::new(&dir.join("crashes"));
            // The script library beside them (made with the examples).
            editor.scripts = grid4d_ui::scripts::Scripts::new(dir.join("scripts"));
        }
        if let Some(problem) = data_problem {
            let alert = problem.alert();
            log::warn!("{}: {}", alert.title, alert.message);
            editor.file.alert = Some(alert);
        }

        let mut doc = Document::with_demo_scene();
        doc.render = grid4d_scene::RenderSettings::for_new_documents();
        // Start with the first object active so the gizmo and Attribute Manager have content.
        if let Some(&first) = doc.scene.roots().first() {
            editor.selection.set_single(first);
        }
        // Opened on the first frame, through the same command as File ▸ Open Recent
        // (errors show the usual alert).
        if let Some(path) = open {
            editor.commands.push(EditorCommand::OpenFile(path));
        }

        // The arrangement the user left, repaired if it lacks a pane.
        let workspace = cc
            .storage
            .and_then(|s| eframe::get_value::<Workspace>(s, workspace::STORAGE_KEY))
            .map(Workspace::repaired)
            .unwrap_or_default();
        Self {
            shell: Shell::new(doc, editor, viewport, workspace),
        }
    }
}

impl eframe::App for Grid4DApp {
    /// Called by eframe on exit and periodically. Window geometry and panel sizes
    /// are persisted by eframe itself (the `persistence` feature).
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let editor = &self.shell.editor;
        storage.set_string(recent::STORAGE_KEY, editor.file.recent.to_storage());
        eframe::set_value(storage, workspace::STORAGE_KEY, &self.shell.workspace);
        eframe::set_value(storage, preferences::STORAGE_KEY, &editor.preferences);
    }

    /// A normal exit: the recovery copy is no longer needed.
    fn on_exit(&mut self) {
        self.shell.editor.autosave.finish();
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.shell.raw_input_hook(ctx, raw_input);
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.shell.logic(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.shell.ui(ui);
    }
}

/// The build's identity (see build.rs) and the licences it contains.
pub fn build_info() -> grid4d_ui::BuildInfo {
    grid4d_ui::BuildInfo {
        version: env!("CARGO_PKG_VERSION"),
        commit: env!("GRID4D_COMMIT"),
        date: env!("GRID4D_COMMIT_DATE"),
        licences: include_str!("../assets/THIRD_PARTY_LICENSES.txt"),
    }
}
