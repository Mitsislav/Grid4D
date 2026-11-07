//! The viewport's part of the performance budget measured without a window
//! (see `docs/performance.md`): the scenes of `cargo run --release --example
//! stress -- <folder>` evaluated, extracted and drawn on the GPU into a
//! viewport target of the reference window's viewport size, frame after
//! frame while the time slider moves, each frame waited for on the GPU — the
//! three parts View ▸ Frame Time shows (the rest of the interface is not
//! drawn). Plain and textured scenes side by side.
//!
//! ```text
//! cargo run --release --example stress -- target/stress
//! cargo test --release -p grid4d-ui --test budget -- --ignored --nocapture
//! ```
//!
//! `GRID4D_STRESS` names another folder.

#![allow(clippy::unwrap_used, reason = "test code")]

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use egui_wgpu::wgpu;
use grid4d_eval::EvaluatedScene;
use grid4d_io::Pixels;
use grid4d_render::{FrameData, OrbitCamera, TexelFormat, TextureLevel, ViewportRenderer, ViewportTarget};
use grid4d_scene::{ColorSpace, Document, TextureSource};

/// The viewport of the reference machine's maximised window, in pixels.
const SIZE: [u32; 2] = [1290, 900];
const WARM_UP: usize = 30;
const FRAMES: usize = 180;

/// Decodes `doc`'s pictures and hands them to `renderer` (as the interface's
/// texture cache does).
fn upload(doc: &Document, renderer: &mut ViewportRenderer, device: &wgpu::Device, queue: &wgpu::Queue) {
    for (id, texture) in doc.scene.textures().iter() {
        let TextureSource::Embedded { bytes, .. } = &texture.source else {
            continue;
        };
        let image = grid4d_io::decode_texture(bytes, texture.color_space).unwrap();
        let format = match image.levels.first().map(|l| &l.pixels) {
            Some(Pixels::RgbaF16(_)) => TexelFormat::Float16,
            _ if texture.color_space == ColorSpace::Linear => TexelFormat::Linear8,
            _ => TexelFormat::Srgb8,
        };
        let bytes: Vec<_> = image.levels.iter().map(grid4d_io::Level::bytes).collect();
        let levels: Vec<TextureLevel<'_>> = image
            .levels
            .iter()
            .zip(&bytes)
            .map(|(level, bytes)| TextureLevel {
                width: level.width,
                height: level.height,
                bytes,
            })
            .collect();
        renderer.upload_texture(device, queue, id, 1, format, &levels);
    }
}

/// The real GPU, as the app picks it: a discrete one first, Vulkan first (the
/// test harness's own choice prefers a software rasteriser).
fn gpu() -> egui_wgpu::WgpuSetup {
    let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    setup.native_adapter_selector = Some(std::sync::Arc::new(|adapters, _surface| {
        let mut adapters: Vec<_> = adapters.iter().collect();
        adapters.sort_by_key(|a| {
            let info = a.get_info();
            let kind = match info.device_type {
                wgpu::DeviceType::DiscreteGpu => 0,
                wgpu::DeviceType::IntegratedGpu => 1,
                wgpu::DeviceType::Other | wgpu::DeviceType::VirtualGpu => 2,
                wgpu::DeviceType::Cpu => 3,
            };
            let backend = match info.backend {
                wgpu::Backend::Vulkan => 0,
                wgpu::Backend::Metal => 0,
                wgpu::Backend::Dx12 => 1,
                wgpu::Backend::Gl | wgpu::Backend::BrowserWebGpu | wgpu::Backend::Noop => 2,
            };
            (kind, backend)
        });
        adapters
            .first()
            .map(|a| (*a).clone())
            .ok_or_else(|| "no GPU".to_owned())
    }));
    egui_wgpu::WgpuSetup::CreateNew(setup)
}

/// Mean and worst time of a frame of `name`: evaluation, extraction and the
/// GPU's drawing, waited for.
fn measure(name: &str) -> Option<(Duration, Duration)> {
    let folder = std::env::var_os("GRID4D_STRESS").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/stress"),
        PathBuf::from,
    );
    let Ok(mut doc) = grid4d_io::load(&folder.join(name)) else {
        println!(
            "{name}: not found in {} (run the stress example first)",
            folder.display()
        );
        return None;
    };
    let rs = egui_kittest::wgpu::create_render_state(gpu(), egui_wgpu::RendererOptions::default());
    let info = rs.adapter.get_info();
    println!("{name}: on {} ({:?}, {:?})", info.name, info.backend, info.device_type);
    let (device, queue) = (&rs.device, &rs.queue);
    let samples = ViewportTarget::supported_samples(&rs.adapter);
    let mut target = ViewportTarget::new(device, SIZE, samples);
    let mut renderer = ViewportRenderer::new(device, &target.config());
    upload(&doc, &mut renderer, device, queue);
    let camera = OrbitCamera::default();
    let mut evaluated = EvaluatedScene::new();
    let mut frame = FrameData::new();
    let aspect = f32::from(u16::try_from(SIZE[0]).unwrap()) / f32::from(u16::try_from(SIZE[1]).unwrap());
    let mut times = Vec::with_capacity(FRAMES);
    for k in 0..WARM_UP + FRAMES {
        let start = Instant::now();
        doc.time.set_frame(i32::try_from(k % 90).unwrap());
        evaluated.set_frame(f64::from(doc.time.frame()));
        evaluated.update(&doc.scene);
        frame.set_camera(&camera, aspect);
        frame.extract_with_materials(&evaluated, Some(doc.scene.materials()), |_| false);
        renderer.render(device, queue, &frame, &mut target);
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(30)),
            })
            .unwrap();
        if k >= WARM_UP {
            times.push(start.elapsed());
        }
    }
    let mean = times.iter().sum::<Duration>() / u32::try_from(times.len()).unwrap();
    let worst = times.iter().copied().max().unwrap_or_default();
    Some((mean, worst))
}

#[test]
#[ignore = "a measurement on the GPU; run by hand in release after the stress example"]
fn textured_scenes_against_the_budget() {
    for name in [
        "stress-10k.g4d",
        "stress-10k-textured.g4d",
        "stress-1m.g4d",
        "stress-1m-textured.g4d",
    ] {
        if let Some((mean, worst)) = measure(name) {
            let fps = 1.0 / mean.as_secs_f64();
            println!(
                "{name}: {:.2} ms a frame ({fps:.0} fps), worst {:.2} ms",
                mean.as_secs_f64() * 1000.0,
                worst.as_secs_f64() * 1000.0
            );
        }
    }
}
