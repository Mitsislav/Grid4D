//! OpenC4D — an open-source, modular 3D content creation suite.
//!
//! This binary is only the composition root: it configures the window and the
//! GPU surface, then hands control to [`app::OpenC4DApp`], which wires the
//! document (`oc4d-scene`), the renderer (`oc4d-render`) and the panels (`oc4d-ui`).

// Release builds on Windows: no console window behind the editor.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;

use app::OpenC4DApp;

fn main() -> eframe::Result {
    env_logger::Builder::from_env(
        env_logger::Env::default()
            .default_filter_or("warn,open_c4d=info,oc4d_scene=info,oc4d_render=info,oc4d_ui=info"),
    )
    .init();

    // `open-c4d scene.oc4d` opens that document (how "Open with" and file
    // associations launch the app).
    let open = std::env::args_os().nth(1).map(std::path::PathBuf::from);

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("OpenC4D")
            .with_app_id("open-c4d")
            // Start maximized; the size below is what un-maximizing returns to, and it
            // fits a 1366 × 768 screen. A saved window state takes precedence.
            .with_maximized(true)
            .with_inner_size([1280.0, 720.0])
            .with_min_inner_size([1024.0, 640.0]),
        // The 3D viewport renders into its own target with its own MSAA and depth,
        // so the window surface (egui's 2D only) needs neither.
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "OpenC4D",
        native_options,
        Box::new(|cc| Ok(Box::new(OpenC4DApp::new(cc, open)))),
    )
}
