//! OpenC4D — an open-source, modular 3D content creation suite.
//!
//! This binary is only the composition root: it configures the window and the
//! GPU surface, then hands control to [`app::OpenC4DApp`], which wires the
//! document (`oc4d-scene`), the renderer (`oc4d-render`) and the panels (`oc4d-ui`).

// Release builds on Windows: no console window behind the editor.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;

use app::{GpuSettings, OpenC4DApp};

/// Depth-buffer precision for the 3D viewport (the viewport draws into egui's pass).
const DEPTH_BITS: u8 = 24;
/// MSAA sample count for the whole window, including the 3D viewport.
const MSAA_SAMPLES: u16 = 4;

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
            .with_inner_size([1600.0, 940.0])
            .with_min_inner_size([1024.0, 640.0]),
        renderer: eframe::Renderer::Wgpu,
        depth_buffer: DEPTH_BITS,
        multisampling: MSAA_SAMPLES,
        ..Default::default()
    };

    eframe::run_native(
        "OpenC4D",
        native_options,
        Box::new(|cc| {
            Ok(Box::new(OpenC4DApp::new(
                cc,
                GpuSettings {
                    depth_bits: DEPTH_BITS,
                    msaa_samples: u32::from(MSAA_SAMPLES),
                },
                open,
            )))
        }),
    )
}
