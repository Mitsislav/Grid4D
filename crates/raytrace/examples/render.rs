//! Renders a Grid4D document — or the demo scene — to a PNG file.
//!
//! ```text
//! cargo run --release -p grid4d-raytrace --example render -- [scene.g4d | demo] [out.png] [WIDTHxHEIGHT] [samples]
//! ```
//!
//! The picture is seen through the document's first camera object, else from a
//! view that frames the whole scene. Defaults: the demo scene, `render.png`,
//! 640 × 360, 16 samples.

use std::{error::Error, fs::File, io::BufWriter, path::Path, time::Instant};

use grid4d_eval::EvaluatedScene;
use grid4d_raytrace::{CameraProjection, RenderCamera, RenderOptions, RenderScene, render};
use grid4d_scene::{Document, ObjectKind};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let source = args.first().map_or("demo", String::as_str);
    let output = args.get(1).map_or("render.png", String::as_str);
    let mut options = RenderOptions::default();
    if let Some((w, h)) = args.get(2).and_then(|size| size.split_once('x')) {
        (options.width, options.height) = (w.parse()?, h.parse()?);
    }
    if let Some(samples) = args.get(3) {
        options.samples = samples.parse()?;
    }

    let doc = if source == "demo" {
        Document::with_demo_scene()
    } else {
        grid4d_io::load(Path::new(source))?
    };
    let mut evaluated = EvaluatedScene::for_render();
    evaluated.update(&doc.scene);
    let started = Instant::now();
    let scene = RenderScene::new(&evaluated);
    let built = started.elapsed();

    let camera = doc
        .scene
        .iter()
        .find_map(|(id, node)| match node.data.kind {
            ObjectKind::Camera { fov_y_deg } => Some(RenderCamera {
                world: doc.scene.world_matrix(id),
                projection: CameraProjection::Perspective {
                    fov_y: fov_y_deg.to_radians(),
                },
            }),
            _ => None,
        })
        .or_else(|| {
            scene
                .bounds()
                .map(|(min, max)| RenderCamera::framing(min, max, 40f32.to_radians()))
        })
        .ok_or("the scene is empty")?;

    let image = render(&scene, &camera, &options);
    println!(
        "{} triangles, built in {:.2?}; {}×{} at {} samples rendered in {:.2?}",
        scene.triangle_count(),
        built,
        image.width,
        image.height,
        options.samples,
        started.elapsed() - built,
    );

    let mut encoder = png::Encoder::new(BufWriter::new(File::create(output)?), image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&image.to_rgba8())?;
    println!("saved {output}");
    Ok(())
}
