//! Grid4D — an open-source, modular 3D content creation suite.
//!
//! This binary is only the composition root: it configures the window and the
//! GPU surface, then hands control to [`app::Grid4DApp`], which wires the
//! document (`grid4d-scene`), the renderer (`grid4d-render`) and the panels (`grid4d-ui`).

// Release builds on Windows: no console window behind the editor.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod plugins;

use app::Grid4DApp;

fn main() -> eframe::Result {
    // The log, whose last lines a crash report keeps.
    let logger = env_logger::Builder::from_env(
        env_logger::Env::default()
            .default_filter_or("warn,grid4d=info,grid4d_scene=info,grid4d_render=info,grid4d_ui=info"),
    )
    .build();
    let level = logger.filter();
    grid4d_ui::crash::install_log(Box::new(logger), level);

    // Settings, recovery copies, scripts and crash reports live here; a
    // panic from now on leaves a report (nothing is sent anywhere).
    let data = eframe::storage_dir("grid4d");
    let data_problem = grid4d_ui::startup::check_data_folder(data.as_deref()).err();
    let data = data.filter(|_| data_problem.is_none());
    if let Some(dir) = &data {
        grid4d_ui::crash::install(dir.join("crashes"), app::build_info());
    }

    // `grid4d scene.g4d` opens that document (how "Open with" and file
    // associations launch the app).
    let open = std::env::args_os().nth(1).map(std::path::PathBuf::from);

    // The plugins built into this Grid4D, before anything evaluates a scene.
    grid4d_plugin::install(plugins::registry());

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Grid4D")
        .with_app_id("grid4d")
        // Start maximized; the size below is what un-maximizing returns to, and it
        // fits a 1366 × 768 screen. A saved window state takes precedence.
        .with_maximized(true)
        .with_inner_size([1280.0, 720.0])
        .with_min_inner_size([1024.0, 640.0]);
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(icon);
    }

    let native_options = eframe::NativeOptions {
        viewport,
        // The 3D viewport renders into its own target with its own MSAA and depth,
        // so the window surface (egui's 2D only) needs neither.
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    let started = eframe::run_native(
        "Grid4D",
        native_options,
        Box::new(|cc| Ok(Box::new(Grid4DApp::new(cc, open, data, data_problem)))),
    );
    // Without usable graphics the window never opens: say why rather than
    // vanish.
    if let Err(err) = &started {
        grid4d_ui::startup::cannot_start(&err.to_string());
    }
    started
}

/// The window, taskbar and dock icon. (On Windows `build.rs` also embeds
/// `grid4d.ico` in the executable, for Explorer and shortcuts.)
fn window_icon() -> Option<egui::IconData> {
    eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon/grid4d.png"))
        .inspect_err(|err| log::warn!("Window icon not loaded: {err}"))
        .ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_window_icon_decodes() {
        let icon = super::window_icon().expect("embedded icon is a valid PNG");
        assert_eq!((icon.width, icon.height), (256, 256));
    }
}
